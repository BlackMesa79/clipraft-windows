use super::*;
#[test]
fn only_main_window_can_query_or_cancel_archive_tasks() {
    assert!(main_only("main").is_ok());
    for label in ["quick", "preview", "", "Main"] {
        assert!(main_only(label).is_err());
    }
}
#[test]
fn background_restore_reports_result_and_keeps_password_out_of_report() {
    use crate::{
        archive::{bundle, jobs::Outcome},
        storage::Storage,
    };
    use std::{
        fs,
        time::{Duration, Instant},
    };
    let root = std::env::temp_dir().join(format!(
        "clipraft-service-test-{}",
        crate::model::digest(&*crate::storage::crypto::random_key().unwrap())
    ));
    fs::create_dir(&root).unwrap();
    let source = root.join("source");
    let master = [61; 32];
    let mut storage = Storage::open(&source, &master).unwrap();
    let frozen = root.join("snapshot");
    storage.create_portable_snapshot(&frozen, &master).unwrap();
    let archive = root.join("history.clipraft");
    let password = "synthetic-service-password";
    bundle::export(
        &frozen,
        &master,
        &mut fs::File::create(&archive).unwrap(),
        password.as_bytes(),
    )
    .unwrap();
    let service = Service::new(source);
    let target = root.join("restored");
    let id = service
        .start_restore(
            archive.clone(),
            target.clone(),
            Zeroizing::new(password.into()),
        )
        .unwrap();
    let wait = || {
        let start = Instant::now();
        loop {
            let report = service.jobs.status().unwrap().unwrap();
            if !report.running {
                return report;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let report = wait();
    assert_eq!(report.id, id);
    assert!(matches!(report.outcome, Some(Outcome::Succeeded(_))));
    assert!(!serde_json::to_string(&report).unwrap().contains(password));
    assert_eq!(*crate::platform::master_key(&target).unwrap(), master);
    let wrong_target = root.join("wrong");
    service
        .start_restore(
            archive,
            wrong_target.clone(),
            Zeroizing::new("wrong-password".into()),
        )
        .unwrap();
    assert!(matches!(wait().outcome, Some(Outcome::Failed { .. })));
    assert!(!wrong_target.exists());
    drop(storage);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn default_restore_uses_managed_temporary_directory_without_changing_current_root() {
    use crate::{
        archive::{bundle, jobs::Outcome},
        storage::Storage,
    };
    use std::{
        fs,
        time::{Duration, Instant},
    };
    let root = std::env::temp_dir().join(format!(
        "clipraft-current-restore-{}",
        crate::model::digest(&*crate::storage::crypto::random_key().unwrap())
    ));
    fs::create_dir(&root).unwrap();
    let current = root.join("current");
    let master = crate::platform::master_key(&current).unwrap();
    let mut storage = Storage::open(&current, &master).unwrap();
    let snapshot = root.join("snapshot");
    storage
        .create_portable_snapshot(&snapshot, &master)
        .unwrap();
    let archive = root.join("history.clipraft");
    bundle::export(
        &snapshot,
        &master,
        &mut fs::File::create(&archive).unwrap(),
        b"synthetic-password",
    )
    .unwrap();
    let before = fs::read(current.join("history.db")).unwrap();
    let service = Service::new(current.clone());
    service
        .start_restore_current(archive.clone(), Zeroizing::new("wrong".into()))
        .unwrap();
    let wait = || {
        let started = Instant::now();
        loop {
            let report = service.jobs.status().unwrap().unwrap();
            if !report.running {
                return report;
            }
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    assert!(matches!(wait().outcome, Some(Outcome::Failed { .. })));
    assert!(!fs::read_dir(&root).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".clipraft-restore-")));
    service
        .start_restore_current(archive, Zeroizing::new("synthetic-password".into()))
        .unwrap();
    let report = wait();
    let Some(Outcome::Succeeded(success)) = report.outcome else {
        panic!("default restore did not succeed");
    };
    assert!(success.replace_current);
    let staged = success.restored_root.unwrap();
    assert_ne!(staged, current);
    assert!(staged
        .parent()
        .unwrap()
        .join(".clipraft-owner.json")
        .exists());
    assert_eq!(fs::read(current.join("history.db")).unwrap(), before);
    drop(storage);
    fs::remove_dir_all(root).unwrap();
}
