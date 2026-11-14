//! 合成生命周期与私有线程消息；不安装全局钩子或注入键盘。
use super::*;
use std::{cell::RefCell, rc::Rc};

type Events = Rc<RefCell<Vec<&'static str>>>;
struct Resource(Events);
impl Drop for Resource {
    fn drop(&mut self) {
        self.0.borrow_mut().push("release");
    }
}

#[test]
fn startup_reply_precedes_messages_and_resource_release() {
    let events = Events::default();
    let (reply, wait) = mpsc::sync_channel(1);
    serve(reply, Resource(events.clone()), || {
        wait.try_recv().unwrap().unwrap();
        events.borrow_mut().push("messages");
        Ok(())
    })
    .unwrap();
    assert_eq!(*events.borrow(), ["messages", "release"]);
}

#[test]
fn abandoned_startup_reply_releases_resource_without_reading_messages() {
    let events = Events::default();
    let (reply, wait) = mpsc::sync_channel(1);
    drop(wait);
    let error = serve(reply, Resource(events.clone()), || {
        panic!("must not read messages after abandoned startup")
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "键盘线程启动回复接收方已退出");
    assert_eq!(*events.borrow(), ["release"]);
}

#[test]
fn message_read_error_stops_dispatch_and_releases_resource() {
    let events = Events::default();
    let (reply, wait) = mpsc::sync_channel(1);
    let mut reads = 0;
    let error = serve(reply, Resource(events.clone()), || {
        message_loop(
            |message| {
                reads += 1;
                if reads == 1 {
                    message.message = CANCEL_PANEL_MESSAGE;
                    return Ok(true);
                }
                Err(anyhow!("synthetic message read failure"))
            },
            |message| {
                assert_eq!(message.message, CANCEL_PANEL_MESSAGE);
                events.borrow_mut().push("dispatch");
            },
        )
    })
    .unwrap_err();
    wait.try_recv().unwrap().unwrap();
    assert_eq!(error.to_string(), "synthetic message read failure");
    assert_eq!(reads, 2);
    assert_eq!(*events.borrow(), ["dispatch", "release"]);
}

#[test]
fn normal_quit_never_dispatches_a_stale_message() {
    let mut reads = 0;
    let mut delivered = Vec::new();
    message_loop(
        |message| {
            reads += 1;
            message.message = WM_APP + reads;
            Ok(reads <= 2)
        },
        |message| delivered.push(message.message),
    )
    .unwrap();
    assert_eq!(delivered, [WM_APP + 1, WM_APP + 2]);
    assert_eq!(reads, 3);
}

#[test]
fn thread_publication_clears_before_even_failed_removal() {
    for fail in [false, true] {
        let thread_id = AtomicU32::new(7);
        let result = release_thread(&thread_id, || {
            assert_eq!(thread_id.load(Ordering::Acquire), 0);
            if fail {
                return Err(anyhow!("synthetic removal failure"));
            }
            Ok(())
        });
        assert_eq!(thread_id.load(Ordering::Acquire), 0);
        assert_eq!(result.is_err(), fail);
    }
}

#[test]
fn unstarted_thread_never_posts_to_caller_and_live_post_errors_propagate() {
    post_if_running(0, WM_QUIT, |_, _| panic!("must not target calling thread")).unwrap();
    post_if_running(0, CANCEL_PANEL_MESSAGE, |_, _| {
        panic!("must not target calling thread")
    })
    .unwrap();
    for message in [WM_QUIT, CANCEL_PANEL_MESSAGE] {
        let result = post_if_running(77, message, |id, actual| {
            assert_eq!((id, actual), (77, message));
            Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x80004005_u32 as i32,
            )))
        });
        assert!(result.is_err());
    }
}

#[test]
fn native_private_thread_queue_receives_stop_and_never_quits_the_caller() {
    // 所有线程消息都在本测试新建的线程队列中，不使用桌面窗口。
    thread::spawn(|| unsafe {
        let mut message = MSG::default();
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
        post_thread(0, WM_QUIT).unwrap();
        assert!(!PeekMessageW(&mut message, None, WM_QUIT, WM_QUIT, PM_REMOVE).as_bool());
        let id = GetCurrentThreadId();
        post_thread(id, CANCEL_PANEL_MESSAGE).unwrap();
        post_thread(id, WM_QUIT).unwrap();
        let mut delivered = Vec::new();
        message_loop(read_message, |message| delivered.push(message.message)).unwrap();
        assert_eq!(delivered, [CANCEL_PANEL_MESSAGE]);
    })
    .join()
    .unwrap();
}
