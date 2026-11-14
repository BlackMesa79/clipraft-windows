//! 使用通道控制后台时序，不接管键鼠，不依赖延时碰撞竞态。
use clipboard_m0::{
    model::ItemView,
    panel_neighborhood::{Coordinator, Loader, Request, Response, ViewState},
    panel_session::Direction,
};
use std::{
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

const DEADLINE: Duration = Duration::from_secs(5);

fn open(session: u64) -> (Coordinator, Request) {
    let ids: Arc<[String]> = (0..1001).map(|index| format!("id-{index}")).collect();
    Coordinator::open(session, ids, 5).unwrap()
}

fn response(request: Request) -> Response {
    let Request::Views(request) = request else {
        panic!("expected view request");
    };
    let items = request
        .ids
        .iter()
        .map(|id| ItemView {
            id: id.clone(),
            ..Default::default()
        })
        .collect();
    Response::Views {
        request,
        result: Ok(items),
    }
}

#[test]
fn blocked_reader_coalesces_one_thousand_inputs_and_delivers_only_latest_view() {
    let (started, calls) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let gate = Mutex::new(gate);
    let (notice, notified) = mpsc::channel();
    let loader = Loader::new(
        move |request| {
            let Request::Views(view) = &request else {
                panic!("expected view request");
            };
            started.send(view.position).unwrap();
            gate.lock().unwrap().recv_timeout(DEADLINE).unwrap();
            response(request)
        },
        move || notice.send(()).unwrap(),
    );
    let (mut panel, first) = open(1);
    loader.submit(first).unwrap();
    assert_eq!(calls.recv_timeout(DEADLINE).unwrap(), 1);
    for _ in 0..1000 {
        loader
            .submit(panel.navigate(Direction::Next).unwrap())
            .unwrap();
    }
    assert!(calls.try_recv().is_err());
    assert!(loader.take_response().unwrap().is_none());
    release.send(()).unwrap();
    assert_eq!(calls.recv_timeout(DEADLINE).unwrap(), 0);
    assert!(notified.try_recv().is_err());
    release.send(()).unwrap();
    notified.recv_timeout(DEADLINE).unwrap();
    panel.apply(loader.take_response().unwrap().unwrap());
    let ViewState::Ready { items, selected } = panel.view() else {
        panic!("not ready");
    };
    assert_eq!(items[*selected].id, "id-0");
    assert_eq!(panel.commit().as_deref(), Some("id-0"));
    assert!(loader.take_response().unwrap().is_none());
    loader.close().unwrap();
}

#[test]
fn clearing_in_flight_work_then_reopening_loads_only_the_new_session() {
    let (started, calls) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let gate = Mutex::new(gate);
    let (notice, notified) = mpsc::channel();
    let loader = Loader::new(
        move |request| {
            started.send(()).unwrap();
            gate.lock().unwrap().recv_timeout(DEADLINE).unwrap();
            response(request)
        },
        move || notice.send(()).unwrap(),
    );
    let (mut old, request) = open(1);
    loader.submit(request).unwrap();
    calls.recv_timeout(DEADLINE).unwrap();
    old.cancel();
    loader.clear().unwrap();
    let (mut new, request) = open(2);
    loader.submit(request).unwrap();
    release.send(()).unwrap();
    calls.recv_timeout(DEADLINE).unwrap();
    assert!(notified.try_recv().is_err());
    release.send(()).unwrap();
    notified.recv_timeout(DEADLINE).unwrap();
    new.apply(loader.take_response().unwrap().unwrap());
    assert!(matches!(old.view(), ViewState::Empty));
    assert!(matches!(new.view(), ViewState::Ready { .. }));
    loader.close().unwrap();
    assert!(loader.submit(open(3).1).is_err());
}

#[test]
fn background_failure_is_delivered_without_restoring_old_content() {
    let (notice, notified) = mpsc::channel();
    let loader = Loader::new(
        |request| request.failed("synthetic storage failure".into()),
        move || notice.send(()).unwrap(),
    );
    let (mut panel, request) = open(1);
    loader.submit(request).unwrap();
    notified.recv_timeout(DEADLINE).unwrap();
    panel.apply(loader.take_response().unwrap().unwrap());
    assert!(matches!(panel.view(), ViewState::Failed(_)));
    assert_eq!(panel.commit().as_deref(), Some("id-1"));
    loader.close().unwrap();
}
