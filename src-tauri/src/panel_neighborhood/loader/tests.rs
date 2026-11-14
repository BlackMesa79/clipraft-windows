use super::*;
use crate::panel_neighborhood::Coordinator;

fn request(session: u64) -> Request {
    Coordinator::open(session, vec!["one".into(), "two".into()].into(), 5)
        .unwrap()
        .1
}

#[test]
fn mailbox_replaces_response_and_coalesces_outstanding_notification() {
    let loader = Loader::new(|request| request.failed("unused".into()), || {});
    let first = request(1);
    loader.0.queue.lock().unwrap().wanted = Some(first.key());
    assert!(loader.0.publish(first.failed("first".into())).unwrap());
    let latest = request(2);
    loader.0.queue.lock().unwrap().wanted = Some(latest.key());
    assert!(!loader.0.publish(latest.failed("latest".into())).unwrap());
    let Response::Views { result, .. } = loader.take_response().unwrap().unwrap() else {
        panic!("expected view response");
    };
    assert_eq!(result.unwrap_err(), "latest");
    assert!(loader.take_response().unwrap().is_none());
}

#[test]
fn close_atomically_discards_pending_request_and_rejects_in_flight_result() {
    let loader = Loader::new(|request| request.failed("unused".into()), || {});
    let pending = request(1);
    loader.0.queue.lock().unwrap().wanted = Some(pending.key());
    loader.0.queue.lock().unwrap().pending = Some(pending.clone());
    loader.close().unwrap();
    assert!(loader.0.next_request().unwrap().is_none());
    assert!(!loader.0.publish(pending.failed("late".into())).unwrap());
    assert!(loader.submit(request(2)).is_err());
    assert!(loader.take_response().unwrap().is_none());
}
