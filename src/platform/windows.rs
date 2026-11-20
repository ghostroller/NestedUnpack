use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use ::windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
use ::windows::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetStdHandle,
};
use ::windows::Win32::UI::Shell::ShellExecuteW;
use ::windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use ::windows::core::{PCWSTR, w};

use crate::i18n::{AppKey, Message};

mod explorer;
mod passwords;

pub use explorer::{
    menu_registered, refresh_menu_language, register_menu, remove_menu, run_explorer_server,
};
pub use passwords::{forget_password, read_passwords, remember_password};
pub(super) use passwords::{merge_passwords, validate_backup_destination};

const SELECTION_PREFIX: &str = "NestedUnpack-Selection-";
const SELECTION_SUFFIX: &str = ".utf8";

pub fn attach_console() {
    let handles = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];
    // Redirected streams belong to the caller and must survive AttachConsole.
    let inherited = handles.map(|kind| unsafe { GetStdHandle(kind) }.ok());
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_ok() {
        for (kind, previous) in handles.into_iter().zip(inherited) {
            if let Some(handle) = previous.filter(|handle| !handle.is_invalid()) {
                let _ = unsafe { SetStdHandle(kind, handle) };
            }
        }
    }
}

pub fn open_directory(path: &Path) -> io::Result<()> {
    let absolute = std::path::absolute(path)?;
    if !absolute.metadata()?.is_dir() {
        return Err(Message::from(AppKey::OutputNotDirectory).io(io::ErrorKind::NotADirectory));
    }
    let path = wide(absolute.as_os_str())?;
    let _apartment = explorer::ComApartment::enter().ok();
    // ShellExecute returns a reserved value greater than 32 on success.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(Message::from(AppKey::OpenOutputShellFailed)
            .arg(result.0 as usize)
            .io(io::ErrorKind::Other))
    }
}

pub fn read_selection(path: &Path) -> io::Result<Vec<PathBuf>> {
    read_selection_in(path, &std::env::temp_dir())
}

fn read_selection_in(path: &Path, temp: &Path) -> io::Result<Vec<PathBuf>> {
    let path = std::path::absolute(path)?;
    // GetTempPath includes a trailing separator while Path::parent omits it.
    let temp: PathBuf = std::path::absolute(temp)?.components().collect();
    let correct_parent = path
        .parent()
        .is_some_and(|parent| equal_ignore_case(parent.as_os_str(), temp.as_os_str()));
    let valid_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.starts_with(SELECTION_PREFIX) && name.ends_with(SELECTION_SUFFIX));
    if !correct_parent || !valid_name || !path.symlink_metadata()?.file_type().is_file() {
        return Err(Message::from(AppKey::InvalidSelectionFile).io(io::ErrorKind::InvalidInput));
    }
    let list = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    list?
        .lines()
        .map(|line| {
            let path = PathBuf::from(line);
            if path.is_absolute() && !line.contains('\0') {
                Ok(path)
            } else {
                Err(Message::from(AppKey::InvalidSelectionEntry).io(io::ErrorKind::InvalidData))
            }
        })
        .collect()
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut result: Vec<u16> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err(Message::from(AppKey::NullInText).io(io::ErrorKind::InvalidInput));
    }
    result.push(0);
    Ok(result)
}

pub(super) fn equal_ignore_case(lhs: &OsStr, rhs: &OsStr) -> bool {
    let lhs: Vec<u16> = lhs.encode_wide().collect();
    let rhs: Vec<u16> = rhs.encode_wide().collect();
    unsafe { CompareStringOrdinal(&lhs, &rhs, true) == CSTR_EQUAL }
}

fn selection_path(path: &[u16]) -> PathBuf {
    PathBuf::from(OsString::from_wide(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_selection_from_temporary_directory_with_trailing_separator() {
        let directory = tempfile::tempdir().unwrap();
        let mut temp = directory.path().as_os_str().to_owned();
        temp.push("\\");
        let path = directory.path().join("NestedUnpack-Selection-test.utf8");
        std::fs::write(&path, "C:\\第一 个.7z\r\nC:\\第二 个.zip\n").unwrap();

        assert_eq!(
            read_selection_in(&path, Path::new(&temp)).unwrap(),
            [
                PathBuf::from(r"C:\第一 个.7z"),
                PathBuf::from(r"C:\第二 个.zip")
            ],
        );
        assert!(!path.exists());
    }
}
