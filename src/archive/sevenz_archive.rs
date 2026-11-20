use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use sevenz_turbo::{ArchiveEntry, ArchiveLimits, ArchiveReader, BlockErrorKind, Error, Password};
use zeroize::Zeroizing;

use crate::i18n::{EngineKey, Message};

use super::input::JoinedVolumes;
use super::{
    COPY_BUFFER, Control, CopyError, ExtractError, check_cancel, copy_member, request_password,
    safe_path,
};

pub(super) fn extract(
    parts: &[PathBuf],
    destination: &Path,
    control: &mut dyn Control,
) -> Result<(), ExtractError> {
    fs::create_dir_all(destination)?;
    if fs::symlink_metadata(destination)?.file_type().is_symlink()
        || fs::read_dir(destination)?.next().is_some()
    {
        return Err(ExtractError::Invalid(
            EngineKey::NonEmptySevenZipStaging.into(),
        ));
    }
    let mut password = Zeroizing::new(String::new());
    let mut buffer = vec![0; COPY_BUFFER];
    loop {
        check_cancel(control)?;
        match extract_attempt(parts, destination, &password, &mut buffer, control) {
            Ok(()) => {
                if !password.is_empty() {
                    control.accepted(&password);
                }
                return Ok(());
            }
            Err(ExtractError::Password) => {
                // A 7z archive may start with unencrypted entries and then fail
                // on an encrypted stream. Discard this attempt before retrying.
                for item in fs::read_dir(destination)? {
                    let item = item?;
                    if item.file_type()?.is_dir() {
                        fs::remove_dir_all(item.path())?;
                    } else {
                        fs::remove_file(item.path())?;
                    }
                }
                request_password(control, &mut password)?;
            }
            Err(error) => return Err(error),
        }
    }
}

fn extract_attempt(
    parts: &[PathBuf],
    destination: &Path,
    password: &str,
    buffer: &mut [u8],
    control: &mut dyn Control,
) -> Result<(), ExtractError> {
    let input = BufReader::with_capacity(256 << 10, JoinedVolumes::open(parts)?);
    let limits = ArchiveLimits {
        max_end_header_bytes: 64 << 20,
        ..ArchiveLimits::default()
    };
    let mut archive =
        ArchiveReader::with_limits(input, Password::new(password), limits).map_err(sevenz_error)?;
    if let Ok(threads) = std::thread::available_parallelism() {
        archive.set_threads(threads.get().min(4) as u32);
    }

    let mut total = 0u64;
    for entry in &archive.archive().files {
        check_cancel(control)?;
        validate_entry(entry, destination)?;
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| ExtractError::Invalid(EngineKey::SevenZipContentTooLarge.into()))?;
    }
    let total = (total != 0).then_some(total);
    let mut done = 0;
    let mut output_error = None;
    control.progress(0, total);
    let result = archive.for_each_entries(|entry, input| {
        if control.canceled() {
            return Err(Error::Cancelled);
        }
        let path = match safe_path(destination, entry.name()) {
            Ok(path) => path,
            Err(error) => {
                output_error = Some(error);
                return Err(Error::Cancelled);
            }
        };
        let output = if entry.is_directory() {
            fs::create_dir_all(path).map_err(CopyError::Output)
        } else {
            copy_member(input, &path, entry.size(), done, total, buffer, control)
        };
        match output {
            Ok(()) => {
                done += entry.size();
                Ok(true)
            }
            Err(CopyError::Source(error)) => Err(error.into()),
            Err(CopyError::Canceled) => Err(Error::Cancelled),
            Err(CopyError::Output(error)) => {
                output_error = Some(ExtractError::Io(error));
                Err(Error::Cancelled)
            }
        }
    });
    match output_error {
        Some(error) => Err(error),
        None => result.map_err(sevenz_error),
    }
}

fn validate_entry(entry: &ArchiveEntry, destination: &Path) -> Result<(), ExtractError> {
    let attributes = entry.windows_attributes();
    let mode = (attributes >> 16) & 0o170000;
    if entry.is_anti_item() || attributes & 0x400 != 0 || !matches!(mode, 0 | 0o040000 | 0o100000) {
        return Err(ExtractError::Invalid(
            Message::from(EngineKey::UnsupportedSevenZipEntry).arg(format!("{:?}", entry.name())),
        ));
    }
    safe_path(destination, entry.name())?;
    Ok(())
}

fn sevenz_error(error: Error) -> ExtractError {
    match error {
        Error::Cancelled => ExtractError::Canceled,
        Error::Io(error, _) | Error::FileOpen(error, _) if error.raw_os_error().is_some() => {
            ExtractError::Io(error)
        }
        error if error.is_password_required() => ExtractError::Password,
        Error::MaybeBadPassword(error) if error.raw_os_error().is_some() => ExtractError::Io(error),
        Error::MaybeBadPassword(_)
        | Error::BlockDecode {
            kind: BlockErrorKind::Password,
            ..
        } => ExtractError::Password,
        error => ExtractError::Invalid(
            error
                .io_error()
                .and_then(|error| error.get_ref())
                .and_then(|error| error.downcast_ref::<Message>())
                .cloned()
                .unwrap_or_else(|| Message::raw(error.to_string())),
        ),
    }
}
