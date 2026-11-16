use super::*;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
fn success() -> Success {
    Success {
        restored_root: None,
        replace_current: false,
        cleanup_warning: None,
    }
}
fn complete(control: &Control) -> Result<Success> {
    control.start()?;
    control.snapshotting()?;
    control.snapshot_finished()?;
    Ok(success())
}
fn wait(manager: &Manager) -> Report {
    let started = Instant::now();
    loop {
        let report = manager.status().unwrap().unwrap();
        if !report.running {
            return report;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "background task timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn cancellation_keeps_slot_until_cleanup_finishes_and_old_ids_are_rejected() {
    let manager = Arc::new(Manager::default());
    let (ready, ready_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let first = manager
        .start(Kind::Export, move |control| {
            control.start()?;
            ready.send(()).unwrap();
            release_rx.recv().unwrap();
            control.check()?;
            Ok(success())
        })
        .unwrap();
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(manager.cancel(first).unwrap());
    assert!(manager.status().unwrap().unwrap().running);
    assert!(manager.start(Kind::Restore, complete).is_err());
    release.send(()).unwrap();
    let report = wait(&manager);
    assert!(matches!(report.outcome, Some(Outcome::Cancelled { .. })));
    assert_eq!(report.progress.phase, Phase::Cancelled);
    let second = manager.start(Kind::Restore, complete).unwrap();
    assert!(second > first);
    assert!(manager.cancel(first).is_err());
    assert!(matches!(
        wait(&manager).outcome,
        Some(Outcome::Succeeded(_))
    ));
    assert!(!manager.cancel(second).unwrap());
}
#[test]
fn failures_and_panics_release_task_slot_and_report_failure() {
    let manager = Arc::new(Manager::default());
    manager
        .start(Kind::Restore, |_| {
            anyhow::bail!("synthetic restore failure")
        })
        .unwrap();
    assert!(
        matches!(wait(&manager).outcome, Some(Outcome::Failed { message }) if message.contains("synthetic"))
    );
    manager
        .start(Kind::Export, |_| panic!("synthetic worker panic"))
        .unwrap();
    let report = wait(&manager);
    assert_eq!(report.progress.phase, Phase::Failed);
    assert!(matches!(report.outcome, Some(Outcome::Failed { .. })));
    manager.start(Kind::Export, complete).unwrap();
    assert_eq!(wait(&manager).progress.phase, Phase::Complete);
}
#[test]
fn incomplete_success_is_not_reported_as_completed() {
    let manager = Arc::new(Manager::default());
    manager.start(Kind::Export, |_| Ok(success())).unwrap();
    let report = wait(&manager);
    assert_eq!(report.progress.phase, Phase::Failed);
    assert!(matches!(report.outcome, Some(Outcome::Failed { .. })));
}
#[test]
fn publication_disables_cancellation_and_preserves_cleanup_warning() {
    let manager = Arc::new(Manager::default());
    let (ready, ready_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let id = manager
        .start(Kind::Export, move |control| {
            control.start()?;
            control.writing(0)?;
            control.publish()?;
            ready.send(()).unwrap();
            release_rx.recv().unwrap();
            control.finish()?;
            Ok(Success {
                restored_root: None,
                replace_current: false,
                cleanup_warning: Some("synthetic cleanup warning".into()),
            })
        })
        .unwrap();
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(!manager.cancel(id).unwrap());
    assert!(manager.start(Kind::Restore, complete).is_err());
    release.send(()).unwrap();
    assert!(matches!(
        wait(&manager).outcome,
        Some(Outcome::Succeeded(Success {
            cleanup_warning: Some(_),
            ..
        }))
    ));
}
#[test]
fn concurrent_starts_reserve_exactly_one_task() {
    let manager = Arc::new(Manager::default());
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    thread::scope(|scope| {
        let attempts: Vec<_> = (0..2)
            .map(|_| {
                let manager = manager.clone();
                let barrier = barrier.clone();
                let release = release.clone();
                scope.spawn(move || {
                    barrier.wait();
                    manager
                        .start(Kind::Export, move |control| {
                            let (lock, condition) = &*release;
                            let _released = condition
                                .wait_while(lock.lock().unwrap(), |value| !*value)
                                .unwrap();
                            complete(control)
                        })
                        .is_ok()
                })
            })
            .collect();
        barrier.wait();
        let successes = attempts
            .into_iter()
            .map(|attempt| usize::from(attempt.join().unwrap()))
            .sum::<usize>();
        assert_eq!(successes, 1);
    });
    let (lock, condition) = &*release;
    *lock.lock().unwrap() = true;
    condition.notify_all();
    assert_eq!(wait(&manager).id, 1);
}

#[test]
fn migration_restart_reservation_blocks_jobs_and_releases_on_failed_schedule() {
    let manager = Arc::new(Manager::default());
    let guard = manager.begin_restart().unwrap();
    assert!(manager.start(Kind::Export, complete).is_err());
    assert!(manager.begin_restart().is_err());
    drop(guard);
    manager.start(Kind::Export, complete).unwrap();
    wait(&manager);
    manager.begin_restart().unwrap().commit();
    assert!(manager.start(Kind::Restore, complete).is_err());
}
#[test]
fn running_archive_prevents_migration_restart() {
    let manager = Arc::new(Manager::default());
    let (release, release_rx) = mpsc::channel();
    manager
        .start(Kind::Export, move |control| {
            release_rx.recv().unwrap();
            complete(control)
        })
        .unwrap();
    assert!(manager.begin_restart().is_err());
    release.send(()).unwrap();
    wait(&manager);
    assert!(manager.begin_restart().is_ok());
}

#[test]
fn restored_restart_uses_successful_task_result_and_rejects_stale_ids() {
    let manager = Arc::new(Manager::default());
    let target = PathBuf::from("D:/synthetic/restored");
    let saved = target.clone();
    let id = manager
        .start(Kind::Restore, move |control| {
            complete(control)?;
            Ok(Success {
                restored_root: Some(saved),
                replace_current: false,
                cleanup_warning: None,
            })
        })
        .unwrap();
    wait(&manager);
    assert!(manager.restored_for_restart(id + 1).is_err());
    let (actual, restart) = manager.restored_for_restart(id).unwrap();
    assert_eq!(actual.restored_root, Some(target));
    assert!(manager.start(Kind::Export, complete).is_err());
    drop(restart);
    let next = manager.start(Kind::Export, complete).unwrap();
    wait(&manager);
    assert!(manager.restored_for_restart(next).is_err());
    assert!(manager.begin_restart().is_ok());
}
