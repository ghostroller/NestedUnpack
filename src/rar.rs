use std::cell::RefCell;
use std::ffi::c_char;
use std::fs::{self, File};
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::ptr::{self, NonNull};

use unrar_sys as rar;
use zeroize::Zeroizing;

use crate::archive::{Control, ExtractError, safe_path};
use crate::i18n::{EngineKey, Message};

#[cfg(test)]
mod tests;

// UnRAR's DLL header is packed to one byte. unrar_sys 0.5.8 uses
// naturally aligned structs, which disagree with this ABI on 64-bit targets.
#[repr(C, packed)]
struct OpenData {
    name: *const c_char,
    name_wide: *const rar::WCHAR,
    mode: u32,
    result: u32,
    comment: *mut c_char,
    comment_size: u32,
    comment_length: u32,
    comment_state: u32,
    flags: u32,
    callback: Option<extern "system" fn(u32, isize, isize, isize) -> i32>,
    user_data: isize,
    operation_flags: u32,
    comment_wide: *mut rar::WCHAR,
    reserved: [u32; 25],
}

#[repr(C, packed)]
struct Header {
    archive: [c_char; 1024],
    archive_wide: [rar::WCHAR; 1024],
    filename: [c_char; 1024],
    filename_wide: [rar::WCHAR; 1024],
    flags: u32,
    packed_size: u32,
    packed_size_high: u32,
    size: u32,
    size_high: u32,
    host_os: u32,
    crc: u32,
    time: u32,
    version: u32,
    method: u32,
    attributes: u32,
    comment: *mut c_char,
    comment_size: u32,
    comment_length: u32,
    comment_state: u32,
    dictionary_size: u32,
    hash_type: u32,
    hash: [c_char; 32],
    redirect_type: u32,
    redirect_name: *mut rar::WCHAR,
    redirect_name_size: u32,
    directory_target: u32,
    times: [u32; 6],
    archive_extended: *mut rar::WCHAR,
    archive_extended_size: u32,
    filename_extended: *mut rar::WCHAR,
    filename_extended_size: u32,
    reserved: [u32; 982],
}

struct Handle(NonNull<rar::Handle>);

impl Drop for Handle {
    fn drop(&mut self) {
        // The native handle is owned by this guard.
        unsafe { rar::RARCloseArchive(self.0.as_ptr()) };
    }
}

struct State<'a> {
    control: &'a mut dyn Control,
    password: Option<&'a str>,
    wide_password: Zeroizing<Vec<rar::WCHAR>>,
    password_used: bool,
    file: Option<File>,
    remaining: u64,
    done: u64,
    total: Option<u64>,
    error: Option<ExtractError>,
}

pub(crate) fn extract(
    source: &Path,
    destination: &Path,
    control: &mut dyn Control,
) -> Result<(), ExtractError> {
    let source = archive_path(source)?;
    let mut password: Option<Zeroizing<String>> = None;
    loop {
        let attempt = tempfile::tempdir_in(destination)?;
        let current = password.as_deref().map(String::as_str);
        let mut state = RefCell::new(State {
            control,
            password: current,
            wide_password: Zeroizing::new(current.map(wide).unwrap_or_default()),
            password_used: false,
            file: None,
            remaining: 0,
            done: 0,
            total: None,
            error: None,
        });
        let result = pass(&source, attempt.path(), &state, false).and_then(|total| {
            let progress = state.get_mut();
            progress.total = Some(total);
            progress.control.progress(0, progress.total);
            pass(&source, attempt.path(), &state, true)
        });
        let state = state.get_mut();
        match result {
            Ok(_) => {
                for entry in fs::read_dir(attempt.path())? {
                    let entry = entry?;
                    fs::rename(entry.path(), destination.join(entry.file_name()))?;
                }
                if state.password_used
                    && let Some(current) = current
                {
                    state.control.accepted(current);
                }
                return Ok(());
            }
            Err(ExtractError::Password) => {
                password = Some(Zeroizing::new(state.control.password(password.is_some())?));
            }
            Err(error) => return Err(error),
        }
    }
}

fn pass(
    source: &ArchivePath,
    destination: &Path,
    state: &RefCell<State<'_>>,
    extract: bool,
) -> Result<u64, ExtractError> {
    let mode = if extract {
        rar::RAR_OM_EXTRACT
    } else {
        rar::RAR_OM_LIST
    };
    let mut opening: OpenData = unsafe { std::mem::zeroed() };
    // On Linux, UnRAR can round-trip the byte path through its locale-aware
    // filename conversion, including bytes outside the current C locale.
    #[cfg(target_os = "linux")]
    {
        opening.name = source.as_ptr();
    }
    #[cfg(not(target_os = "linux"))]
    {
        opening.name_wide = source.as_ptr();
    }
    opening.mode = mode;
    opening.callback = Some(callback);
    opening.user_data = std::ptr::from_ref(state).expose_provenance() as isize;
    // The packed buffers match dll.hpp. The caller and context outlive each
    // synchronous native call, including callback invocations.
    let raw = unsafe { rar::RAROpenArchiveEx((&raw mut opening).cast()) };
    let handle = NonNull::new(raw.cast_mut()).map(Handle);
    status(opening.result as i32, &mut state.borrow_mut(), false)?;
    let handle = handle.ok_or_else(|| invalid(EngineKey::OpenRar))?;
    let headers_encrypted = opening.flags & rar::ROADF_ENCHEADERS != 0;
    let mut total = 0_u64;
    let mut name = vec![0; 32_768];
    loop {
        if state.borrow().control.canceled() {
            return Err(ExtractError::Canceled);
        }
        let mut header: Header = unsafe { std::mem::zeroed() };
        name[0] = 0;
        header.filename_extended = name.as_mut_ptr();
        header.filename_extended_size = name.len() as u32;
        let result = unsafe { rar::RARReadHeaderEx(handle.0.as_ptr(), (&raw mut header).cast()) };
        if result == rar::ERAR_END_ARCHIVE && state.borrow().error.is_none() {
            return Ok(total);
        }
        status(result, &mut state.borrow_mut(), headers_encrypted)?;
        let directory = header.flags & rar::RHDF_DIRECTORY != 0;
        let encrypted = header.flags & rar::RHDF_ENCRYPTED != 0;
        if header.flags & rar::RHDF_SPLITBEFORE != 0 {
            return Err(invalid(EngineKey::StartFirstRarVolume));
        }
        let entry_name = decoded_name(&name)?;
        let attributes = header.attributes;
        let unix_type = attributes & 0xf000;
        if header.redirect_type != 0
            || (header.host_os == 3 && !matches!(unix_type, 0 | 0x4000 | 0x8000))
            || (header.host_os == 2 && attributes & 0x400 != 0)
        {
            return Err(invalid(
                Message::from(EngineKey::UnsupportedRarEntry).arg(format!("{entry_name:?}")),
            ));
        }
        let target = safe_path(destination, &entry_name)?;
        let size = (u64::from(header.size_high) << 32) | u64::from(header.size);
        let operation = if extract && !directory {
            fs::create_dir_all(
                target
                    .parent()
                    .ok_or_else(|| invalid(EngineKey::InvalidRarPath))?,
            )?;
            let mut state = state.borrow_mut();
            state.file = Some(File::create_new(&target)?);
            state.remaining = size;
            rar::RAR_TEST
        } else {
            if extract {
                fs::create_dir_all(&target)?;
            }
            rar::RAR_SKIP
        };
        // Test mode verifies checksums and sends unpacked bytes to the callback.
        // Only Rust creates files, after validating each destination.
        let result =
            unsafe { rar::RARProcessFileW(handle.0.as_ptr(), operation, ptr::null(), ptr::null()) };
        let mut state = state.borrow_mut();
        state.file = None;
        status(result, &mut state, encrypted || headers_encrypted)?;
        if operation == rar::RAR_TEST && state.remaining != 0 {
            return Err(invalid(
                Message::from(EngineKey::RarFileSizeMismatch).arg(format!("{entry_name:?}")),
            ));
        }
        if !directory {
            total = total
                .checked_add(size)
                .ok_or_else(|| invalid(EngineKey::RarContentTooLarge))?;
        }
    }
}

fn status(result: i32, state: &mut State<'_>, encrypted: bool) -> Result<(), ExtractError> {
    if let Some(error) = state.error.take() {
        return Err(error);
    }
    match result {
        rar::ERAR_SUCCESS => Ok(()),
        rar::ERAR_MISSING_PASSWORD | rar::ERAR_BAD_PASSWORD => Err(ExtractError::Password),
        rar::ERAR_BAD_DATA if encrypted => Err(ExtractError::Password),
        _ => Err(invalid(
            Message::from(EngineKey::UnrarError).arg(result.to_string()),
        )),
    }
}

extern "system" fn callback(message: u32, context: isize, arg: isize, value: isize) -> i32 {
    // UnRAR invokes this synchronously with the live state pointer passed at open.
    let state = unsafe { &*ptr::with_exposed_provenance::<RefCell<State<'_>>>(context as usize) };
    let Ok(mut state) = state.try_borrow_mut() else {
        return -1;
    };
    match catch_unwind(AssertUnwindSafe(|| state.event(message, arg, value))) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            state.error = Some(error);
            -1
        }
        Err(_) => {
            state.error = Some(invalid(EngineKey::RarCallbackError));
            -1
        }
    }
}

impl State<'_> {
    fn event(&mut self, message: u32, arg: isize, value: isize) -> Result<i32, ExtractError> {
        if self.control.canceled() {
            return Err(ExtractError::Canceled);
        }
        match message {
            rar::UCM_PROCESSDATA => {
                if let Some(file) = &mut self.file
                    && value > 0
                {
                    self.remaining = self
                        .remaining
                        .checked_sub(value as u64)
                        .ok_or_else(|| invalid(EngineKey::RarExceedsDeclaredSize))?;
                    // UnRAR owns this data for the duration of the callback.
                    let data =
                        unsafe { std::slice::from_raw_parts(arg as *const u8, value as usize) };
                    file.write_all(data)?;
                    self.done += value as u64;
                    self.control.progress(self.done, self.total);
                }
                Ok(1)
            }
            rar::UCM_NEEDPASSWORD | rar::UCM_NEEDPASSWORDW => {
                let password = self.password.ok_or(ExtractError::Password)?;
                if message == rar::UCM_NEEDPASSWORDW {
                    copy_password(arg, value, &self.wide_password)?;
                } else {
                    copy_password(arg, value, password.as_bytes())?;
                }
                self.password_used = true;
                Ok(1)
            }
            rar::UCM_CHANGEVOLUME | rar::UCM_CHANGEVOLUMEW if value == 0 => {
                Err(invalid(EngineKey::MissingRarVolume))
            }
            rar::UCM_CHANGEVOLUME | rar::UCM_CHANGEVOLUMEW => Ok(1),
            // Returning zero retains UnRAR's default dictionary memory limit.
            _ => Ok(0),
        }
    }
}

fn copy_password<T: Copy + Default + PartialEq>(
    target: isize,
    capacity: isize,
    password: &[T],
) -> Result<(), ExtractError> {
    if capacity <= 0 || password.len() >= capacity as usize || password.contains(&T::default()) {
        return Err(invalid(EngineKey::InvalidRarPassword));
    }
    let target = target as *mut T;
    // The native caller provides capacity elements of the matching type.
    unsafe {
        ptr::copy_nonoverlapping(password.as_ptr(), target, password.len());
        target.add(password.len()).write(T::default());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
type ArchivePath = std::ffi::CString;
#[cfg(not(target_os = "linux"))]
type ArchivePath = Vec<rar::WCHAR>;

#[cfg(target_os = "linux")]
fn archive_path(path: &Path) -> Result<ArchivePath, ExtractError> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::path::absolute(path)?;
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| invalid(EngineKey::NulRarPath))
}

#[cfg(not(target_os = "linux"))]
fn archive_path(path: &Path) -> Result<ArchivePath, ExtractError> {
    let path = std::path::absolute(path)?;
    #[cfg(windows)]
    let units = {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().collect::<Vec<_>>()
    };
    #[cfg(not(windows))]
    let units = wide(
        path.to_str()
            .ok_or_else(|| invalid(EngineKey::NonUnicodeRarPath))?,
    );
    if units.contains(&0) {
        return Err(invalid(EngineKey::NulRarPath));
    }
    Ok(units.into_iter().chain([0]).collect())
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<rar::WCHAR> {
    text.encode_utf16().collect()
}

#[cfg(not(windows))]
fn wide(text: &str) -> Vec<rar::WCHAR> {
    text.chars()
        .map(|character| character as rar::WCHAR)
        .collect()
}

fn decoded_name(units: &[rar::WCHAR]) -> Result<String, ExtractError> {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .filter(|end| *end > 0 && *end < units.len() - 1)
        .ok_or_else(|| invalid(EngineKey::InvalidRarFilenameLength))?;
    #[cfg(windows)]
    let decoded = String::from_utf16(&units[..end]).ok();
    #[cfg(not(windows))]
    let decoded = units[..end]
        .iter()
        .map(|unit| char::from_u32(*unit as u32))
        .collect::<Option<String>>();
    decoded.ok_or_else(|| invalid(EngineKey::NonUnicodeRarFilename))
}

fn invalid(message: impl Into<Message>) -> ExtractError {
    ExtractError::Invalid(message.into())
}
