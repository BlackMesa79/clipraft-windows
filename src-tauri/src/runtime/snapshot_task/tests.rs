use super::*;
use crate::archive::control::Phase;

#[test]
fn runtime_snapshot_releases_maintenance_and_locks_on_success_and_failure() {
    let _serial = TEST_SERIAL.lock().unwrap();
    let root = std::env::temp_dir().join(format!(
        "clipraft-runtime-snapshot-{}",
        crate::model::digest(&*crate::storage::crypto::random_key().unwrap())
    ));
    std::fs::create_dir(&root).unwrap();
    let state = crate::runtime::test_support::state(&root.join("source"));
    let target = root.join("snapshot");
    let control = Control::default();
    create_with_state(&state, &target, &[72; 32], &control).unwrap();
    assert_eq!(control.progress().unwrap().phase, Phase::Complete);
    assert!(!active());
    assert!(state.storage.try_lock().is_ok());
    assert!(state.capture_context.try_lock().is_ok());
    Storage::verify_snapshot(&target, &[72; 32]).unwrap();
    let control = Control::default();
    assert!(create_with_state(&state, &root.join("wrong-key"), &[73; 32], &control).is_err());
    assert_eq!(control.progress().unwrap().phase, Phase::Failed);
    assert!(!root.join("wrong-key").exists());
    assert!(!active());
    assert!(state.storage.try_lock().is_ok());
    let control = Control::default();
    control.cancel().unwrap();
    assert!(create_with_state(&state, &root.join("cancelled"), &[72; 32], &control).is_err());
    assert!(!active());
    drop(state);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn waiting_for_storage_lock_is_cancellable_without_touching_clipboard() {
    let lock = Mutex::new(());
    let held = lock.lock().unwrap();
    let control = Control::default();
    thread::scope(|scope| {
        let waiter = scope.spawn(|| lock_checked(&lock, &control).is_err());
        control.cancel().unwrap();
        assert!(waiter.join().unwrap());
    });
    drop(held);
    assert!(lock.try_lock().is_ok());
}
