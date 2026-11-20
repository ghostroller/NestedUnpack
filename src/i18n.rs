use std::{
    fmt::{self, Display, Write as _},
    fs,
    io::{self, Write as _},
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

macro_rules! catalog {
    ($name:ident { $($(#[$attr:meta])* $key:ident => ($zh:literal, $en:literal)),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub(crate) enum $name {
            $($(#[$attr])* $key),*
        }

        impl From<$name> for $crate::i18n::Text {
            fn from(key: $name) -> Self {
                match key {
                    $($(#[$attr])* $name::$key => Self { zh: $zh, en: $en }),*
                }
            }
        }

        impl From<$name> for $crate::i18n::Message {
            fn from(key: $name) -> Self {
                $crate::i18n::Text::from(key).into()
            }
        }
    };
}

mod app_text;
mod engine_text;
mod ui_text;

pub(crate) use app_text::AppKey;
pub(crate) use engine_text::EngineKey;
pub(crate) use ui_text::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Language {
    ZhCn,
    En,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum Preference {
    System,
    ZhCn,
    En,
}

impl Preference {
    fn code(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::ZhCn => "zh-CN",
            Self::En => "en",
        }
    }

    fn read(text: &str) -> Self {
        match text.trim() {
            "zh-CN" => Self::ZhCn,
            "en" => Self::En,
            _ => Self::System,
        }
    }
}

fn preference_state() -> &'static AtomicU8 {
    static PREFERENCE: OnceLock<AtomicU8> = OnceLock::new();
    PREFERENCE.get_or_init(|| {
        let preference = crate::platform::config_dir()
            .and_then(|directory| fs::read_to_string(directory.join("language")).ok())
            .map_or(Preference::System, |value| Preference::read(&value));
        AtomicU8::new(preference as u8)
    })
}

pub(crate) fn preference() -> Preference {
    match preference_state().load(Ordering::Relaxed) {
        value if value == Preference::ZhCn as u8 => Preference::ZhCn,
        value if value == Preference::En as u8 => Preference::En,
        _ => Preference::System,
    }
}

pub(crate) fn set_preference(preference: Preference) -> io::Result<()> {
    let directory = crate::platform::config_dir()
        .ok_or_else(|| Message::from(AppKey::NoConfigDirectory).io(io::ErrorKind::NotFound))?;
    write_preference(&directory, preference)?;
    preference_state().store(preference as u8, Ordering::Relaxed);
    Ok(())
}

fn write_preference(directory: &Path, preference: Preference) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    writeln!(file, "{}", preference.code())?;
    file.as_file().sync_all()?;
    file.persist(directory.join("language"))
        .map_err(|error| error.error)?;
    Ok(())
}

pub(crate) fn current() -> Language {
    static SYSTEM_LANGUAGE: OnceLock<Language> = OnceLock::new();
    match preference() {
        Preference::System => *SYSTEM_LANGUAGE.get_or_init(system_language),
        Preference::ZhCn => Language::ZhCn,
        Preference::En => Language::En,
    }
}

#[cfg(windows)]
fn system_language() -> Language {
    let locale = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    if locale & 0x03ff == 0x0004 {
        Language::ZhCn
    } else {
        Language::En
    }
}

#[cfg(not(windows))]
fn system_language() -> Language {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()));
    locale.as_deref().map_or(Language::En, language_from_locale)
}

#[cfg(any(not(windows), test))]
fn language_from_locale(locale: &str) -> Language {
    let language = locale
        .split(['-', '_', '.', '@'])
        .next()
        .unwrap_or_default();
    if language.eq_ignore_ascii_case("zh") {
        Language::ZhCn
    } else {
        Language::En
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Text {
    zh: &'static str,
    en: &'static str,
}

impl Language {
    pub(crate) fn text(self, key: impl Into<Text>) -> &'static str {
        let Text { zh, en } = key.into();
        match self {
            Self::ZhCn => zh,
            Self::En => en,
        }
    }
}

pub(crate) fn tr(key: impl Into<Text>) -> &'static str {
    current().text(key)
}

pub(crate) fn format(key: impl Into<Text>, arguments: &[&dyn Display]) -> String {
    let template = tr(key);
    let mut output = String::with_capacity(template.len());
    append_template(template, &mut output, |index, output| {
        arguments
            .get(index)
            .is_some_and(|argument| write!(output, "{argument}").is_ok())
    });
    output
}

fn append_template(
    mut template: &str,
    output: &mut String,
    mut append_argument: impl FnMut(usize, &mut String) -> bool,
) {
    while let Some((prefix, tail)) = template.split_once('{') {
        output.push_str(prefix);
        let Some((slot, remaining)) = tail.split_once('}') else {
            output.push('{');
            output.push_str(tail);
            return;
        };
        if !slot
            .parse()
            .is_ok_and(|index| append_argument(index, output))
        {
            output.push('{');
            output.push_str(slot);
            output.push('}');
        }
        template = remaining;
    }
    output.push_str(template);
}

#[derive(Clone, Debug)]
pub(crate) enum Message {
    Localized(Text, Vec<Self>),
    Raw(String),
}

impl Message {
    pub(crate) fn raw(value: impl Into<String>) -> Self {
        Self::Raw(value.into())
    }

    pub(crate) fn arg(mut self, argument: impl Into<Self>) -> Self {
        let Self::Localized(_, arguments) = &mut self else {
            unreachable!("Message arguments require a template");
        };
        arguments.push(argument.into());
        self
    }

    pub(crate) fn render(&self, language: Language) -> String {
        let mut output = String::new();
        self.append(language, &mut output);
        output
    }

    fn append(&self, language: Language, output: &mut String) {
        match self {
            Self::Raw(value) => output.push_str(value),
            Self::Localized(key, arguments) => {
                append_template(language.text(*key), output, |index, output| {
                    if let Some(argument) = arguments.get(index) {
                        argument.append(language, output);
                        true
                    } else {
                        false
                    }
                });
            }
        }
    }

    pub(crate) fn io(self, kind: io::ErrorKind) -> io::Error {
        io::Error::new(kind, self)
    }

    pub(crate) fn from_error(error: io::Error) -> Self {
        error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Self>())
            .cloned()
            .unwrap_or_else(|| Self::raw(error.to_string()))
    }
}

impl From<Text> for Message {
    fn from(key: Text) -> Self {
        Self::Localized(key, Vec::new())
    }
}

impl From<String> for Message {
    fn from(value: String) -> Self {
        Self::Raw(value)
    }
}

impl From<&str> for Message {
    fn from(value: &str) -> Self {
        Self::raw(value)
    }
}

macro_rules! number_arguments {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for Message {
                fn from(value: $ty) -> Self {
                    Self::raw(value.to_string())
                }
            }
        )*
    };
}

number_arguments!(usize, u64, u32, u16);

impl Display for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render(current()))
    }
}

impl std::error::Error for Message {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_preference_and_matches_chinese_locale_variants() {
        let test_dir = tempfile::tempdir().unwrap();
        for choice in [Preference::ZhCn, Preference::En, Preference::System] {
            write_preference(test_dir.path(), choice).unwrap();
            let contents = fs::read_to_string(test_dir.path().join("language")).unwrap();
            assert_eq!(Preference::read(&contents), choice);
        }
        assert_eq!(Preference::read("unrecognized"), Preference::System);
        for locale in ["zh_CN.UTF-8", "zh-HK", "ZH", "zh_TW"] {
            assert_eq!(language_from_locale(locale), Language::ZhCn);
        }
        for locale in ["en-US", "C.UTF-8", "fr-FR", ""] {
            assert_eq!(language_from_locale(locale), Language::En);
        }
    }

    #[test]
    fn localized_errors_keep_message_ids_and_do_not_interpret_path_braces() {
        let error = Message::from(AppKey::OpenOutputFailed)
            .arg(Message::from(AppKey::OutputNotDirectory))
            .io(io::ErrorKind::NotADirectory);
        let message = Message::from_error(error);
        assert_eq!(
            message.render(Language::En),
            "Couldn't open the output folder. The output path is not a folder."
        );
        assert_eq!(
            message.render(Language::ZhCn),
            "无法打开解压目录。输出路径不是文件夹"
        );
        let message = Message::from(AppKey::OutputAt).arg("资料/{0}.zip");
        assert_eq!(message.render(Language::En), "Output: 资料/{0}.zip");
        assert_eq!(message.render(Language::ZhCn), "已解压到 资料/{0}.zip");
    }
}
