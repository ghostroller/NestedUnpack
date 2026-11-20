use std::io::{self, IsTerminal};
#[cfg(not(windows))]
use std::path::{Path, PathBuf};

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
    register_menu, remember_password, remove_menu, run_explorer_server,
};

#[cfg(windows)]
pub use windows::read_selection;

#[cfg(not(windows))]
pub fn read_selection(_path: &Path) -> io::Result<Vec<PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "此平台不支持资源管理器选区",
    ))
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
    use zeroize::Zeroize;

    let mut unique = Vec::with_capacity(passwords.len());
    for mut password in passwords {
        if password.is_empty() || unique.contains(&password) {
            password.zeroize();
        } else {
            unique.push(password);
        }
    }
    unique
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
