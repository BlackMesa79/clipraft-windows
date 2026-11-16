use super::*;
use crate::{
    archive::{bundle, control::Phase},
    model::{Clip, FormatData},
    storage::Storage,
};
use std::io::Cursor;

const MASTER: [u8; 32] = [74; 32];
struct Fixture {
    root: PathBuf,
    archive: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "clipraft-restore-test-{}",
            crate::model::digest(&*crate::storage::crypto::random_key().unwrap())
        ));
        fs::create_dir(&root).unwrap();
        let mut source = Storage::open(&root.join("source"), &MASTER).unwrap();
        source.save(&Self::clip()).unwrap();
        source
            .save_settings(&crate::settings::Settings {
                autostart: true,
                max_items: 2468,
                ..Default::default()
            })
            .unwrap();
        let frozen = root.join("snapshot");
        source.create_snapshot(&frozen, &MASTER).unwrap();
        let archive = root.join("history.clipraft");
        bundle::export(
            &frozen,
            &MASTER,
            &mut fs::File::create(&archive).unwrap(),
            b"test-password",
        )
        .unwrap();
        Self { root, archive }
    }
    fn clip() -> Clip {
        Clip {
            formats: vec![FormatData {
                name: "HTML Format".into(),
                bytes: vec![b'x'; 70_000],
            }],
        }
    }
    fn assert_no_stage(&self) {
        assert!(fs::read_dir(&self.root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".clipraft-restore-")));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn restore_job_publishes_verified_portable_database_without_changing_source() {
    let fixture = Fixture::new();
    let destination = fixture.root.join("restored");
    let request = Request {
        archive: &fixture.archive,
        password: b"test-password",
        destination: &destination,
    };
    let control = Control::default();
    let outcome = run(&request, &control).unwrap();
    assert_eq!(
        fs::canonicalize(outcome.root).unwrap(),
        fs::canonicalize(&destination).unwrap()
    );
    assert!(outcome.cleanup_warning.is_none());
    assert_eq!(control.progress().unwrap().phase, Phase::Complete);
    assert_eq!(
        control.progress().unwrap().completed_bytes,
        control.progress().unwrap().total_bytes
    );
    assert!(!control.cancel().unwrap());
    let key = crate::platform::master_key(&destination).unwrap();
    assert_eq!(*key, MASTER);
    let restored = Storage::open(&destination, &key).unwrap();
    assert_eq!(
        restored.load(&Fixture::clip().hash()).unwrap().formats,
        Fixture::clip().formats
    );
    assert!(!restored.load_settings().unwrap().autostart);
    assert_eq!(restored.load_settings().unwrap().max_items, 2468);
    assert!(
        Storage::open(&fixture.root.join("source"), &MASTER)
            .unwrap()
            .load_settings()
            .unwrap()
            .autostart
    );
    fixture.assert_no_stage();
}

#[test]
fn wrong_password_capacity_failure_and_cancel_leave_no_target_or_stage() {
    let fixture = Fixture::new();
    let destination = fixture.root.join("restored");
    let mut request = Request {
        archive: &fixture.archive,
        password: b"wrong",
        destination: &destination,
    };
    assert!(run(&request, &Control::default()).is_err());
    request.password = b"test-password";
    let control = Control::default();
    assert!(run_with_capacity(&request, &control, |_, bytes| {
        assert!(bytes > 0);
        bail!("synthetic disk full")
    })
    .is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Failed);
    let control = Control::default();
    assert!(run_with_capacity(&request, &control, |_, _| {
        control.cancel()?;
        Ok(())
    })
    .is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Cancelled);
    assert!(!destination.exists());
    fixture.assert_no_stage();
}

#[test]
fn truncated_archive_removes_partial_restore_and_preserves_unrelated_files() {
    let fixture = Fixture::new();
    let bytes = fs::read(&fixture.archive).unwrap();
    fs::write(&fixture.archive, &bytes[..bytes.len() - 1]).unwrap();
    fs::write(fixture.root.join("keep"), b"keep").unwrap();
    let destination = fixture.root.join("restored");
    let request = Request {
        archive: &fixture.archive,
        password: b"test-password",
        destination: &destination,
    };
    assert!(run(&request, &Control::default()).is_err());
    assert!(!destination.exists());
    assert_eq!(fs::read(fixture.root.join("keep")).unwrap(), b"keep");
    fixture.assert_no_stage();
}

#[test]
fn existing_and_racing_target_directories_are_preserved() {
    let fixture = Fixture::new();
    let destination = fixture.root.join("restored");
    let request = Request {
        archive: &fixture.archive,
        password: b"test-password",
        destination: &destination,
    };
    assert!(run_with_capacity(&request, &Control::default(), |_, _| {
        fs::create_dir(&destination)?;
        fs::write(destination.join("keep"), b"keep")?;
        Ok(())
    })
    .is_err());
    assert!(run(&request, &Control::default()).is_err());
    assert_eq!(fs::read(destination.join("keep")).unwrap(), b"keep");
    assert!(!destination.join("history.db").exists());
    fixture.assert_no_stage();
}

struct CancelReader<'a> {
    input: Cursor<Vec<u8>>,
    control: &'a Control,
}
impl Read for CancelReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.input.read(buffer)?;
        if self.control.progress().unwrap().phase == Phase::Writing {
            self.control.cancel().unwrap();
        }
        Ok(count)
    }
}
#[test]
fn cancellation_during_content_read_cleans_created_files() {
    let fixture = Fixture::new();
    let control = Control::default();
    control.start().unwrap();
    let mut input = CancelReader {
        input: Cursor::new(fs::read(&fixture.archive).unwrap()),
        control: &control,
    };
    let mut plan = PreparedRestore::new(&mut input, b"test-password").unwrap();
    control.writing(plan.bytes()).unwrap();
    let target = fixture.root.join("partial");
    assert!(plan.extract(&mut input, &target, &control).is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Cancelled);
    assert!(!target.join("key.bin").exists());
    plan.cleanup(&target).unwrap();
    assert!(!target.exists());
}

#[test]
fn verification_accepts_cancel_and_cleanup_preserves_unknown_content() {
    let fixture = Fixture::new();
    let control = Control::default();
    control.start().unwrap();
    let mut input = fs::File::open(&fixture.archive).unwrap();
    let mut plan = PreparedRestore::new(&mut input, b"test-password").unwrap();
    control.writing(plan.bytes()).unwrap();
    let target = fixture.root.join("pending");
    plan.extract(&mut input, &target, &control).unwrap();
    assert_eq!(control.progress().unwrap().phase, Phase::Verifying);
    assert!(control.cancel().unwrap());
    assert!(control.publish().is_err());
    fs::write(target.join("keep"), b"keep").unwrap();
    assert!(plan.cleanup(&target).is_err());
    assert_eq!(fs::read(target.join("keep")).unwrap(), b"keep");
    assert!(!target.join("key.bin").exists());
}

#[test]
fn corrupt_authenticated_object_is_not_published_and_stage_is_cleaned() {
    let fixture = Fixture::new();
    let snapshot = fixture.root.join("snapshot");
    let object = fs::read_dir(snapshot.join("data/blobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(object, b"synthetic corrupted object").unwrap();
    bundle::export(
        &snapshot,
        &MASTER,
        &mut fs::File::create(&fixture.archive).unwrap(),
        b"test-password",
    )
    .unwrap();
    let destination = fixture.root.join("restored");
    let request = Request {
        archive: &fixture.archive,
        password: b"test-password",
        destination: &destination,
    };
    let control = Control::default();
    assert!(run(&request, &control).is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Failed);
    assert!(!destination.exists());
    fixture.assert_no_stage();
}
