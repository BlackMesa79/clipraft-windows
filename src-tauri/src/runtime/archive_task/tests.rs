use super::*;
use crate::{
    archive::{bundle, control::Phase},
    model::{Clip, FormatData},
    storage::Storage,
};
const MASTER: [u8; 32] = [72; 32];
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "clipraft-export-flow-{}",
            crate::model::digest(&*crate::storage::crypto::random_key().unwrap())
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn assert_clean(&self) {
        assert!(fs::read_dir(&self.0).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".clipraft-")));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn clip() -> Clip {
    Clip {
        formats: vec![FormatData {
            name: "HTML Format".into(),
            bytes: vec![b'x'; 70_000],
        }],
    }
}

#[test]
fn full_export_releases_runtime_locks_and_restores_portable_history() {
    let _serial = snapshot_task::TEST_SERIAL.lock().unwrap();
    let fixture = Fixture::new();
    let state = crate::runtime::test_support::state(&fixture.0.join("source"));
    state.storage.lock().unwrap().save(&clip()).unwrap();
    state
        .storage
        .lock()
        .unwrap()
        .save_settings(&crate::settings::Settings {
            autostart: true,
            ..Default::default()
        })
        .unwrap();
    let destination = fixture.0.join("history.clipraft");
    let request = ExportRequest {
        temporary_parent: &fixture.0,
        destination: &destination,
        master: &MASTER,
        password: b"test-password",
    };
    let control = Control::default();
    let outcome = export_with_snapshot(&request, &control, |files| {
        snapshot_task::create_owned(&state, files, &MASTER, &control)?;
        assert!(!snapshot_task::active());
        assert!(state.storage.try_lock().is_ok());
        assert!(state.capture_context.try_lock().is_ok());
        assert_eq!(control.progress()?.phase, Phase::Snapshotting);
        Ok(())
    })
    .unwrap();
    assert!(outcome.cleanup_warning.is_none());
    assert_eq!(control.progress().unwrap().phase, Phase::Complete);
    fixture.assert_clean();
    let target = fixture.0.join("restored");
    bundle::restore(
        &mut fs::File::open(destination).unwrap(),
        &target,
        b"test-password",
    )
    .unwrap();
    let restored = Storage::open(&target, &MASTER).unwrap();
    assert_eq!(
        restored.load(&clip().hash()).unwrap().formats,
        clip().formats
    );
    assert!(!restored.load_settings().unwrap().autostart);
    assert!(
        state
            .storage
            .lock()
            .unwrap()
            .load_settings()
            .unwrap()
            .autostart
    );
}

#[test]
fn cancellation_between_snapshot_and_archive_cleans_snapshot_without_publication() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.0.join("source"), &MASTER).unwrap();
    let destination = fixture.0.join("history.clipraft");
    let request = ExportRequest {
        temporary_parent: &fixture.0,
        destination: &destination,
        master: &MASTER,
        password: b"test-password",
    };
    let control = Control::default();
    assert!(export_with_snapshot(&request, &control, |files| {
        control.snapshotting()?;
        storage.create_snapshot_owned(files, &MASTER, || Ok(()))?;
        control.cancel()?;
        Ok(())
    })
    .is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Cancelled);
    assert!(!destination.exists());
    fixture.assert_clean();
}

#[test]
fn failed_snapshot_cleans_only_created_files_and_keeps_source_writable() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.0.join("source"), &MASTER).unwrap();
    storage.save(&clip()).unwrap();
    let destination = fixture.0.join("history.clipraft");
    let request = ExportRequest {
        temporary_parent: &fixture.0,
        destination: &destination,
        master: &MASTER,
        password: b"test-password",
    };
    let control = Control::default();
    let count = std::cell::Cell::new(0);
    assert!(export_with_snapshot(&request, &control, |files| {
        control.snapshotting()?;
        storage.create_snapshot_owned(files, &MASTER, || {
            count.set(count.get() + 1);
            if count.get() == 6 {
                bail!("synthetic object copy failure");
            }
            Ok(())
        })
    })
    .is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Failed);
    assert!(!destination.exists());
    fixture.assert_clean();
    storage.save(&clip()).unwrap();
}

#[test]
fn invalid_requests_do_not_start_snapshot_or_create_staging() {
    let fixture = Fixture::new();
    let destination = fixture.0.join("history.clipraft");
    let request = ExportRequest {
        temporary_parent: &fixture.0,
        destination: &destination,
        master: &MASTER,
        password: b"",
    };
    assert!(
        export_with_snapshot(&request, &Control::default(), |_| panic!(
            "must not create snapshot"
        ))
        .is_err()
    );
    fs::write(&destination, b"existing").unwrap();
    let request = ExportRequest {
        password: b"test-password",
        ..request
    };
    assert!(
        export_with_snapshot(&request, &Control::default(), |_| panic!(
            "must not create snapshot"
        ))
        .is_err()
    );
    assert_eq!(fs::read(destination).unwrap(), b"existing");
    fixture.assert_clean();
}

#[test]
fn successful_archive_with_unknown_snapshot_content_returns_cleanup_warning() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.0.join("source"), &MASTER).unwrap();
    let destination = fixture.0.join("history.clipraft");
    let request = ExportRequest {
        temporary_parent: &fixture.0,
        destination: &destination,
        master: &MASTER,
        password: b"test-password",
    };
    let control = Control::default();
    let mut extra = PathBuf::new();
    let outcome = export_with_snapshot(&request, &control, |files| {
        control.snapshotting()?;
        storage.create_snapshot_owned(files, &MASTER, || Ok(()))?;
        extra = files.root().join("keep");
        fs::write(&extra, b"unrelated")?;
        Ok(())
    })
    .unwrap();
    assert!(outcome.cleanup_warning.is_some());
    assert_eq!(control.progress().unwrap().phase, Phase::Complete);
    assert!(destination.exists());
    assert_eq!(fs::read(extra).unwrap(), b"unrelated");
}

#[test]
fn destination_race_after_snapshot_cleans_owned_snapshot_and_keeps_destination() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.0.join("source"), &MASTER).unwrap();
    let destination = fixture.0.join("history.clipraft");
    let request = ExportRequest {
        temporary_parent: &fixture.0,
        destination: &destination,
        master: &MASTER,
        password: b"test-password",
    };
    let control = Control::default();
    assert!(export_with_snapshot(&request, &control, |files| {
        control.snapshotting()?;
        storage.create_snapshot_owned(files, &MASTER, || Ok(()))?;
        fs::write(&destination, b"racing destination")?;
        Ok(())
    })
    .is_err());
    assert_eq!(fs::read(destination).unwrap(), b"racing destination");
    assert_eq!(control.progress().unwrap().phase, Phase::Failed);
    fixture.assert_clean();
}

#[test]
fn failed_exclusive_snapshot_creation_never_owns_existing_directory() {
    let fixture = Fixture::new();
    let target = fixture.0.join("existing");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), b"keep").unwrap();
    let mut files = SnapshotFiles::new(&target);
    assert!(files.create().is_err());
    files.cleanup().unwrap();
    assert_eq!(fs::read(target.join("keep")).unwrap(), b"keep");
}
