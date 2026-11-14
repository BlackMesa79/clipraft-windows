use super::*;
use crate::runtime::{history_gate, history_index, test_support::Fixture};
use std::time::Duration;

struct Harness {
    controller: Controller,
    messages: Receiver<Control>,
    jobs: Receiver<Job>,
    ids: Arc<[String]>,
    _fixture: Fixture,
}

impl Harness {
    fn new(app: &AppHandle) -> Self {
        let fixture = Fixture::new("u3-integration");
        let mut state = fixture.state();
        let (control, messages) = mpsc::channel();
        let (jobs, pending) = mpsc::sync_channel(4);
        state.control = control;
        state.jobs = jobs;
        {
            let mut storage = state.storage.lock().unwrap();
            for index in 0..241 {
                storage
                    .save(&clip(&format!("U3 synthetic {index}")))
                    .unwrap();
            }
        }
        history_index::initialize(&state).unwrap();
        let ids = state.history_index.snapshot().unwrap().ids.clone();
        Self {
            controller: Controller::new(app.clone(), Arc::new(state)),
            messages,
            jobs: pending,
            ids,
            _fixture: fixture,
        }
    }

    fn settle(&mut self) {
        while self.controller.panel.loading {
            let message = self.messages.recv_timeout(Duration::from_secs(5)).unwrap();
            self.controller.handle(message).unwrap();
        }
    }

    fn selected_id(&self) -> &str {
        self.controller
            .coordinator
            .as_ref()
            .unwrap()
            .selected_id()
            .unwrap()
    }
}

fn clip(text: &str) -> crate::model::Clip {
    crate::model::Clip {
        formats: vec![crate::model::FormatData {
            name: "CF_UNICODETEXT".into(),
            bytes: text
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
        }],
    }
}

pub(in crate::runtime) fn check_integration(app: &AppHandle) {
    let mut harness = Harness::new(app);
    selection_and_commit(&mut harness);
    frozen_order_and_reopen(&mut harness);
    sensitive_and_deletion(&mut harness);
    delayed_result_and_reopen(&mut harness);
    failed_views_still_commit(&mut harness);
    index_and_mutation_reject_open(&mut harness);
}

fn selection_and_commit(harness: &mut Harness) {
    let storage_state = harness.controller.state.clone();
    let storage = storage_state.storage.lock().unwrap();
    // 实际后台 reader 被 storage 锁挡住，控制路径仍能打开、导航并提交。
    harness.controller.start_session().unwrap();
    assert_eq!(harness.controller.panel.position, 1);
    assert_eq!(harness.controller.panel.total, 241);
    for _ in 0..240 {
        harness.controller.navigate(Action::Next).unwrap();
    }
    assert_eq!(harness.controller.panel.position, 0);
    harness.controller.navigate(Action::Previous).unwrap();
    assert_eq!(harness.selected_id(), harness.ids[240]);
    assert!(harness.controller.panel.items.is_empty());
    assert!(harness.controller.panel.loading);
    harness.controller.commit(77).unwrap();
    assert!(
        matches!(harness.jobs.try_recv().unwrap(), Job::Paste { id, generation: 77, .. } if id == harness.ids[240])
    );
    harness.controller.commit(77).unwrap();
    assert!(harness.jobs.try_recv().is_err());
    drop(storage);
    harness.controller.start_session().unwrap();
    harness.settle();
    assert!(harness.controller.panel.items.len() <= 15);
    assert_eq!(
        harness.controller.panel.items[harness.controller.panel.selected].id,
        harness.ids[1]
    );
    for count in [3, 4, 15] {
        harness
            .controller
            .state
            .settings
            .lock()
            .unwrap()
            .quick_visible_count = count;
        harness.controller.start_session().unwrap();
        harness.settle();
        assert_eq!(harness.controller.panel.items.len(), count as usize);
        assert_eq!(
            harness.controller.panel.items[harness.controller.panel.selected].id,
            harness.ids[1]
        );
    }
}

fn frozen_order_and_reopen(harness: &mut Harness) {
    let state = harness.controller.state.clone();
    let change = history_gate::begin(&state).unwrap();
    let new_clip = clip("U3 synthetic added later");
    {
        let mut storage = state.storage.lock().unwrap();
        storage.save(&new_clip).unwrap();
        storage.touch(&harness.ids[240]).unwrap();
        storage
            .set_pinned(&[harness.ids[239].clone()], true)
            .unwrap();
        state.history_index.refresh(&storage).unwrap();
    }
    drop(change);
    harness.controller.history_changed().unwrap();
    harness.settle();
    assert_eq!(harness.controller.panel.total, 241);
    assert_eq!(harness.selected_id(), harness.ids[1]);
    harness.controller.navigate(Action::Next).unwrap();
    assert_eq!(harness.selected_id(), harness.ids[2]);
    harness.controller.start_session().unwrap();
    harness.settle();
    assert_eq!(harness.controller.panel.total, 242);
    assert_eq!(
        harness.controller.coordinator.as_ref().unwrap().total(),
        242
    );
    assert_eq!(
        state.history_index.snapshot().unwrap().ids[0],
        harness.ids[239]
    );
}

fn sensitive_and_deletion(harness: &mut Harness) {
    for _ in 0..220 {
        harness.controller.navigate(Action::Next).unwrap();
    }
    harness.settle();
    let selected = harness.selected_id().to_owned();
    let state = harness.controller.state.clone();
    let stale = harness.controller.panel.clone();
    let change = history_gate::begin(&state).unwrap();
    {
        let mut storage = state.storage.lock().unwrap();
        storage
            .set_sensitive(std::slice::from_ref(&selected), true, 10)
            .unwrap();
        storage.delete(&[harness.ids[0].clone()]).unwrap();
        state.history_index.refresh(&storage).unwrap();
    }
    // 故意暂不处理 HistoryChanged，旧正文也不能经过真实 publish 或快照读取。
    publish(&harness.controller.app, &state, &stale).unwrap();
    assert!(super::super::panel_snapshot(&state)
        .unwrap()
        .items
        .is_empty());
    harness.controller.history_changed().unwrap();
    assert!(harness.controller.panel.items.is_empty());
    harness.controller.navigate(Action::Next).unwrap();
    harness.controller.navigate(Action::Previous).unwrap();
    drop(change);
    harness.controller.history_changed().unwrap();
    harness.settle();
    assert_eq!(harness.selected_id(), selected);
    assert_eq!(harness.controller.panel.total, 241);
    let item = &harness.controller.panel.items[harness.controller.panel.selected];
    assert!(item.sensitive);
    assert!(item.thumbnail.is_none() && item.files.is_empty() && item.source_title.is_none());
    let change = history_gate::begin(&state).unwrap();
    state.storage.lock().unwrap().delete(&[selected]).unwrap();
    drop(change);
    harness.controller.history_changed().unwrap();
    harness.settle();
    assert!(!harness.controller.panel.visible);
    harness.controller.commit(77).unwrap();
    assert!(harness.jobs.try_recv().is_err());
    history_index::initialize(&state).unwrap();
}

fn delayed_result_and_reopen(harness: &mut Harness) {
    use crate::panel_neighborhood::Loader;
    let state = harness.controller.state.clone();
    let reader_state = state.clone();
    let control = state.control.clone();
    let (started, wait_started) = mpsc::channel();
    let (release, wait_release) = mpsc::channel();
    let wait_release = std::sync::Mutex::new(wait_release);
    harness.controller.loader = Loader::new(
        move |request| {
            let response = session::read(&reader_state, request);
            started.send(()).unwrap();
            wait_release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            response
        },
        move || {
            control.send(Control::NeighborhoodReady).unwrap();
        },
    );
    harness.controller.start_session().unwrap();
    let old_session = harness.controller.panel.session;
    wait_started.recv_timeout(Duration::from_secs(5)).unwrap();
    harness.controller.close().unwrap();
    harness.controller.start_session().unwrap();
    assert!(harness.controller.panel.session > old_session);
    release.send(()).unwrap();
    wait_started.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(harness.controller.panel.items.is_empty());
    release.send(()).unwrap();
    harness.settle();
    assert!(!harness.controller.panel.items.is_empty());
    harness.controller.close().unwrap();
    harness.controller.loader = Controller::new(harness.controller.app.clone(), state).loader;
}

fn failed_views_still_commit(harness: &mut Harness) {
    let control = harness.controller.state.control.clone();
    harness.controller.loader = crate::panel_neighborhood::Loader::new(
        |request| request.failed("合成读取失败".into()),
        move || {
            control.send(Control::NeighborhoodReady).unwrap();
        },
    );
    harness.controller.start_session().unwrap();
    harness.settle();
    assert_eq!(
        harness.controller.panel.error.as_deref(),
        Some("合成读取失败")
    );
    assert!(harness.controller.panel.items.is_empty());
    let selected = harness.selected_id().to_owned();
    harness.controller.commit(78).unwrap();
    assert!(matches!(harness.jobs.try_recv().unwrap(), Job::Paste { id, .. } if id == selected));
}

fn index_and_mutation_reject_open(harness: &mut Harness) {
    let state = harness.controller.state.clone();
    let change = history_gate::begin(&state).unwrap();
    assert!(harness
        .controller
        .start_session()
        .unwrap_err()
        .to_string()
        .contains("历史正在更新"));
    assert!(harness.controller.coordinator.is_none());
    drop(change);
    state.history_index.fail("合成索引失败".into()).unwrap();
    assert!(harness
        .controller
        .start_session()
        .unwrap_err()
        .to_string()
        .contains("合成索引失败"));
    assert!(harness.controller.coordinator.is_none());
}
