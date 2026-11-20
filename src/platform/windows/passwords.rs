use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ::windows::Win32::Foundation::{HLOCAL, LocalFree};
use ::windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use ::windows::core::PCWSTR;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    i18n::{AppKey, Message},
    platform::{change_passwords, config_dir, normalize_passwords},
};

fn book_path() -> io::Result<PathBuf> {
    config_dir()
        .map(|directory| directory.join("passwords.dpapi"))
        .ok_or_else(|| Message::from(AppKey::NoConfigDirectory).io(io::ErrorKind::Other))
}

fn lock_file(path: &Path) -> io::Result<File> {
    let mut lock = OsString::from(path.as_os_str());
    lock.push(".lock");
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock)
}

pub fn read_passwords() -> io::Result<Vec<String>> {
    let path = book_path()?;
    let lock = match lock_file(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    lock.lock_shared()?;
    read_unlocked(&path)
}

pub fn remember_password(password: &str) -> io::Result<()> {
    update(&book_path()?, password, true)
}

pub fn forget_password(password: &str) -> io::Result<()> {
    update(&book_path()?, password, false)
}

fn update(path: &Path, password: &str, remember: bool) -> io::Result<()> {
    if password.is_empty() {
        return Ok(());
    }
    let parent = path.parent().ok_or_else(invalid_format)?;
    fs::create_dir_all(parent)?;
    let lock = lock_file(path)?;
    // The original Go implementation locked the first byte of this same file.
    lock.lock()?;
    let mut passwords = Zeroizing::new(read_unlocked(path)?);
    if change_passwords(&mut passwords, password, remember) {
        let encoded = Zeroizing::new(encode_book(&passwords)?);
        let protected = protect(&encoded, true)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&protected)?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|error| error.error)?;
    }
    Ok(())
}

fn read_unlocked(path: &Path) -> io::Result<Vec<String>> {
    let encrypted = match fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let plain = Zeroizing::new(protect(&encrypted, false)?);
    decode_book(&plain)
}

fn invalid_format() -> io::Error {
    Message::from(AppKey::InvalidPasswordBook).io(io::ErrorKind::InvalidData)
}

fn encode_book(passwords: &[String]) -> io::Result<Vec<u8>> {
    // Go encodes its [][]byte payload as JSON strings containing standard base64.
    let entries = Zeroizing::new(
        passwords
            .iter()
            .map(|password| STANDARD.encode(password.as_bytes()))
            .collect::<Vec<_>>(),
    );
    serde_json::to_vec(&*entries).map_err(|_| invalid_format())
}

fn decode_book(plain: &[u8]) -> io::Result<Vec<String>> {
    let mut entries: Option<Vec<Option<String>>> =
        serde_json::from_slice(plain).map_err(|_| invalid_format())?;
    let entries = entries.get_or_insert_default();
    let mut passwords = Zeroizing::new(Vec::with_capacity(entries.len()));
    let result = entries.iter().flatten().try_for_each(|entry| {
        let decoded = STANDARD.decode(entry).map_err(|_| invalid_format())?;
        let password = String::from_utf8(decoded).map_err(|error| {
            error.into_bytes().zeroize();
            invalid_format()
        })?;
        passwords.push(password);
        Ok::<_, io::Error>(())
    });
    for entry in entries.iter_mut().flatten() {
        entry.zeroize();
    }
    result?;
    Ok(normalize_passwords(std::mem::take(&mut *passwords)))
}

#[derive(Default)]
struct DpapiOutput(CRYPT_INTEGER_BLOB);

impl Drop for DpapiOutput {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // The API allocates this owned buffer with LocalAlloc.
            unsafe {
                std::slice::from_raw_parts_mut(self.0.pbData, self.0.cbData as usize).zeroize();
                let _ = LocalFree(Some(HLOCAL(self.0.pbData.cast())));
            }
        }
    }
}

fn protect(data: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
    let length = u32::try_from(data.len()).map_err(|_| invalid_format())?;
    if length == 0 {
        return Err(invalid_format());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: length,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut output = DpapiOutput::default();
    let result = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        }
    };
    result.map_err(|error| {
        let action = if encrypt {
            AppKey::EncryptPasswordBookFailed
        } else {
            AppKey::DecryptPasswordBookFailed
        };
        Message::from(action)
            .arg(error.to_string())
            .io(io::ErrorKind::Other)
    })?;
    if output.0.pbData.is_null() || output.0.cbData == 0 {
        return Err(invalid_format());
    }
    // Copy before reclaiming and erasing the original DPAPI buffer.
    Ok(unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_password_book_payload_and_updates() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("passwords.dpapi");
        let old_payload = br#"["5Lit5paH",null,"","aW50ZXJvcA==","5Lit5paH"]"#;
        fs::write(&path, protect(old_payload, true).unwrap()).unwrap();

        assert_eq!(read_unlocked(&path).unwrap(), ["中文", "interop"]);
        update(&path, "  有空格的密码  ", true).unwrap();
        update(&path, "中文", true).unwrap();
        update(&path, "interop", false).unwrap();
        assert_eq!(read_unlocked(&path).unwrap(), ["  有空格的密码  ", "中文"]);

        let clear = protect(&fs::read(&path).unwrap(), false).unwrap();
        let entries: Vec<String> = serde_json::from_slice(&clear).unwrap();
        assert_eq!(entries[1], "5Lit5paH");
    }
}
