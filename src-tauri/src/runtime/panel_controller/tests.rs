use super::*;
use crate::runtime::{
    new_paste_intent,
    test_support::{poison, Fixture},
};

pub(in crate::runtime) fn check_failures(app: &AppHandle) {
    let fixture = Fixture::new("panel-controller");
    let mut state = fixture.state();
    let (jobs, queued) = mpsc::sync_channel(4);
    state.jobs = jobs;
    let state = Arc::new(state);
    let mut controller = Controller {
        app: app.clone(),
        state: state.clone(),
        panel: Panel::default(),
        target: 0,
        notice: NoticeDeadline::default(),
    };
    controller.panel = Panel {
        items: vec![crate::model::ItemView {
            id: "synthetic".into(),
            ..Default::default()
        }],
        selected: 0,
        visible: true,
        error: None,
    };
    poison(&state.error);
    let generation = new_paste_intent();
    let old = generation.wrapping_sub(1);
    controller
        .handle(Control::PasteReport("stale".into(), old))
        .unwrap();
    controller
        .handle(Control::Hook(Action::Commit, old))
        .unwrap();
    assert!(queued.try_recv().is_err());
    assert!(controller.panel.visible);
    assert!(controller.panel.error.is_none());
    assert!(controller
        .handle(Control::Report("current".into()))
        .is_err());
    assert!(controller.handle(Control::Toggle).is_err());
    assert!(controller.panel.visible);
    poison(&state.items);
    assert!(controller.handle(Control::HistoryChanged).is_err());
    poison(&state.panel);
    assert!(controller
        .handle(Control::Hook(Action::Next, generation))
        .is_err());
    controller.notice.reset(Instant::now());
    controller.cancel_session();
    assert!(!controller.panel.visible);
    assert!(controller.panel.items.is_empty());
    assert_eq!(
        controller.notice.remaining(Instant::now()),
        std::time::Duration::from_secs(86400)
    );
    assert_ne!(paste_generation(), generation);
    controller
        .handle(Control::Hook(Action::Commit, generation))
        .unwrap();
    assert!(queued.try_recv().is_err());
    poison(&state.capture_context);
    assert!(controller.handle(Control::TogglePause).is_err());
    drop(CAPTURE_GATE.lock().unwrap());
}
