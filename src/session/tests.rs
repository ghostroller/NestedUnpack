use std::io::{Cursor, Write};

use zip::{ZipWriter, write::SimpleFileOptions};

use super::*;

#[derive(Default)]
struct Recorder(Vec<Event>);

impl Observer for Recorder {
    fn event(&mut self, event: Event) {
        self.0.push(event);
    }

    fn password(&mut self, _: PasswordRequest, _: &AtomicBool) -> Option<PasswordReply> {
        None
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
        zip(&[("part/middle.zip", &middle), ("book.epub", &epub)]),
    )
    .unwrap();

    let occupied = working.path().join("合集-解压");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("keep.txt"), b"keep").unwrap();
    let mut observer = Recorder::default();
    let cancel = AtomicBool::new(false);
    let mut session = Session {
        observer: &mut observer,
        cancel: &cancel,
        recent: None,
        stored: Zeroizing::new(Vec::new()),
        index: 0,
        next_layer: 1,
        archives: 0,
        depth: 0,
    };

    let (output, files) = session.extract_source(&source).unwrap().unwrap();
    assert_ne!(output, occupied);
    assert_eq!(files, 2);
    assert_eq!((session.archives, session.depth), (3, 3));
    assert_eq!(
        fs::read(output.join("part/middle-解压/资料.txt")).unwrap(),
        b"deep data"
    );
    assert_eq!(fs::read(output.join("book.epub")).unwrap(), epub);
    assert_eq!(fs::read(occupied.join("keep.txt")).unwrap(), b"keep");
    assert!(source.is_file());
    assert!(
        observer
            .0
            .iter()
            .any(|event| matches!(event, Event::Archive { node, .. } if
        node.name == "leaf.zip" && node.depth == 3 && node.status == Status::Success))
    );
}
