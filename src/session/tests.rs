use std::io::{Cursor, Write};

use zip::{ZipWriter, write::SimpleFileOptions};

use super::*;

#[derive(Default)]
struct Recorder {
    events: Vec<Event>,
    password: Option<&'static str>,
}

impl Recorder {
    fn session<'a>(&'a mut self, cancel: &'a AtomicBool) -> Session<'a> {
        Session {
            observer: self,
            cancel,
            recent: None,
            stored: Zeroizing::new(Vec::new()),
            index: 0,
            next_layer: 1,
            archives: 0,
            depth: 0,
        }
    }
}

impl Observer for Recorder {
    fn event(&mut self, event: Event) {
        self.events.push(event);
    }

    fn password(&mut self, _: PasswordRequest, _: &AtomicBool) -> Option<PasswordReply> {
        self.password.take().map(|value| PasswordReply {
            value: value.to_owned(),
            remember: false,
        })
    }
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        archive
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        archive.write_all(data).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

#[test]
fn preserves_siblings_and_never_overwrites_an_existing_result() {
    let working = tempfile::tempdir().unwrap();
    let leaf = zip(&[("资料.txt", b"deep data")]);
    let middle = zip(&[("leaf.zip", &leaf)]);
    let epub = zip(&[("chapter.html", b"preserve this ZIP-based document")]);
    let source = working.path().join("合集.zip");
    fs::write(
        &source,
        zip(&[
            ("part/middle.zip", &middle),
            ("part/conflict.zip", &leaf),
            ("part/conflict/keep.txt", b"keep nested sibling"),
            ("book.epub", &epub),
        ]),
    )
    .unwrap();

    let occupied = working.path().join("合集");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("keep.txt"), b"keep").unwrap();
    let occupied_file = working.path().join("合集 (2)");
    fs::write(&occupied_file, b"keep this too").unwrap();
    let mut observer = Recorder::default();
    let cancel = AtomicBool::new(false);
    let mut session = observer.session(&cancel);

    let (output, files) = session.extract_source(&source).unwrap().unwrap();
    assert_eq!(output, working.path().join("合集 (3)"));
    assert_eq!(files, 4);
    assert_eq!((session.archives, session.depth), (4, 3));
    assert_eq!(
        fs::read(output.join("part/middle/资料.txt")).unwrap(),
        b"deep data"
    );
    assert_eq!(
        fs::read(output.join("part/conflict (2)/资料.txt")).unwrap(),
        b"deep data"
    );
    assert_eq!(
        fs::read(output.join("part/conflict/keep.txt")).unwrap(),
        b"keep nested sibling"
    );
    assert_eq!(fs::read(output.join("book.epub")).unwrap(), epub);
    assert_eq!(fs::read(occupied.join("keep.txt")).unwrap(), b"keep");
    assert_eq!(fs::read(occupied_file).unwrap(), b"keep this too");
    assert!(source.is_file());
    assert!(
        observer
            .events
            .iter()
            .any(|event| matches!(event, Event::Archive { node, .. } if
        node.name == "leaf.zip" && node.depth == 3 && node.status == Status::Success))
    );
}

#[test]
fn uses_the_archive_name_for_split_sevenz_results() {
    let working = tempfile::tempdir().unwrap();
    let parts: Vec<_> = include_bytes!("../../testdata/encrypted-inner.7z")
        .chunks(120)
        .enumerate()
        .map(|(index, data)| {
            let path = working.path().join(format!("资料.7z.{:03}", index + 1));
            fs::write(&path, data).unwrap();
            path
        })
        .collect();
    let mut observer = Recorder {
        password: Some("nestedunpack-regression-only"),
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);

    let (output, files) = observer
        .session(&cancel)
        .extract_source(&parts[1])
        .unwrap()
        .unwrap();
    assert_eq!(output, working.path().join("资料"));
    assert_eq!(files, 1);
    assert_eq!(
        fs::read(output.join("media/detail.txt")).unwrap(),
        b"nested media payload\n".repeat(600)
    );
}

#[cfg(windows)]
#[test]
fn publishes_shell_accessible_folder_names_on_windows() {
    let working = tempfile::tempdir().unwrap();
    for (source, expected) in [
        ("尾点..zip", "尾点"),
        ("尾空格 .zip", "尾空格"),
        ("..zip", "解压结果"),
        ("CON.data.zip", "_CON.data"),
    ] {
        let stage = tempfile::tempdir_in(working.path()).unwrap();
        let output = publish(stage.path(), &working.path().join(source), false).unwrap();
        assert_eq!(output, working.path().join(expected));
        assert!(output.is_dir());
    }
}
