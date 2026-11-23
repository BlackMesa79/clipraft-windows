use super::*;

const MASTER: [u8; 32] = [93; 32];
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "clipraft-snapshot-{}-{}",
            std::process::id(),
            model::digest(&*crypto::random_key().unwrap())
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn source(&self) -> Storage {
        Storage::open(&self.0.join("source"), &MASTER).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn clip() -> Clip {
    Clip {
        formats: vec![model::FormatData {
            name: "HTML Format".into(),
            bytes: vec![b'x'; 70_000],
        }],
    }
}

fn bundle(fixture: &Fixture) -> Vec<u8> {
    let mut source = fixture.source();
    source.save(&clip()).unwrap();
    source
        .create_snapshot(&fixture.0.join("frozen"), &MASTER)
        .unwrap();
    let mut bytes = Vec::new();
    crate::archive::bundle::export(
        &fixture.0.join("frozen"),
        &MASTER,
        &mut bytes,
        b"test-only-password",
    )
    .unwrap();
    bytes
}

#[test]
fn password_bundle_restores_with_account_protected_key() {
    let fixture = Fixture::new();
    let bytes = bundle(&fixture);
    let target = fixture.0.join("restored");
    crate::archive::bundle::restore(
        &mut std::io::Cursor::new(bytes),
        &target,
        b"test-only-password",
    )
    .unwrap();
    let key = crate::platform::master_key(&target).unwrap();
    assert_eq!(*key, MASTER);
    assert_ne!(fs::read(target.join("key.bin")).unwrap(), MASTER);
    let restored = Storage::open(&target, &key).unwrap();
    assert_eq!(
        restored.load(&clip().hash()).unwrap().formats,
        clip().formats
    );
}

#[test]
fn bundle_rejects_wrong_password_existing_target_and_truncation() {
    let fixture = Fixture::new();
    let bytes = bundle(&fixture);
    let target = fixture.0.join("wrong-password");
    assert!(
        crate::archive::bundle::restore(&mut std::io::Cursor::new(&bytes), &target, b"wrong")
            .is_err()
    );
    assert!(!target.exists());
    let existing = fixture.0.join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), b"keep").unwrap();
    assert!(crate::archive::bundle::restore(
        &mut std::io::Cursor::new(&bytes),
        &existing,
        b"test-only-password"
    )
    .is_err());
    assert_eq!(fs::read(existing.join("keep")).unwrap(), b"keep");
    let truncated = fixture.0.join("truncated");
    assert!(crate::archive::bundle::restore(
        &mut std::io::Cursor::new(&bytes[..bytes.len() - 1]),
        &truncated,
        b"test-only-password"
    )
    .is_err());
    assert!(!truncated.join("key.bin").exists());
}

#[test]
fn authenticated_bundle_with_corrupt_object_cannot_publish_key() {
    let fixture = Fixture::new();
    let _ = bundle(&fixture);
    let frozen = fixture.0.join("frozen");
    let object = fs::read_dir(frozen.join("data/blobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(object, b"synthetic corrupted object").unwrap();
    let mut bytes = Vec::new();
    crate::archive::bundle::export(&frozen, &MASTER, &mut bytes, b"test-only-password").unwrap();
    let target = fixture.0.join("corrupt");
    assert!(crate::archive::bundle::restore(
        &mut std::io::Cursor::new(bytes),
        &target,
        b"test-only-password"
    )
    .is_err());
    assert!(!target.join("key.bin").exists());
}

#[test]
fn snapshot_survives_source_deletion_and_preserves_schema() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    let clip = clip();
    source.save(&clip).unwrap();
    let settings = crate::settings::Settings {
        max_items: 2345,
        ..Default::default()
    };
    source.save_settings(&settings).unwrap();
    fs::write(source.blobs.join("unreferenced"), b"not included").unwrap();
    source
        .conn
        .execute("INSERT INTO groups VALUES (1,'快照分组',0,0)", [])
        .unwrap();
    source
        .conn
        .execute("UPDATE items SET favorite=1, group_id=1", [])
        .unwrap();
    let target = fixture.0.join("snapshot");
    source.create_snapshot(&target, &MASTER).unwrap();
    source.delete(&[clip.hash()]).unwrap();
    let snapshot = Storage::open(&target, &MASTER).unwrap();
    assert_eq!(snapshot.load(&clip.hash()).unwrap().formats, clip.formats);
    assert_eq!(
        snapshot
            .conn
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        migrations::VERSION
    );
    let item = &snapshot.list().unwrap()[0];
    assert!(item.favorite);
    assert_eq!(item.group_id, Some(1));
    assert!(!fs::read(target.join("history.db"))
        .unwrap()
        .starts_with(b"SQLite format 3"));
    assert!(!target.join("key.bin").exists());
    assert!(!target.join("data/blobs/unreferenced").exists());
    assert_eq!(snapshot.load_settings().unwrap(), settings);
}

#[test]
fn snapshot_rejects_existing_target_and_wrong_key() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    let target = fixture.0.join("snapshot");
    assert!(source.create_snapshot(&target, &[0; 32]).is_err());
    assert!(!target.exists());
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), b"unchanged").unwrap();
    assert!(source.create_snapshot(&target, &MASTER).is_err());
    assert_eq!(fs::read(target.join("keep")).unwrap(), b"unchanged");
}

#[test]
fn missing_object_fails_without_poisoning_source_connection() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    source.save(&clip()).unwrap();
    let hash: String = source
        .conn
        .query_row("SELECT blob FROM formats", [], |row| row.get(0))
        .unwrap();
    let object = source.blobs.join(hash);
    let bytes = fs::read(&object).unwrap();
    fs::remove_file(&object).unwrap();
    assert!(source
        .create_snapshot(&fixture.0.join("failed"), &MASTER)
        .is_err());
    fs::write(&object, bytes).unwrap();
    source
        .create_snapshot(&fixture.0.join("retry"), &MASTER)
        .unwrap();
    assert_eq!(source.list().unwrap().len(), 1);
}

#[test]
fn malformed_object_reference_cannot_escape_snapshot() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    source.save(&clip()).unwrap();
    source
        .conn
        .execute("UPDATE formats SET blob='../outside'", [])
        .unwrap();
    let target = fixture.0.join("snapshot");
    assert!(source
        .create_snapshot(&target, &MASTER)
        .unwrap_err()
        .to_string()
        .contains("非法对象引用"));
    assert!(!target.join("outside").exists());
}

#[test]
fn portable_snapshot_filters_settings_without_changing_source_or_raw_backup() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    let settings = crate::settings::Settings {
        autostart: true,
        auto_check_updates: true,
        pause_on_start: true,
        theme: crate::settings::Theme::System,
        backdrop_strength: 37,
        max_items: 2345,
        main_hotkey: crate::settings::Hotkey::CtrlBacktick,
        ..Default::default()
    };
    source.save_settings(&settings).unwrap();
    source
        .conn
        .execute(
            "INSERT INTO settings VALUES('future_machine_setting','true')",
            [],
        )
        .unwrap();
    let before = source.test_settings_rows();
    let raw = fixture.0.join("raw");
    source.create_snapshot(&raw, &MASTER).unwrap();
    let portable = fixture.0.join("portable");
    source.create_portable_snapshot(&portable, &MASTER).unwrap();
    assert_eq!(source.test_settings_rows(), before);
    assert_eq!(
        Storage::open(&raw, &MASTER).unwrap().test_settings_rows(),
        before
    );
    let copied = Storage::open(&portable, &MASTER).unwrap();
    let mut expected = settings;
    expected.autostart = false;
    assert_eq!(copied.load_settings().unwrap(), expected);
    let mut expected_rows = before;
    expected_rows.remove("autostart");
    expected_rows.remove("future_machine_setting");
    assert_eq!(copied.test_settings_rows(), expected_rows);
    assert!(!fs::read(portable.join("history.db"))
        .unwrap()
        .starts_with(b"SQLite format 3"));
}

#[test]
fn restore_filters_legacy_settings_before_installing_key() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    let settings = crate::settings::Settings {
        autostart: true,
        sensitive_ttl_minutes: 0,
        ..Default::default()
    };
    source.save_settings(&settings).unwrap();
    source
        .conn
        .execute("DELETE FROM settings WHERE key='skin'", [])
        .unwrap();
    source.save(&clip()).unwrap();
    source
        .conn
        .execute("UPDATE items SET sensitive=1, expires_at=4102444800000", [])
        .unwrap();
    source
        .conn
        .execute("INSERT INTO settings VALUES('future','true')", [])
        .unwrap();
    let frozen = fixture.0.join("legacy");
    source.create_snapshot(&frozen, &MASTER).unwrap();
    let mut bytes = Vec::new();
    crate::archive::bundle::export(&frozen, &MASTER, &mut bytes, b"test-only-password").unwrap();
    let target = fixture.0.join("restored");
    crate::archive::bundle::restore(
        &mut std::io::Cursor::new(bytes),
        &target,
        b"test-only-password",
    )
    .unwrap();
    let restored = Storage::open(&target, &MASTER).unwrap();
    assert!(!restored.load_settings().unwrap().autostart);
    assert_eq!(
        restored.load_settings().unwrap().skin,
        crate::settings::Skin::Clipraft
    );
    assert!(!restored.test_settings_rows().contains_key("future"));
    let expires: i64 = restored
        .conn
        .query_row("SELECT expires_at FROM items", [], |row| row.get(0))
        .unwrap();
    assert_eq!(expires, 4102444800000);
    assert_eq!(*crate::platform::master_key(&target).unwrap(), MASTER);
    assert!(source.load_settings().unwrap().autostart);
}

#[test]
fn restore_settings_filter_failure_does_not_install_key() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    source
        .save_settings(&crate::settings::Settings {
            autostart: true,
            ..Default::default()
        })
        .unwrap();
    source.conn.execute_batch("CREATE TRIGGER reject_portable_filter BEFORE DELETE ON settings BEGIN SELECT RAISE(ABORT,'synthetic settings failure'); END;").unwrap();
    let frozen = fixture.0.join("frozen");
    source.create_snapshot(&frozen, &MASTER).unwrap();
    let mut bytes = Vec::new();
    crate::archive::bundle::export(&frozen, &MASTER, &mut bytes, b"test-only-password").unwrap();
    let target = fixture.0.join("failed");
    assert!(crate::archive::bundle::restore(
        &mut std::io::Cursor::new(bytes),
        &target,
        b"test-only-password"
    )
    .is_err());
    assert!(!target.join("key.bin").exists());
    assert!(source.load_settings().unwrap().autostart);
    assert!(
        Storage::open(&target, &MASTER)
            .unwrap()
            .load_settings()
            .unwrap()
            .autostart
    );
}

#[test]
fn atomic_export_job_roundtrips_portable_snapshot() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    source.save(&clip()).unwrap();
    source
        .save_settings(&crate::settings::Settings {
            autostart: true,
            max_items: 2468,
            ..Default::default()
        })
        .unwrap();
    let frozen = fixture.0.join("portable");
    source.create_portable_snapshot(&frozen, &MASTER).unwrap();
    let destination = fixture.0.join("history.clipraft");
    let control = crate::archive::control::Control::default();
    crate::archive::export_job::run(
        &crate::archive::export_job::Request {
            snapshot: &frozen,
            master: &MASTER,
            password: b"test-password",
            destination: &destination,
        },
        &control,
    )
    .unwrap();
    let target = fixture.0.join("restored");
    crate::archive::bundle::restore(
        &mut fs::File::open(destination).unwrap(),
        &target,
        b"test-password",
    )
    .unwrap();
    let restored = Storage::open(&target, &crate::platform::master_key(&target).unwrap()).unwrap();
    assert_eq!(
        restored.load(&clip().hash()).unwrap().formats,
        clip().formats
    );
    let settings = restored.load_settings().unwrap();
    assert!(!settings.autostart);
    assert_eq!(settings.max_items, 2468);
    assert!(source.load_settings().unwrap().autostart);
}

#[test]
fn snapshot_verification_checkpoint_can_stop_between_objects() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    source.save(&clip()).unwrap();
    let target = fixture.0.join("snapshot");
    source.create_snapshot(&target, &MASTER).unwrap();
    let checks = std::cell::Cell::new(0);
    let result = Storage::verify_snapshot_with_check(&target, &MASTER, || {
        checks.set(checks.get() + 1);
        if checks.get() == 2 {
            anyhow::bail!("synthetic verification cancellation");
        }
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(checks.get(), 2);
    assert!(!target.join("key.bin").exists());
    Storage::verify_snapshot(&target, &MASTER).unwrap();
}

#[test]
fn snapshot_cancel_after_sql_export_detaches_and_source_remains_writable() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    source.save(&clip()).unwrap();
    let calls = std::cell::Cell::new(0);
    let target = fixture.0.join("cancelled");
    assert!(source
        .create_snapshot_checked(&target, &MASTER, || {
            calls.set(calls.get() + 1);
            if calls.get() == 3 {
                anyhow::bail!("synthetic cancellation after SQL export");
            }
            Ok(())
        })
        .is_err());
    source.save(&clip()).unwrap();
    source
        .create_snapshot(&fixture.0.join("retry"), &MASTER)
        .unwrap();
    assert_eq!(source.load(&clip().hash()).unwrap().formats, clip().formats);
}

#[test]
fn snapshot_cancel_during_object_copy_detaches_and_can_retry() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    source.save(&clip()).unwrap();
    let target = fixture.0.join("partial");
    let calls = std::cell::Cell::new(0);
    assert!(source
        .create_snapshot_checked(&target, &MASTER, || {
            calls.set(calls.get() + 1);
            if calls.get() == 6 {
                anyhow::bail!("synthetic object cancellation");
            }
            Ok(())
        })
        .is_err());
    source
        .create_snapshot(&fixture.0.join("retry"), &MASTER)
        .unwrap();
    assert!(!target.join("key.bin").exists());
    assert!(fs::read_dir(target.join("data/blobs"))
        .unwrap()
        .any(|entry| entry.unwrap().metadata().unwrap().len() > 0));
}

#[test]
fn snapshot_estimate_excludes_orphans_and_honors_cancel_and_missing_objects() {
    let fixture = Fixture::new();
    let mut source = fixture.source();
    source.save(&clip()).unwrap();
    let bytes = source.snapshot_bytes(|| Ok(())).unwrap();
    assert!(bytes > 0);
    fs::write(source.blobs.join("orphan"), vec![0; 100_000]).unwrap();
    assert_eq!(source.snapshot_bytes(|| Ok(())).unwrap(), bytes);
    assert!(source
        .snapshot_bytes(|| anyhow::bail!("synthetic estimate cancellation"))
        .is_err());
    let hash = source
        .conn
        .query_row(
            "SELECT blob FROM formats WHERE blob IS NOT NULL LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    fs::remove_file(source.blobs.join(hash)).unwrap();
    assert!(source.snapshot_bytes(|| Ok(())).is_err());
}
