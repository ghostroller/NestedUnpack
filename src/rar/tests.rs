use std::collections::VecDeque;
use std::fs;

use super::{Control, ExtractError, extract};

#[derive(Default)]
struct FixtureControl {
    passwords: VecDeque<&'static str>,
    accepted: Vec<String>,
    attempts: Vec<bool>,
    cancel_after_data: bool,
    canceled: bool,
}

impl Control for FixtureControl {
    fn canceled(&self) -> bool {
        self.canceled
    }

    fn progress(&mut self, done: u64, total: Option<u64>) {
        assert!(total.is_none_or(|total| total >= done));
        self.canceled |= self.cancel_after_data && done > 0;
    }

    fn password(&mut self, retry: bool) -> Result<String, ExtractError> {
        self.attempts.push(retry);
        self.passwords
            .pop_front()
            .map(str::to_owned)
            .ok_or(ExtractError::Skipped)
    }

    fn accepted(&mut self, password: &str) {
        self.accepted.push(password.to_owned());
    }
}

#[test]
fn decodes_solid_and_encrypted_rar() {
    for (fixture, encrypted) in [
        ("rar3-solid.rar", false),
        ("rar5-solid.rar", false),
        ("rar5-hpsw.rar", true),
        ("rar5-psw.rar", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join(fixture);
        let source = dir.path().join("压缩包.rar");
        fs::copy(&fixture, &source).unwrap();
        let output = dir.path().join("out");
        fs::create_dir(&output).unwrap();
        let mut control = FixtureControl {
            passwords: VecDeque::from(["wrong", "password"]),
            ..Default::default()
        };
        extract(&source, &output, &mut control).unwrap_or_else(|err| panic!("{fixture:?}: {err}"));
        assert!(fs::read_dir(&output).unwrap().next().is_some());
        if encrypted {
            assert_eq!(control.attempts, [false, true]);
            assert_eq!(control.accepted, ["password"]);
        } else {
            assert!(control.attempts.is_empty());
            assert!(control.accepted.is_empty());
        }
    }
}

#[test]
fn rejects_rar_links_and_discards_canceled_output() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
    let output = tempfile::tempdir().unwrap();
    let err = extract(
        &root.join("rar5-hlink.rar"),
        output.path(),
        &mut FixtureControl::default(),
    );
    assert!(matches!(err, Err(ExtractError::Invalid(_))));
    assert!(fs::read_dir(output.path()).unwrap().next().is_none());
    let mut control = FixtureControl {
        cancel_after_data: true,
        ..Default::default()
    };
    let err = extract(&root.join("rar5-solid.rar"), output.path(), &mut control);
    assert!(matches!(err, Err(ExtractError::Canceled)));
    assert!(fs::read_dir(output.path()).unwrap().next().is_none());
}
