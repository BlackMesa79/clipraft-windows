use super::*;
use crate::archive::staging_owner::NAME;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "clipraft-abandoned-test-{}",
            crate::model::digest(&*crate::storage::crypto::random_key().unwrap())
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn stage(&self, kind: &str) -> Directory {
        Directory::create(&self.0, kind).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn active_task_is_preserved_and_released_empty_task_can_be_cleaned() {
    let fixture = Fixture::new();
    let stage = fixture.stage("snapshot");
    let root = stage.path().to_path_buf();
    let report = scan(&fixture.0).unwrap();
    assert_eq!(report.candidates[0].status, Status::Active);
    assert!(cleanup_empty(&root).is_err());
    assert!(root.join(NAME).exists());
    drop(stage);
    assert_eq!(
        scan(&fixture.0).unwrap().candidates[0].status,
        Status::Empty
    );
    cleanup_empty(&root).unwrap();
    assert!(!root.exists());
}

#[test]
fn interrupted_content_and_unknown_extra_files_are_retained() {
    let fixture = Fixture::new();
    for kind in ["snapshot", "export", "restore"] {
        let stage = fixture.stage(kind);
        let root = stage.path().to_path_buf();
        fs::write(root.join("keep"), b"keep").unwrap();
        assert!(stage.cleanup().is_err());
        assert!(root.join(NAME).exists());
        assert!(cleanup_empty(&root).is_err());
        assert_eq!(fs::read(root.join("keep")).unwrap(), b"keep");
    }
    assert!(scan(&fixture.0)
        .unwrap()
        .candidates
        .iter()
        .all(|candidate| candidate.status == Status::Retained));
}

#[test]
fn legacy_missing_corrupt_oversized_and_relocated_markers_are_not_owned() {
    let fixture = Fixture::new();
    for (index, bytes) in [None, Some(b"invalid".to_vec()), Some(vec![b'x'; 8193])]
        .into_iter()
        .enumerate()
    {
        let root = fixture.0.join(format!(".clipraft-export-{index:032x}"));
        fs::create_dir(&root).unwrap();
        if let Some(bytes) = bytes {
            fs::write(root.join(NAME), bytes).unwrap();
        }
        assert!(cleanup_empty(&root).is_err());
        assert!(root.exists());
    }
    let stage = fixture.stage("restore");
    let original = stage.path().to_path_buf();
    drop(stage);
    let renamed = fixture
        .0
        .join(format!(".clipraft-restore-{}", "a".repeat(32)));
    fs::rename(original, &renamed).unwrap();
    assert!(cleanup_empty(&renamed).is_err());
    assert!(scan(&fixture.0)
        .unwrap()
        .candidates
        .iter()
        .all(|candidate| matches!(candidate.status, Status::Unrecognized { .. })));
}

#[test]
fn stale_scan_does_not_authorize_cleaning_new_content() {
    let fixture = Fixture::new();
    let stage = fixture.stage("export");
    let root = stage.path().to_path_buf();
    drop(stage);
    assert_eq!(
        scan(&fixture.0).unwrap().candidates[0].status,
        Status::Empty
    );
    fs::write(root.join("archive.part"), b"new content").unwrap();
    assert!(cleanup_empty(&root).is_err());
    assert_eq!(fs::read(root.join("archive.part")).unwrap(), b"new content");
}

#[test]
fn scan_is_bounded_and_does_not_recurse_into_unrelated_directories() {
    let fixture = Fixture::new();
    let nested = fixture.0.join("unrelated");
    fs::create_dir(&nested).unwrap();
    let stage = Directory::create(&nested, "snapshot").unwrap();
    let root = stage.path().to_path_buf();
    drop(stage);
    for index in 0..MAX_CANDIDATES + 1 {
        fs::create_dir(fixture.0.join(format!(".clipraft-export-{index:032x}"))).unwrap();
    }
    let report = scan(&fixture.0).unwrap();
    assert_eq!(report.candidates.len(), MAX_CANDIDATES);
    assert!(report.truncated);
    assert!(root.exists());
    assert!(scan(Path::new("relative")).is_err());
}

#[test]
fn child_process_lock_probe() {
    let Some(root) = std::env::var_os("CLIPRAFT_OWNER_TEST_ROOT") else {
        return;
    };
    let directory = Directory::claim(Path::new(&root)).unwrap().unwrap();
    let ready = std::env::var_os("CLIPRAFT_OWNER_TEST_READY").unwrap();
    fs::write(&ready, b"ready").unwrap();
    let release = PathBuf::from(std::env::var_os("CLIPRAFT_OWNER_TEST_RELEASE").unwrap());
    let started = std::time::Instant::now();
    while !release.exists() && started.elapsed() < std::time::Duration::from_secs(10) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    drop(directory);
}

#[test]
fn terminated_process_releases_owner_lock_for_empty_cleanup() {
    use std::os::windows::process::CommandExt;
    let fixture = Fixture::new();
    let stage = fixture.stage("snapshot");
    let root = stage.path().to_path_buf();
    drop(stage);
    let ready = fixture.0.join("ready");
    let release = fixture.0.join("release");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "archive::abandoned::tests::child_process_lock_probe",
            "--test-threads=1",
        ])
        .env("CLIPRAFT_OWNER_TEST_ROOT", &root)
        .env("CLIPRAFT_OWNER_TEST_READY", &ready)
        .env("CLIPRAFT_OWNER_TEST_RELEASE", &release)
        .creation_flags(0x08000000)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while !ready.exists() && started.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let active = scan(&fixture.0).unwrap().candidates[0].status == Status::Active;
    let blocked = cleanup_empty(&root).is_err();
    let stopped = child.kill().and_then(|()| child.wait()).is_ok();
    assert!(ready.exists() && active && blocked && stopped);
    cleanup_empty(&root).unwrap();
}
