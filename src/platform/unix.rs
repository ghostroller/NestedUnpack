use std::io;
use std::path::Path;

use crate::i18n::{AppKey, Message};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod passwords {
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};

    use keyring::{Entry, Error};
    use zeroize::Zeroizing;

    use crate::platform::{
        append_passwords, change_passwords, config_dir, normalize_passwords,
        password_backup::ensure_distinct_destination,
    };

    use super::{AppKey, Message};

    fn lock_path() -> io::Result<PathBuf> {
        config_dir()
            .map(|directory| directory.join("passwords.lock"))
            .ok_or_else(|| Message::from(AppKey::NoConfigDirectory).io(io::ErrorKind::Other))
    }

    fn lock_file() -> io::Result<File> {
        let path = lock_path()?;
        let directory = path
            .parent()
            .expect("lock path has a configuration directory");
        fs::create_dir_all(directory)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
    }

    pub(crate) fn validate_backup_destination(path: &Path) -> io::Result<()> {
        ensure_distinct_destination(path, &lock_path()?)
    }

    fn entry() -> io::Result<Entry> {
        Entry::new("com.nestedunpack.app", "archive-password-book").map_err(store_error)
    }

    fn store_error(error: Error) -> io::Error {
        Message::from(AppKey::SystemPasswordStoreFailed)
            .arg(error.to_string())
            .io(io::ErrorKind::Other)
    }

    fn read_unlocked(entry: &Entry) -> io::Result<Vec<String>> {
        let secret = match entry.get_secret() {
            Ok(secret) => Zeroizing::new(secret),
            Err(Error::NoEntry) => return Ok(Vec::new()),
            Err(error) => return Err(store_error(error)),
        };
        serde_json::from_slice(&secret)
            .map(normalize_passwords)
            .map_err(|_| Message::from(AppKey::InvalidPasswordBook).io(io::ErrorKind::InvalidData))
    }

    pub fn read_passwords() -> io::Result<Vec<String>> {
        let lock = lock_file()?;
        lock.lock_shared()?;
        read_unlocked(&entry()?)
    }

    fn update(password: &str, remember: bool) -> io::Result<()> {
        if password.is_empty() {
            return Ok(());
        }
        edit_book(|passwords| usize::from(change_passwords(passwords, password, remember)))?;
        Ok(())
    }

    fn edit_book(edit: impl FnOnce(&mut Vec<String>) -> usize) -> io::Result<usize> {
        let lock = lock_file()?;
        lock.lock()?;
        let entry = entry()?;
        let mut passwords = Zeroizing::new(read_unlocked(&entry)?);
        let changed = edit(&mut passwords);
        if changed != 0 {
            let secret = Zeroizing::new(serde_json::to_vec(&*passwords)?);
            entry.set_secret(&secret).map_err(store_error)?;
        }
        Ok(changed)
    }

    pub(crate) fn merge_passwords(imported: &mut Vec<String>) -> io::Result<usize> {
        edit_book(|passwords| append_passwords(passwords, std::mem::take(imported)))
    }

    pub fn remember_password(password: &str) -> io::Result<()> {
        update(password, true)
    }

    pub fn forget_password(password: &str) -> io::Result<()> {
        update(password, false)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use passwords::{forget_password, read_passwords, remember_password};
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) use passwords::{merge_passwords, validate_backup_destination};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn merge_passwords(_imported: &mut Vec<String>) -> io::Result<usize> {
    Err(unsupported())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn read_passwords() -> io::Result<Vec<String>> {
    Err(unsupported())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn remember_password(_password: &str) -> io::Result<()> {
    Err(unsupported())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn forget_password(_password: &str) -> io::Result<()> {
    Err(unsupported())
}

pub fn open_directory(path: &Path) -> io::Result<()> {
    let absolute = std::path::absolute(path)?;
    if !absolute.metadata()?.is_dir() {
        return Err(Message::from(AppKey::OutputNotDirectory).io(io::ErrorKind::NotADirectory));
    }
    #[cfg(target_os = "macos")]
    let program = "/usr/bin/open";
    #[cfg(not(target_os = "macos"))]
    let program = "xdg-open";
    let mut child = std::process::Command::new(program)
        .arg(absolute)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    // Some desktop openers remain alive for the lifetime of the file manager.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn attach_console() {}

fn unsupported() -> io::Error {
    Message::from(AppKey::PlatformUnsupported).io(io::ErrorKind::Unsupported)
}

pub fn menu_registered() -> io::Result<bool> {
    Ok(false)
}

pub fn refresh_menu_language() -> io::Result<()> {
    Ok(())
}

pub fn register_menu() -> io::Result<()> {
    Err(unsupported())
}

pub fn remove_menu() -> io::Result<()> {
    Err(unsupported())
}

pub fn run_explorer_server() -> i32 {
    2
}
