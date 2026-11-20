use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;
use zip::read::ZipReadOptions;
use zip::result::ZipError;
use zip::{System, ZipArchive};

use crate::i18n::{EngineKey, Message};

use super::input::ZipView;
use super::{
    COPY_BUFFER, Control, CopyError, ExtractError, RAR4, RAR5, check_cancel, copy_member,
    open_file, request_password, safe_path,
};

const TAIL_WINDOW: u64 = 1 << 20;
const END_HEADER_SIZE: usize = 22;
type Reader = ZipArchive<BufReader<ZipView>>;

struct Section {
    start: u64,
    end: u64,
    count: u64,
    count_mask: Option<u64>,
}

pub(super) fn open_archive(mut file: File, length: u64) -> io::Result<Option<Reader>> {
    if length < END_HEADER_SIZE as u64 {
        return Ok(None);
    }
    let tail_size = length.min(TAIL_WINDOW);
    let tail_start = length - tail_size;
    let mut tail = vec![0; tail_size as usize];
    file.seek(SeekFrom::Start(tail_start))?;
    file.read_exact(&mut tail)?;

    let mut limit = tail.len();
    while let Some(end) = find_last(&tail[..limit], b"PK\x05\x06") {
        limit = end;
        let Some(section) = locate_section(&tail, tail_start, end, length) else {
            continue;
        };
        let view = ZipView::new(
            file.try_clone()?,
            section.start,
            section.end - section.start,
            section.count_mask,
        )?;
        if let Some(archive) = parse(view)?
            && (archive.len() as u64 == section.count || archive.has_duplicate_names())
        {
            return Ok(Some(archive));
        }
    }

    // ZIP readers can also recognize archives with physical (already prefixed)
    // central-directory offsets. Allow the normal parser to handle those.
    parse(ZipView::new(file, 0, length, None)?)
}

fn parse(view: ZipView) -> io::Result<Option<Reader>> {
    match ZipArchive::new(BufReader::with_capacity(128 << 10, view)) {
        Ok(archive) => Ok(Some(archive)),
        Err(ZipError::Io(error)) if error.raw_os_error().is_some() => Err(error),
        Err(_) => Ok(None),
    }
}

fn locate_section(tail: &[u8], tail_start: u64, end: usize, length: u64) -> Option<Section> {
    let eocd = tail.get(end..end.checked_add(END_HEADER_SIZE)?)?;
    let footer = tail_start + end as u64;
    let end_of_zip = footer.checked_add(END_HEADER_SIZE as u64 + u64::from(le16(&eocd[20..])))?;
    if end_of_zip > length
        || le16(&eocd[4..]) != 0
        || le16(&eocd[6..]) != 0
        || le16(&eocd[8..]) != le16(&eocd[10..])
    {
        return None;
    }
    if let Some(locator) = end.checked_sub(20)
        && &tail[locator..locator + 4] == b"PK\x06\x07"
    {
        return locate_zip64(tail, tail_start, locator, end_of_zip);
    }

    let directory_size = u64::from(le32(&eocd[12..]));
    let directory_offset = u64::from(le32(&eocd[16..]));
    let start = footer.checked_sub(directory_size + directory_offset)?;
    Some(Section {
        start,
        end: end_of_zip,
        count: u64::from(le16(&eocd[10..])),
        count_mask: None,
    })
}

fn locate_zip64(tail: &[u8], tail_start: u64, locator: usize, end: u64) -> Option<Section> {
    let loc = &tail[locator..locator + 20];
    if le32(&loc[4..]) != 0 || le32(&loc[16..]) != 1 {
        return None;
    }
    let locator_offset = le64(&loc[8..]);
    let mut limit = locator;
    while let Some(index) = find_last(&tail[..limit], b"PK\x06\x06") {
        limit = index;
        let record = &tail[index..locator];
        if record.len() < 56
            || le64(&record[4..]) != (record.len() - 12) as u64
            || le32(&record[16..]) != 0
            || le32(&record[20..]) != 0
        {
            continue;
        }
        let count = le64(&record[32..]);
        let directory_size = le64(&record[40..]);
        let directory_offset = le64(&record[48..]);
        let footer = tail_start + index as u64;
        if count != le64(&record[24..])
            || directory_offset.checked_add(directory_size) != Some(locator_offset)
            || locator_offset > footer
        {
            continue;
        }
        let start = footer - locator_offset;
        return Some(Section {
            start,
            end,
            count,
            count_mask: Some(tail_start + locator as u64 + 28 - start),
        });
    }
    None
}

fn find_last(bytes: &[u8], signature: &[u8; 4]) -> Option<usize> {
    bytes
        .windows(4)
        .rposition(|candidate| candidate == signature)
}

fn le16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes[..2].try_into().unwrap())
}
fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}
fn le64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[..8].try_into().unwrap())
}

struct Entry {
    index: usize,
    name: String,
    path: PathBuf,
    directory: bool,
    encrypted: bool,
    size: u64,
}

pub(super) fn extract(
    source: &Path,
    destination: &Path,
    control: &mut dyn Control,
    outer_rars_only: bool,
) -> Result<(), ExtractError> {
    check_cancel(control)?;
    let (file, length) = open_file(source)?;
    let mut archive = open_archive(file, length)?
        .ok_or_else(|| ExtractError::Invalid(EngineKey::NotZip.into()))?;
    if archive.has_duplicate_names() {
        return Err(ExtractError::Invalid(EngineKey::DuplicateZipEntries.into()));
    }

    let mut entries = Vec::with_capacity(archive.len());
    let mut total = 0u64;
    for index in 0..archive.len() {
        check_cancel(control)?;
        let entry = archive.by_index_data(index).map_err(zip_error)?;
        let name = entry.name().map_err(zip_error)?;
        if outer_rars_only
            && (entry.is_dir()
                || !name
                    .rsplit_once('.')
                    .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("rar")))
        {
            continue;
        }
        let mode = entry.unix_mode().unwrap_or(0) & 0o170000;
        let reparse_point =
            entry.system() != System::Unix && entry.external_attributes() & 0x400 != 0;
        if entry.is_symlink() || reparse_point || !matches!(mode, 0 | 0o040000 | 0o100000) {
            return Err(ExtractError::Invalid(
                Message::from(EngineKey::UnsupportedZipEntry).arg(format!("{name:?}")),
            ));
        }
        let path = safe_path(destination, &name)?;
        let size = entry.size();
        total = total
            .checked_add(size)
            .ok_or_else(|| ExtractError::Invalid(EngineKey::ZipContentTooLarge.into()))?;
        entries.push(Entry {
            index,
            name: name.into_owned(),
            path,
            directory: entry.is_dir(),
            encrypted: entry.encrypted(),
            size,
        });
    }

    let total = (total != 0).then_some(total);
    control.progress(0, total);
    let mut done = 0;
    let mut buffer = vec![0; COPY_BUFFER];
    for entry in entries {
        check_cancel(control)?;
        if entry.directory {
            fs::create_dir_all(&entry.path)?;
        } else {
            extract_member(
                &mut archive,
                &entry,
                done,
                total,
                &mut buffer,
                control,
                outer_rars_only,
            )?;
        }
        done += entry.size;
    }
    Ok(())
}

fn extract_member(
    archive: &mut Reader,
    entry: &Entry,
    done: u64,
    total: Option<u64>,
    buffer: &mut [u8],
    control: &mut dyn Control,
    validate_rar: bool,
) -> Result<(), ExtractError> {
    let mut password = Zeroizing::new(String::new());
    loop {
        check_cancel(control)?;
        let candidate = (!password.is_empty()).then_some(password.as_bytes());
        let result = archive
            .by_index_with_options(entry.index, ZipReadOptions::new().password(candidate))
            .map_err(zip_error)
            .and_then(|mut file| {
                copy_member(
                    &mut file,
                    &entry.path,
                    entry.size,
                    done,
                    total,
                    buffer,
                    control,
                )
                .map_err(|error| match error {
                    CopyError::Source(error)
                        if entry.encrypted && error.raw_os_error().is_none() =>
                    {
                        ExtractError::Password
                    }
                    error => error.into(),
                })
            });
        match result {
            Err(ExtractError::Password) => request_password(control, &mut password)?,
            Err(ExtractError::Io(error)) => {
                let kind = error.kind();
                return Err(Message::from(EngineKey::ArchiveEntryError)
                    .arg(entry.name.as_str())
                    .arg(Message::from_error(error))
                    .io(kind)
                    .into());
            }
            Err(error) => return Err(error),
            Ok(()) => {
                if validate_rar && !has_rar_signature(&entry.path)? {
                    fs::remove_file(&entry.path)?;
                    return Err(ExtractError::Invalid(
                        Message::from(EngineKey::NoRarSignature).arg(entry.name.as_str()),
                    ));
                }
                if !password.is_empty() {
                    control.accepted(&password);
                }
                return Ok(());
            }
        }
    }
}

fn has_rar_signature(path: &Path) -> io::Result<bool> {
    let (mut file, length) = open_file(path)?;
    let mut signature = [0; 8];
    let count = length.min(signature.len() as u64) as usize;
    file.read_exact(&mut signature[..count])?;
    Ok(signature[..count].starts_with(RAR4) || signature[..count].starts_with(RAR5))
}

fn zip_error(error: ZipError) -> ExtractError {
    match error {
        ZipError::InvalidPassword | ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED) => {
            ExtractError::Password
        }
        ZipError::Io(error) => error.into(),
        error => ExtractError::Invalid(Message::raw(error.to_string())),
    }
}
