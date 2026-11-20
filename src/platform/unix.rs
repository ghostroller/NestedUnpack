use std::io;
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod passwords {
    use std::env;
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::PathBuf;

    use keyring::{Entry, Error};
    use zeroize::Zeroizing;

    use crate::platform::{change_passwords, normalize_passwords};

    fn lock_file() -> io::Result<File> {
        #[cfg(target_os = "linux")]
        let config = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|path| path.join(".config"))
            });
        #[cfg(target_os = "macos")]
        let config = env::var_os("HOME")
            .map(PathBuf::from)
            .map(|path| path.join("Library/Application Support"));
        let directory = config
            .filter(|path| path.is_absolute())
            .ok_or_else(|| io::Error::other("无法定位当前用户的配置目录"))?
            .join("NestedUnpack");
        fs::create_dir_all(&directory)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(directory.join("passwords.lock"))
    }

    fn entry() -> io::Result<Entry> {
        Entry::new("com.nestedunpack.app", "archive-password-book").map_err(store_error)
    }

    fn store_error(error: Error) -> io::Error {
        io::Error::other(format!("无法访问系统密码库：{error}"))
    }

    fn read_unlocked(entry: &Entry) -> io::Result<Vec<String>> {
        let secret = match entry.get_secret() {
            Ok(secret) => Zeroizing::new(secret),
            Err(Error::NoEntry) => return Ok(Vec::new()),
            Err(error) => return Err(store_error(error)),
        };
        serde_json::from_slice(&secret)
            .map(normalize_passwords)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "密码簿格式无效"))
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
        let lock = lock_file()?;
        lock.lock()?;
        let entry = entry()?;
        let mut passwords = Zeroizing::new(read_unlocked(&entry)?);
        if change_passwords(&mut passwords, password, remember) {
            let secret = Zeroizing::new(serde_json::to_vec(&*passwords)?);
            entry.set_secret(&secret).map_err(store_error)?;
        }
        Ok(())
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
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "输出路径不是文件夹",
        ));
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
    io::Error::new(io::ErrorKind::Unsupported, "此平台不支持该操作")
}

pub fn menu_registered() -> io::Result<bool> {
    Ok(false)
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
