use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStringExt;

use ::windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use ::windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, KEY_WRITE,
    REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteTreeW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use ::windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use ::windows::core::{GUID, PCWSTR};

use super::super::{equal_ignore_case, wide};

pub(super) const CLASS: GUID = GUID::from_u128(0x9e941531_6a19_4358_a773_30cd664be034);
const CLASS_TEXT: &str = "{9E941531-6A19-4358-A773-30CD664BE034}";
const VERB: &str = r"Software\Classes\*\shell\NestedUnpack.Extract";
const COMMAND: &str = r"Software\Classes\*\shell\NestedUnpack.Extract\command";
const CLASS_KEY: &str = r"Software\Classes\CLSID\{9E941531-6A19-4358-A773-30CD664BE034}";
const SERVER: &str = r"Software\Classes\CLSID\{9E941531-6A19-4358-A773-30CD664BE034}\LocalServer32";
const APPLIES_TO: &str = concat!(
    r#"System.FileExtension:=".zip" OR System.FileExtension:=".zipx" OR System.FileExtension:=".mp4" OR "#,
    r#"System.FileExtension:=".rar" OR System.FileExtension:=".7z" OR System.FileName:~>".7z.001""#,
);
const OLD_KEYS: [&str; 3] = [
    r"Software\Classes\*\shell\OuterRar.ExtractNested",
    r"Software\Classes\*\shell\OuterRar.Extract",
    r"Software\Classes\CLSID\{DF5835F3-405C-4950-94CE-121C0EA39879}",
];

fn server_command(exe: &OsStr) -> OsString {
    let mut command = OsString::from("\"");
    command.push(exe);
    command.push("\" --explorer-server");
    command
}

pub fn menu_registered() -> io::Result<bool> {
    let executable = std::env::current_exe()?;
    let exe = executable.as_os_str();
    let command = server_command(exe);
    let expected = [
        (SERVER, "", command.as_os_str()),
        (SERVER, "ServerExecutable", exe),
        (COMMAND, "DelegateExecute", OsStr::new(CLASS_TEXT)),
    ];
    for (key, name, expected) in expected {
        let value = match read_value(key, name) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if !equal_ignore_case(&value, expected) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn register_menu() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let _notify = AssociationChange;
    write_values(
        SERVER,
        &[
            ("", &server_command(exe.as_os_str())),
            ("ServerExecutable", exe.as_os_str()),
        ],
    )?;
    write_values(COMMAND, &[("DelegateExecute", OsStr::new(CLASS_TEXT))])?;
    write_values(
        VERB,
        &[
            ("MUIVerb", OsStr::new("解开嵌套压缩包")),
            ("NeverDefault", OsStr::new("")),
            ("MultiSelectModel", OsStr::new("Player")),
            ("AppliesTo", OsStr::new(APPLIES_TO)),
        ],
    )?;
    delete_keys(OLD_KEYS)
}

pub fn remove_menu() -> io::Result<()> {
    let _notify = AssociationChange;
    delete_keys([VERB, CLASS_KEY].into_iter().chain(OLD_KEYS))
}

struct AssociationChange;

impl Drop for AssociationChange {
    fn drop(&mut self) {
        // Tell the shell to clear its cached associations after either operation.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
}

struct Key(HKEY);

impl Key {
    fn open(path: &str, access: REG_SAM_FLAGS) -> io::Result<Self> {
        let path = wide(path.as_ref())?;
        let mut key = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                access | KEY_WOW64_64KEY,
                &mut key,
            )
        };
        check(status)?;
        Ok(Self(key))
    }

    fn create(path: &str) -> io::Result<Self> {
        let path = wide(path.as_ref())?;
        let mut key = HKEY::default();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | KEY_WOW64_64KEY,
                None,
                &mut key,
                None,
            )
        };
        check(status)?;
        Ok(Self(key))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn check(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status.0 as i32))
    }
}

fn write_values(path: &str, values: &[(&str, &OsStr)]) -> io::Result<()> {
    let key = Key::create(path)?;
    for (name, data) in values {
        let name = wide(name.as_ref())?;
        let data: Vec<u8> = wide(data)?.into_iter().flat_map(u16::to_le_bytes).collect();
        let status =
            unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&data)) };
        check(status)?;
    }
    Ok(())
}

fn read_value(path: &str, name: &str) -> io::Result<OsString> {
    let key = Key::open(path, KEY_QUERY_VALUE)?;
    let name = wide(name.as_ref())?;
    let mut size = 0;
    let value = PCWSTR(name.as_ptr());
    let status = unsafe {
        RegGetValueW(
            key.0,
            PCWSTR::null(),
            value,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    check(status)?;
    if size % 2 != 0 || size > 128 << 10 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "注册表字符串无效",
        ));
    }
    let mut buffer = vec![0u16; size as usize / 2];
    let status = unsafe {
        RegGetValueW(
            key.0,
            PCWSTR::null(),
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    check(status)?;
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    Ok(OsString::from_wide(&buffer[..end]))
}

fn delete_keys<'a>(paths: impl IntoIterator<Item = &'a str>) -> io::Result<()> {
    let mut first_error = None;
    for path in paths {
        let result = delete_key(path);
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn delete_key(path: &str) -> io::Result<()> {
    let (parent, name) = path
        .rsplit_once('\\')
        .expect("registry path includes parent");
    const DELETE: REG_SAM_FLAGS = REG_SAM_FLAGS(0x0001_0000);
    let key = match Key::open(parent, KEY_READ | KEY_WRITE | DELETE) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let name = wide(name.as_ref())?;
    let status = unsafe { RegDeleteTreeW(key.0, PCWSTR(name.as_ptr())) };
    if status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        check(status)
    }
}
