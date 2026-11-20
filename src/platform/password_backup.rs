use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde_json::{Map, Value};
use zeroize::Zeroizing;

use crate::i18n::{AppKey, Message};

const FORMAT: &str = "nestedunpack-password-book";
const VERSION: u64 = 1;

pub fn import_passwords(path: &Path) -> io::Result<usize> {
    let bytes = Zeroizing::new(fs::read(path)?);
    let mut passwords = decode_backup(&bytes)?;
    super::native::merge_passwords(&mut passwords)
}

pub fn export_passwords(path: &Path) -> io::Result<()> {
    let path = std::path::absolute(path)?;
    let passwords = Zeroizing::new(super::read_passwords()?);
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    super::native::validate_backup_destination(&path)?;
    write_backup(&path, &passwords)
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub(super) fn ensure_distinct_destination(selected: &Path, protected: &Path) -> io::Result<()> {
    #[cfg(windows)]
    let equal = |left: &Path, right: &Path| {
        super::windows::equal_ignore_case(left.as_os_str(), right.as_os_str())
    };
    #[cfg(not(windows))]
    let equal = |left: &Path, right: &Path| left == right;
    let resolve = |path: &Path| {
        path.canonicalize()
            .ok()
            .or_else(|| Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?)))
    };
    let same_path = equal(selected, protected)
        || resolve(selected)
            .zip(resolve(protected))
            .is_some_and(|(destination, reserved)| equal(&destination, &reserved));
    // Unix files can have distinct paths but the same inode, including case aliases on macOS.
    #[cfg(not(windows))]
    let same_path = same_path || same_unix_file(selected, protected);
    if same_path {
        return Err(
            Message::from(AppKey::ProtectedPasswordBackupPath).io(io::ErrorKind::InvalidInput)
        );
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn same_unix_file(left: &Path, right: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.metadata()
        .ok()
        .zip(right.metadata().ok())
        .is_some_and(|(left, right)| left.dev() == right.dev() && left.ino() == right.ino())
}

fn write_backup(path: &Path, passwords: &[String]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    // Serialize the password array directly so the file writer owns no extra copy.
    write!(
        output,
        "{{\n  \"format\": \"{FORMAT}\",\n  \"version\": {VERSION},\n  \"passwords\": "
    )?;
    serde_json::to_writer(&mut output, passwords)?;
    writeln!(output, "\n}}")?;
    output.as_file().sync_all()?;
    output.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn decode_backup(bytes: &[u8]) -> io::Result<Zeroizing<Vec<String>>> {
    let mut document: Map<String, Value> =
        serde_json::from_slice(bytes).map_err(|_| invalid_backup())?;
    let passwords = document
        .remove("passwords")
        .ok_or_else(invalid_backup)
        .and_then(|value| serde_json::from_value(value).map_err(|_| invalid_backup()))
        .map(Zeroizing::new);
    let format = document.get("format").and_then(Value::as_str);
    let version = document.get("version").and_then(Value::as_u64);
    match (format, version) {
        (Some(FORMAT), Some(VERSION)) => passwords,
        (Some(FORMAT), Some(_)) => {
            Err(Message::from(AppKey::UnsupportedPasswordBackupVersion)
                .io(io::ErrorKind::Unsupported))
        }
        _ => Err(invalid_backup()),
    }
}

fn invalid_backup() -> io::Error {
    Message::from(AppKey::InvalidPasswordBackup).io(io::ErrorKind::InvalidData)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_round_trip_preserves_text_and_merges_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.json");
        fs::write(&path, b"previous backup").unwrap();
        let entries = [
            "  档案 🔑\t",
            "already saved",
            "escaped\n\"quote\"",
            "  档案 🔑\t",
        ]
        .map(str::to_owned);
        write_backup(&path, &entries).unwrap();
        let mut imported = decode_backup(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(imported.as_slice(), &entries);

        let mut book = vec!["first".to_owned(), "already saved".to_owned()];
        assert_eq!(
            super::super::append_passwords(&mut book, std::mem::take(&mut *imported)),
            2
        );
        assert_eq!(
            book,
            [
                "first",
                "already saved",
                "  档案 🔑\t",
                "escaped\n\"quote\""
            ]
        );
    }

    #[test]
    fn rejects_wrong_format_version_and_entry_type() {
        for invalid in [
            "{",
            "[]",
            r#"{"format":"other","version":1,"passwords":[]}"#,
            r#"{"format":"nestedunpack-password-book","version":"1","passwords":[]}"#,
            r#"{"format":"nestedunpack-password-book","version":1,"passwords":["good",false]}"#,
        ] {
            assert_eq!(
                decode_backup(invalid.as_bytes()).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        let newer = br#"{"format":"nestedunpack-password-book","version":2,"passwords":[]}"#;
        assert_eq!(
            decode_backup(newer).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }
}
