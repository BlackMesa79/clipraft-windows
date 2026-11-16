use super::*;
use crate::{
    archive::staging::Directory,
    model::{Clip, FormatData},
    storage::Storage,
};
use std::sync::Mutex;
struct Fixture {
    root: PathBuf,
    current: PathBuf,
    restored: PathBuf,
    locations: Locations,
}
fn clip(text: &str) -> Clip {
    Clip {
        formats: vec![FormatData {
            name: "CF_UNICODETEXT".into(),
            bytes: text
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
        }],
    }
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "clipraft-inplace-test-{}",
            config::nonce().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let current = root.join("current");
        let master = crate::platform::master_key(&current).unwrap();
        let mut old = Storage::open(&current, &master).unwrap();
        old.save(&clip("old-only")).unwrap();
        drop(old);
        fs::write(current.join("notes.txt"), b"user note").unwrap();
        fs::write(current.join("data/user.txt"), b"nested user note").unwrap();
        fs::create_dir(current.join("webview")).unwrap();
        fs::write(current.join("webview/cache"), b"keep cache").unwrap();
        let profile = root.join("profile");
        fs::create_dir(&profile).unwrap();
        let locations = Locations::new(current.clone(), Some(profile)).unwrap();
        locations
            .save(&Configuration::initial(current.clone()))
            .unwrap();
        let source = root.join("incoming");
        let key = crate::platform::master_key(&source).unwrap();
        let mut incoming = Storage::open(&source, &key).unwrap();
        incoming.save(&clip("imported-only")).unwrap();
        let directory = Directory::create(&root, "restore").unwrap();
        let restored = directory.path().join("database");
        incoming.create_portable_snapshot(&restored, &key).unwrap();
        crate::platform::install_master_key(&restored, &key).unwrap();
        drop(directory);
        Self {
            root,
            current,
            restored,
            locations,
        }
    }
    fn control(&self) -> DirectoryControl {
        DirectoryControl {
            locations: self.locations.clone(),
            root: self.current.clone(),
            plan: Mutex::new(None),
        }
    }
    fn schedule(&self) -> Configuration {
        drop(self.control().schedule_replacement(&self.restored).unwrap());
        self.locations.read().unwrap()
    }
    fn assert_replaced(&self, backup: &Path) {
        let key = crate::platform::master_key(&self.current).unwrap();
        let store = Storage::open(&self.current, &key).unwrap();
        assert!(store.load(&clip("imported-only").hash()).is_ok());
        assert!(store.load(&clip("old-only").hash()).is_err());
        let key = crate::platform::master_key(backup).unwrap();
        let old = Storage::open(backup, &key).unwrap();
        assert!(old.load(&clip("old-only").hash()).is_ok());
        assert_eq!(
            fs::read(self.current.join("notes.txt")).unwrap(),
            b"user note"
        );
        assert_eq!(
            fs::read(self.current.join("data/user.txt")).unwrap(),
            b"nested user note"
        );
        assert_eq!(
            fs::read(self.current.join("webview/cache")).unwrap(),
            b"keep cache"
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn inplace_restore_keeps_current_directory_and_backs_up_latest_old_data() {
    let f = Fixture::new();
    let config = f.schedule();
    let task = config.replacement.as_ref().unwrap();
    assert_eq!(config.root, f.current);
    let session = super::super::Session::open_at(f.locations.clone()).unwrap();
    assert_eq!(session.root, f.current);
    assert_eq!(
        session.startup_capture,
        crate::capture_policy::StartupCapture::ChangesOnly
    );
    assert!(f.locations.read().unwrap().replacement.is_none());
    assert_eq!(
        f.locations.read().unwrap().previous.as_ref(),
        Some(&task.backup)
    );
    assert!(
        !f.restored.parent().unwrap().exists(),
        "completed temporary import removed"
    );
    f.assert_replaced(&task.backup);
}
#[test]
fn interruption_during_preservation_resumes_without_mixed_database() {
    let f = Fixture::new();
    let config = f.schedule();
    let task = config.replacement.as_ref().unwrap();
    let _journal = journal::load_or_create(task).unwrap();
    let entry = files::inventory(&f.current)
        .unwrap()
        .into_iter()
        .find(|entry| entry.path == Path::new("history.db"))
        .unwrap();
    files::ensure_parents(&task.backup, &entry.path).unwrap();
    config::move_new(&f.current.join(&entry.path), &task.backup.join(&entry.path)).unwrap();
    assert!(!f.current.join("history.db").exists());
    let session = super::super::Session::open_at(f.locations.clone()).unwrap();
    assert_eq!(session.root, f.current);
    f.assert_replaced(&task.backup);
}
#[test]
fn interruption_after_installation_before_commit_resumes_and_removes_stage() {
    let f = Fixture::new();
    let config = f.schedule();
    let task = config.replacement.as_ref().unwrap();
    let mut journal = journal::load_or_create(task).unwrap();
    journal.apply(task).unwrap();
    assert!(f.locations.read().unwrap().replacement.is_some());
    let committed = execute(&f.locations, &config).unwrap();
    assert!(committed.replacement.is_none());
    f.assert_replaced(&task.backup);
    assert!(!f.restored.exists());
}
#[test]
fn damaged_staged_database_stops_before_old_data_is_moved() {
    let f = Fixture::new();
    let config = f.schedule();
    let before = files::inventory(&f.current).unwrap();
    fs::write(f.restored.join("history.db"), b"bad staged db").unwrap();
    assert!(execute(&f.locations, &config).is_err());
    assert_eq!(files::inventory(&f.current).unwrap(), before);
    assert!(f.locations.read().unwrap().replacement.is_some());
}
#[test]
fn config_commit_failure_preserves_backup_and_can_retry() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let config = f.schedule();
    let task = config.replacement.as_ref().unwrap();
    let hold = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(f.locations.config.as_ref().unwrap())
        .unwrap();
    assert!(execute(&f.locations, &config).is_err());
    assert!(task.backup.join("history.db").exists());
    assert!(f.restored.join("history.db").exists());
    drop(hold);
    execute(&f.locations, &config).unwrap();
    f.assert_replaced(&task.backup);
}
#[test]
fn schedule_failure_or_pending_operation_does_not_mutate_current_data() {
    let f = Fixture::new();
    let before = files::inventory(&f.current).unwrap();
    assert!(f.control().schedule_replacement(&f.current).is_err());
    let _config = f.schedule();
    assert!(f.control().schedule_replacement(&f.restored).is_err());
    assert_eq!(files::inventory(&f.current).unwrap(), before);
}

#[test]
fn repeating_default_restore_never_changes_user_data_directory() {
    let f = Fixture::new();
    let first = f.schedule();
    let first_backup = first.replacement.as_ref().unwrap().backup.clone();
    execute(&f.locations, &first).unwrap();
    let source_key = crate::platform::master_key(&f.root.join("incoming")).unwrap();
    let mut incoming = Storage::open(&f.root.join("incoming"), &source_key).unwrap();
    incoming.save(&clip("second import")).unwrap();
    let directory = Directory::create(&f.root, "restore").unwrap();
    let second_root = directory.path().join("database");
    incoming
        .create_portable_snapshot(&second_root, &source_key)
        .unwrap();
    crate::platform::install_master_key(&second_root, &source_key).unwrap();
    drop(directory);
    drop(f.control().schedule_replacement(&second_root).unwrap());
    let config = f.locations.read().unwrap();
    execute(&f.locations, &config).unwrap();
    assert_eq!(f.locations.read().unwrap().root, f.current);
    assert!(first_backup.join("history.db").exists());
    let key = crate::platform::master_key(&f.current).unwrap();
    let current = Storage::open(&f.current, &key).unwrap();
    assert!(current.load(&clip("second import").hash()).is_ok());
    assert!(!second_root.parent().unwrap().exists());
}

#[test]
fn valid_but_modified_staged_database_after_confirmation_is_rejected() {
    let f = Fixture::new();
    let config = f.schedule();
    let master = crate::platform::master_key(&f.restored).unwrap();
    let mut changed = Storage::open(&f.restored, &master).unwrap();
    changed.save(&clip("not the confirmed import")).unwrap();
    drop(changed);
    let before = files::inventory(&f.current).unwrap();
    assert!(execute(&f.locations, &config).is_err());
    assert_eq!(files::inventory(&f.current).unwrap(), before);
}
