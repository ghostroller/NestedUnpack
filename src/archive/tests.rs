use std::collections::VecDeque;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

use zip::unstable::write::FileOptionsExt;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::{Control, ExtractError, Kind, detect, extract, extract_outer_rars, safe_path};

const PASSWORD: &str = "nestedunpack-regression-only";

#[derive(Default)]
struct TestControl {
    replies: VecDeque<&'static str>,
    prompts: Vec<bool>,
    accepted: Vec<String>,
    canceled: bool,
    cancel_after: Option<u64>,
}

impl Control for TestControl {
    fn canceled(&self) -> bool {
        self.canceled
    }

    fn progress(&mut self, done: u64, total: Option<u64>) {
        assert!(total.is_none_or(|size| done <= size));
        if self.cancel_after.is_some_and(|threshold| done >= threshold) {
            self.canceled = true;
        }
    }

    fn password(&mut self, retry: bool) -> Result<String, ExtractError> {
        self.prompts.push(retry);
        self.replies
            .pop_front()
            .map(str::to_owned)
            .ok_or(ExtractError::Skipped)
    }

    fn accepted(&mut self, password: &str) {
        self.accepted.push(password.to_owned());
    }
}

fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn append_zip64_directory(bytes: &mut Vec<u8>) {
    let position = bytes.len() - 22;
    let eocd = bytes.split_off(position);
    assert_eq!(&eocd[..4], b"PK\x05\x06");
    let count = u16::from_le_bytes(eocd[10..12].try_into().unwrap()) as u64;
    let size = u32::from_le_bytes(eocd[12..16].try_into().unwrap()) as u64;
    let offset = u32::from_le_bytes(eocd[16..20].try_into().unwrap()) as u64;
    bytes.extend_from_slice(b"PK\x06\x06");
    bytes.extend_from_slice(&44u64.to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&45u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 8]);
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&offset.to_le_bytes());
    bytes.extend_from_slice(b"PK\x06\x07");
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&(position as u64).to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    // Keep the real legacy count to exercise ZIP64 records written without
    // sentinel counts, as encountered in appended archives.
    bytes.extend_from_slice(&eocd);
}

#[test]
fn extracts_appended_zip64_with_trailing_data() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("资料.mp4");
    let dest = temp.path().join("out");
    let mut archive = make_zip(&[("media/资料.txt", b"bounded ZIP64")]);
    append_zip64_directory(&mut archive);
    let mut media = b"\0\0\0\x18ftypmp42".to_vec();
    media.extend(archive);
    media.resize(media.len() + (128 << 10), b'0');
    fs::write(&path, media).unwrap();

    assert_eq!(detect(&path, false).unwrap(), None);
    assert_eq!(detect(&path, true).unwrap(), Some(Kind::Zip));
    extract(Kind::Zip, &[path], &dest, &mut TestControl::default()).unwrap();
    assert_eq!(
        fs::read(dest.join("media/资料.txt")).unwrap(),
        b"bounded ZIP64"
    );
}

#[test]
fn extracts_appended_winzip_aes_and_retries() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("encrypted.mp4");
    let dest = temp.path().join("out");
    let mut media = b"\0\0\0\x18ftypmp42".to_vec();
    media.extend_from_slice(include_bytes!("../../testdata/aes256.zip"));
    fs::write(&path, media).unwrap();
    let mut control = TestControl {
        replies: VecDeque::from(["incorrect", PASSWORD]),
        ..Default::default()
    };

    assert_eq!(detect(&path, true).unwrap(), Some(Kind::Zip));
    extract(Kind::Zip, &[path], &dest, &mut control).unwrap();
    assert_eq!(control.prompts, [false, true]);
    assert_eq!(control.accepted, [PASSWORD]);
    assert_eq!(
        fs::read(dest.join("message.txt")).unwrap(),
        b"NestedUnpack AES regression fixture.\n".repeat(40)
    );
}

#[test]
fn extracts_legacy_zipcrypto_member() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("legacy.zip");
    let dest = temp.path().join("out");
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .with_deprecated_encryption(PASSWORD.as_bytes())
        .unwrap();
    writer.start_file("message.txt", options).unwrap();
    writer.write_all(b"legacy ZipCrypto").unwrap();
    fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();

    let mut control = TestControl {
        replies: VecDeque::from(["incorrect", PASSWORD]),
        ..Default::default()
    };
    extract(Kind::Zip, &[path], &dest, &mut control).unwrap();
    assert_eq!(control.prompts, [false, true]);
    assert_eq!(control.accepted, [PASSWORD]);
    assert_eq!(
        fs::read(dest.join("message.txt")).unwrap(),
        b"legacy ZipCrypto"
    );
}

#[test]
fn extracts_split_encrypted_sevenz_and_retries() {
    let temp = tempfile::tempdir().unwrap();
    let dest = temp.path().join("out");
    let parts: Vec<_> = include_bytes!("../../testdata/encrypted-inner.7z")
        .chunks(120)
        .enumerate()
        .map(|(index, data)| {
            let path = temp.path().join(format!("资料.7z.{:03}", index + 1));
            fs::write(&path, data).unwrap();
            path
        })
        .collect();
    let mut control = TestControl {
        replies: VecDeque::from(["incorrect", PASSWORD]),
        ..Default::default()
    };

    assert_eq!(detect(&parts[0], true).unwrap(), Some(Kind::SevenZip));
    extract(Kind::SevenZip, &parts, &dest, &mut control).unwrap();
    assert_eq!(control.prompts, [false, true]);
    assert_eq!(control.accepted, [PASSWORD]);

    let inner = dest.join("inner.zip");
    let nested_dest = temp.path().join("inner");
    extract(
        Kind::Zip,
        &[inner],
        &nested_dest,
        &mut TestControl::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(nested_dest.join("media/detail.txt")).unwrap(),
        b"nested media payload\n".repeat(600)
    );
}

#[test]
fn outer_rars_preserve_valid_member_paths() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("outer.zip");
    let dest = temp.path().join("out");
    let rar = include_bytes!("../../testdata/rar5-solid.rar");
    fs::write(
        &path,
        make_zip(&[
            ("files/document.txt", b"keep original"),
            ("files/inner.RAR", rar),
        ]),
    )
    .unwrap();
    extract_outer_rars(&path, &dest, &mut TestControl::default()).unwrap();
    assert_eq!(fs::read(dest.join("files/inner.RAR")).unwrap(), rar);
    assert!(!dest.join("files/document.txt").exists());
}

#[test]
fn zip_extraction_does_not_replace_a_file_and_removes_canceled_member() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("plain.zip");
    let dest = temp.path().join("out");
    fs::create_dir(&dest).unwrap();
    fs::write(&path, make_zip(&[("data.txt", b"archive content")])).unwrap();
    let output = dest.join("data.txt");
    fs::write(&output, b"existing content").unwrap();
    assert!(
        extract(
            Kind::Zip,
            &[path.clone()],
            &dest,
            &mut TestControl::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&output).unwrap(), b"existing content");
    fs::remove_file(&output).unwrap();

    let mut control = TestControl {
        cancel_after: Some(1),
        ..Default::default()
    };
    assert!(matches!(
        extract(Kind::Zip, &[path], &dest, &mut control),
        Err(ExtractError::Canceled)
    ));
    assert!(!output.exists());
}

#[test]
fn rejects_unsafe_member_names_consistently_across_platforms() {
    let root = Path::new("output");
    assert_eq!(
        safe_path(root, "资料\\目录/內容.txt").unwrap(),
        root.join("资料").join("目录").join("內容.txt")
    );
    for name in [
        "",
        "../outside",
        "/absolute",
        "C:/absolute",
        "D:drive.txt",
        "parent//member",
        "name.",
        "parent/NUL.txt",
        "LPT².out",
        "a:b.txt",
        "\\\\remote\\path",
    ] {
        assert!(safe_path(root, name).is_err(), "accepted {name:?}");
    }

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("invalid.zip");
    let dest = temp.path().join("out");
    fs::write(
        &path,
        make_zip(&[
            ("ordinary.txt", b"test"),
            ("parent/../invalid.txt", b"bad member"),
        ]),
    )
    .unwrap();
    assert!(matches!(
        extract(Kind::Zip, &[path], &dest, &mut TestControl::default()),
        Err(ExtractError::Invalid(_))
    ));
    assert!(!dest.exists());
}
