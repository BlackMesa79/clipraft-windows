use super::*;
use crate::archive::control::Phase;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "clipraft-export-test-{}",
            crate::model::digest(&*crate::storage::crypto::random_key().unwrap())
        ));
        fs::create_dir_all(root.join("snapshot/data/blobs")).unwrap();
        fs::write(
            root.join("snapshot/history.db"),
            vec![83; 2 * super::super::CHUNK_SIZE + 3],
        )
        .unwrap();
        Self(root)
    }
    fn request(&self) -> (PathBuf, PathBuf) {
        (self.0.join("snapshot"), self.0.join("history.clipraft"))
    }
    fn assert_no_stage(&self) {
        assert!(fs::read_dir(&self.0).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".clipraft-export-")));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn export_publishes_exact_size_and_preserves_existing_destination() {
    let fixture = Fixture::new();
    let (snapshot, destination) = fixture.request();
    let request = Request {
        snapshot: &snapshot,
        destination: &destination,
        master: &[42; 32],
        password: b"test-password",
    };
    let control = Control::default();
    let outcome = run(&request, &control).unwrap();
    assert!(outcome.cleanup_warning.is_none());
    let progress = control.progress().unwrap();
    assert_eq!(progress.phase, Phase::Complete);
    assert_eq!(progress.completed_bytes, progress.total_bytes);
    assert_eq!(
        fs::metadata(&destination).unwrap().len(),
        progress.total_bytes
    );
    assert!(!control.cancel().unwrap());
    let before = fs::read(&destination).unwrap();
    assert!(run(&request, &Control::default()).is_err());
    assert_eq!(fs::read(&destination).unwrap(), before);
    assert!(run(&request, &control).is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Complete);
    fixture.assert_no_stage();
}

#[test]
fn cancellation_and_capacity_failure_never_create_output() {
    let fixture = Fixture::new();
    let (snapshot, destination) = fixture.request();
    let request = Request {
        snapshot: &snapshot,
        destination: &destination,
        master: &[42; 32],
        password: b"test-password",
    };
    let control = Control::default();
    assert!(control.cancel().unwrap());
    assert!(run(&request, &control).is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Cancelled);
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
fn write_failure_cleans_owned_partial_and_keeps_unrelated_file() {
    let fixture = Fixture::new();
    let (snapshot, destination) = fixture.request();
    fs::write(fixture.0.join("keep"), b"keep").unwrap();
    let request = Request {
        snapshot: &snapshot,
        destination: &destination,
        master: &[42; 32],
        password: b"",
    };
    let control = Control::default();
    assert!(run(&request, &control).is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Failed);
    assert!(!destination.exists());
    assert_eq!(fs::read(fixture.0.join("keep")).unwrap(), b"keep");
    fixture.assert_no_stage();
}

#[test]
fn cancelled_writer_stops_and_publish_boundary_rejects_late_cancel() {
    let fixture = Fixture::new();
    let control = Control::default();
    control.start().unwrap();
    control.writing(10).unwrap();
    let mut stage = Stage::create(&fixture.0).unwrap();
    let mut output = ProgressWriter {
        file: stage.output.as_mut().unwrap(),
        control: &control,
    };
    output.write_all(b"abc").unwrap();
    assert!(control.cancel().unwrap());
    assert!(output.write_all(b"def").is_err());
    assert!(control.publish().is_err());
    assert_eq!(control.progress().unwrap().completed_bytes, 3);
    stage.cleanup().unwrap();
    let control = Control::default();
    control.start().unwrap();
    control.writing(0).unwrap();
    control.publish().unwrap();
    assert!(!control.cancel().unwrap());
    control.finish().unwrap();
    fixture.assert_no_stage();
}

#[test]
fn destination_created_after_preflight_is_never_replaced() {
    let fixture = Fixture::new();
    let (snapshot, destination) = fixture.request();
    let request = Request {
        snapshot: &snapshot,
        destination: &destination,
        master: &[42; 32],
        password: b"test-password",
    };
    let control = Control::default();
    assert!(run_with_capacity(&request, &control, |_, _| {
        fs::write(&destination, b"racing destination")?;
        Ok(())
    })
    .is_err());
    assert_eq!(fs::read(destination).unwrap(), b"racing destination");
    assert_eq!(control.progress().unwrap().phase, Phase::Failed);
    fixture.assert_no_stage();
}

#[test]
fn source_changed_after_inventory_is_not_published() {
    let fixture = Fixture::new();
    let (snapshot, destination) = fixture.request();
    let request = Request {
        snapshot: &snapshot,
        destination: &destination,
        master: &[42; 32],
        password: b"test-password",
    };
    let control = Control::default();
    assert!(run_with_capacity(&request, &control, |_, _| {
        fs::write(snapshot.join("history.db"), b"changed")?;
        Ok(())
    })
    .is_err());
    assert!(!destination.exists());
    fixture.assert_no_stage();
}

#[test]
fn cleanup_preserves_unexpected_files_and_does_not_delete_recreated_output() {
    let fixture = Fixture::new();
    let mut stage = Stage::create(&fixture.0).unwrap();
    let root = stage.root.clone();
    stage.output.take();
    fs::remove_file(stage.file()).unwrap();
    stage.published = true;
    fs::write(stage.file(), b"not owned by completed task").unwrap();
    assert!(stage.cleanup().is_err());
    assert_eq!(
        fs::read(root.join("archive.part")).unwrap(),
        b"not owned by completed task"
    );
}

#[test]
fn staging_root_is_locked_and_existing_partial_is_not_owned() {
    let fixture = Fixture::new();
    let stage = Stage::create(&fixture.0).unwrap();
    assert!(fs::rename(&stage.root, fixture.0.join("moved-stage")).is_err());
    stage.cleanup().unwrap();
    let existing = fixture.0.join("foreign-stage");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("archive.part"), b"foreign").unwrap();
    assert!(Stage::open(existing.clone()).is_err());
    assert_eq!(fs::read(existing.join("archive.part")).unwrap(), b"foreign");
}
