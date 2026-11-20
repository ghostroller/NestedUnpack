use std::io::{self, IsTerminal};
#[cfg(not(windows))]
use std::path::Path;
use std::path::PathBuf;

mod password_backup;
#[cfg(not(windows))]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
use unix as native;
#[cfg(windows)]
use windows as native;

pub use native::{
    attach_console, forget_password, menu_registered, open_directory, read_passwords,
    refresh_menu_language, register_menu, remember_password, remove_menu, run_explorer_server,
};
pub use password_backup::{export_passwords, import_passwords};

#[cfg(windows)]
pub use windows::read_selection;

#[cfg(not(windows))]
pub fn read_selection(_path: &Path) -> io::Result<Vec<PathBuf>> {
    use crate::i18n::{AppKey, Message};
    Err(Message::from(AppKey::ExplorerSelectionUnsupported).io(io::ErrorKind::Unsupported))
}

pub(crate) fn config_dir() -> Option<PathBuf> {
    let absolute_env = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    #[cfg(windows)]
    let base = absolute_env("LOCALAPPDATA").or_else(|| absolute_env("APPDATA"));
    #[cfg(target_os = "macos")]
    let base = absolute_env("HOME").map(|path| path.join("Library/Application Support"));
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = absolute_env("XDG_CONFIG_HOME")
        .or_else(|| absolute_env("HOME").map(|path| path.join(".config")));

    base.map(|path| path.join("NestedUnpack"))
}

pub fn read_console_password() -> io::Result<String> {
    let input = io::stdin();
    if input.is_terminal() {
        return rpassword::read_password();
    }
    // Keep redirected input usable without opening a terminal or echoing its contents.
    let mut value = String::new();
    input.read_line(&mut value)?;
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    Ok(value)
}

fn normalize_passwords(passwords: Vec<String>) -> Vec<String> {
    let mut unique = Vec::with_capacity(passwords.len());
    append_passwords(&mut unique, passwords);
    unique
}

fn append_passwords(passwords: &mut Vec<String>, imported: Vec<String>) -> usize {
    use zeroize::Zeroize;

    let original_count = passwords.len();
    for mut password in imported {
        if password.is_empty() || passwords.contains(&password) {
            password.zeroize();
        } else {
            passwords.push(password);
        }
    }
    passwords.len() - original_count
}

// This changes the order only when adding a new password, as in the previous book.
fn change_passwords(passwords: &mut Vec<String>, password: &str, remember: bool) -> bool {
    use zeroize::Zeroize;

    match (
        remember,
        passwords.iter().position(|value| value == password),
    ) {
        (true, None) if !password.is_empty() => {
            passwords.insert(0, password.to_owned());
            true
        }
        (false, Some(index)) => {
            passwords.remove(index).zeroize();
            true
        }
        _ => false,
    }
}
