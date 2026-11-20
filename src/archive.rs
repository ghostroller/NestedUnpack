mod input;
mod sevenz_archive;
#[cfg(test)]
mod tests;
mod zip_archive;

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::i18n::{EngineKey, Message, tr};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum Kind {
    Zip,
    SevenZip,
    Rar,
}

#[derive(Debug)]
pub(crate) enum ExtractError {
    Io(io::Error),
    Invalid(Message),
    Password,
    Skipped,
    Canceled,
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Invalid(message) => message.fmt(f),
            Self::Password => f.write_str(tr(EngineKey::PasswordOrCorrupt)),
            Self::Skipped => f.write_str(tr(EngineKey::NoPassword)),
            Self::Canceled => f.write_str(tr(EngineKey::ExtractionCanceled)),
        }
    }
}

impl ExtractError {
    pub(crate) fn into_message(self) -> Message {
        match self {
            Self::Io(error) => Message::from_error(error),
            Self::Invalid(message) => message,
            Self::Password => EngineKey::PasswordOrCorrupt.into(),
            Self::Skipped => EngineKey::NoPassword.into(),
            Self::Canceled => EngineKey::ExtractionCanceled.into(),
        }
    }
}

impl std::error::Error for ExtractError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ExtractError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(crate) trait Control {
    fn canceled(&self) -> bool;
    fn progress(&mut self, done: u64, total: Option<u64>);
    fn password(&mut self, retry: bool) -> Result<String, ExtractError>;
    fn accepted(&mut self, password: &str);
}

const RAR4: &[u8] = b"Rar!\x1a\x07\x00";
const RAR5: &[u8] = b"Rar!\x1a\x07\x01\x00";
const SEVEN_ZIP: &[u8] = b"7z\xbc\xaf\x27\x1c";

pub(crate) fn detect(path: &Path, scan_zip_tail: bool) -> io::Result<Option<Kind>> {
    let (mut file, length) = open_file(path)?;
    let mut header = [0; 8];
    let count = length.min(header.len() as u64) as usize;
    let header = &mut header[..count];
    file.read_exact(header)?;
    if header.starts_with(RAR4) || header.starts_with(RAR5) {
        return Ok(Some(Kind::Rar));
    }
    if header.starts_with(SEVEN_ZIP) {
        return Ok(Some(Kind::SevenZip));
    }
    if scan_zip_tail
        || matches!(
            header.get(..4),
            Some(b"PK\x03\x04" | b"PK\x05\x06" | b"PK\x06\x06")
        )
    {
        return zip_archive::open_archive(file, length).map(|archive| archive.map(|_| Kind::Zip));
    }
    Ok(None)
}

pub(crate) fn extract(
    kind: Kind,
    parts: &[PathBuf],
    destination: &Path,
    control: &mut dyn Control,
) -> Result<(), ExtractError> {
    let source = parts
        .first()
        .ok_or_else(|| ExtractError::Invalid(EngineKey::MissingArchivePath.into()))?;
    check_cancel(control)?;
    match kind {
        Kind::Zip => zip_archive::extract(source, destination, control, false),
        Kind::SevenZip => sevenz_archive::extract(parts, destination, control),
        Kind::Rar => crate::rar::extract(source, destination, control),
    }
}

pub(crate) fn extract_outer_rars(
    source: &Path,
    destination: &Path,
    control: &mut dyn Control,
) -> Result<(), ExtractError> {
    zip_archive::extract(source, destination, control, true)
}

pub(crate) fn safe_path(destination: &Path, name: &str) -> Result<PathBuf, ExtractError> {
    let normalized = name.strip_suffix(['/', '\\']).unwrap_or(name);
    let mut output = destination.to_owned();
    for part in normalized.split(['/', '\\']) {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with([' ', '.'])
            || part.contains(['<', '>', ':', '"', '|', '?', '*'])
            || part.chars().any(|character| character < '\u{20}')
            || windows_device_name(part)
        {
            return Err(ExtractError::Invalid(
                Message::from(EngineKey::UnsupportedArchivePath).arg(format!("{name:?}")),
            ));
        }
        output.push(part);
    }
    Ok(output)
}

pub(crate) fn windows_device_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end_matches(' ');
    let stem = stem.to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    matches!(
        stem.strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT")),
        Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
    )
}

fn open_file(path: &Path) -> io::Result<(File, u64)> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Message::from(EngineKey::ReadRegularFile).io(io::ErrorKind::InvalidInput));
    }
    Ok((file, metadata.len()))
}

fn check_cancel(control: &dyn Control) -> Result<(), ExtractError> {
    if control.canceled() {
        Err(ExtractError::Canceled)
    } else {
        Ok(())
    }
}

fn request_password(
    control: &mut dyn Control,
    previous: &mut Zeroizing<String>,
) -> Result<(), ExtractError> {
    check_cancel(control)?;
    let password = Zeroizing::new(control.password(!previous.is_empty())?);
    if password.is_empty() {
        return Err(ExtractError::Skipped);
    }
    *previous = password;
    Ok(())
}

const COPY_BUFFER: usize = 1 << 20;

#[derive(Debug)]
enum CopyError {
    Source(io::Error),
    Output(io::Error),
    Canceled,
}

impl From<CopyError> for ExtractError {
    fn from(error: CopyError) -> Self {
        match error {
            CopyError::Source(error) | CopyError::Output(error) => error.into(),
            CopyError::Canceled => Self::Canceled,
        }
    }
}

fn copy_member(
    reader: &mut dyn Read,
    path: &Path,
    size: u64,
    mut position: u64,
    total: Option<u64>,
    buffer: &mut [u8],
    control: &mut dyn Control,
) -> Result<(), CopyError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(CopyError::Output)?;
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(CopyError::Output)?;
    let result = (|| {
        let mut remaining = size;
        loop {
            if control.canceled() {
                return Err(CopyError::Canceled);
            }
            // Read to EOF to let decryptors and decompressors check integrity.
            let capacity = (buffer.len() as u64).min(remaining.saturating_add(1)) as usize;
            let length = reader
                .read(&mut buffer[..capacity])
                .map_err(CopyError::Source)?;
            if length as u64 > remaining {
                return Err(CopyError::Source(
                    Message::from(EngineKey::ExtractedTooLong).io(io::ErrorKind::InvalidData),
                ));
            }
            if length == 0 {
                if remaining != 0 {
                    return Err(CopyError::Source(
                        Message::from(EngineKey::ExtractedTooShort)
                            .io(io::ErrorKind::UnexpectedEof),
                    ));
                }
                return Ok(());
            }
            output
                .write_all(&buffer[..length])
                .map_err(CopyError::Output)?;
            remaining -= length as u64;
            position += length as u64;
            control.progress(position, total);
        }
    })();
    drop(output);
    if result.is_err() {
        fs::remove_file(path).map_err(CopyError::Output)?;
    }
    result
}
