use super::*;
use std::{
    cell::Cell,
    sync::atomic::{AtomicU64, Ordering},
};
fn target() -> Target {
    Target {
        window: 1,
        focus: Identity {
            raw: 2,
            pid: 3,
            tid: 4,
        },
    }
}
fn sample(ms: u64, matches: bool) -> Sample {
    Sample {
        elapsed: Duration::from_millis(ms),
        foreground: 1,
        focused: matches.then_some(target().focus),
        valid: true,
    }
}
#[test]
fn returns_only_after_expected_input_focus_is_stable() {
    let generation = AtomicU64::new(1);
    let intent = PasteIntent::new(&generation, 1);
    let mut samples = vec![
        sample(0, false),
        sample(5, true),
        sample(20, false),
        sample(25, true),
        sample(54, true),
        sample(55, true),
    ]
    .into_iter();
    let pauses = Cell::new(0);
    assert_eq!(
        wait_with(
            target(),
            &intent,
            || samples.next().unwrap(),
            || pauses.set(pauses.get() + 1)
        )
        .unwrap(),
        PasteStatus::Complete
    );
    assert_eq!(pauses.get(), 5);
}
#[test]
fn wrong_child_times_out_even_when_top_level_matches() {
    let generation = AtomicU64::new(1);
    let intent = PasteIntent::new(&generation, 1);
    let error = wait_with(
        target(),
        &intent,
        || sample(300, false),
        || panic!("已到界限不应再等待"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("焦点尚未恢复"));
}
#[test]
fn superseded_intent_never_samples_or_restores_focus() {
    let generation = AtomicU64::new(2);
    let intent = PasteIntent::new(&generation, 1);
    assert_eq!(
        wait_with(
            target(),
            &intent,
            || panic!("旧意图不采样"),
            || panic!("旧意图不等待")
        )
        .unwrap(),
        PasteStatus::Superseded
    );
}
#[test]
fn cancellation_during_stability_wait_stops_before_ready() {
    let generation = AtomicU64::new(1);
    let intent = PasteIntent::new(&generation, 1);
    assert_eq!(
        wait_with(
            target(),
            &intent,
            || sample(0, true),
            || generation.store(2, Ordering::Release)
        )
        .unwrap(),
        PasteStatus::Superseded
    );
}
#[test]
fn invalid_or_changed_target_is_not_accepted() {
    let generation = AtomicU64::new(1);
    let intent = PasteIntent::new(&generation, 1);
    for (foreground, valid) in [(99, true), (1, false)] {
        assert!(wait_with(
            target(),
            &intent,
            || Sample {
                foreground,
                valid,
                ..sample(0, true)
            },
            || panic!("无效目标不等待")
        )
        .is_err());
    }
}
