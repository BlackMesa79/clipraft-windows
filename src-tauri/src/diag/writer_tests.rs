//! 合成写入故障与私有线程消息队列，不调用键盘钩子或桌面剪贴板。
use super::*;
use std::io;

#[test]
fn reply_precedes_reads_and_thread_publication_clears_on_normal_exit() {
    let thread = AtomicU32::new(7);
    let (ready, wait) = mpsc::sync_channel(1);
    serve(&thread, ready, || {
        wait.try_recv().unwrap();
        assert_eq!(thread.load(Ordering::Acquire), 7);
        Ok(())
    })
    .unwrap();
    assert_eq!(thread.load(Ordering::Acquire), 0);
}

#[test]
fn abandoned_reply_clears_thread_without_reading_messages() {
    let thread = AtomicU32::new(7);
    let (ready, wait) = mpsc::sync_channel(1);
    drop(wait);
    assert!(serve(&thread, ready, || panic!(
        "must not read after abandoned reply"
    ))
    .is_err());
    assert_eq!(thread.load(Ordering::Acquire), 0);
}

#[test]
fn key_text_and_drop_count_preserve_schema_and_order() {
    let (send, pending) = mpsc::sync_channel(2);
    send.send((3, "synthetic".into())).unwrap();
    let lost = AtomicU32::new(2);
    let message = MSG {
        message: KEY_MESSAGE,
        wParam: WPARAM(0x100 | 512 | (1 << 12)),
        lParam: LPARAM(9),
        ..Default::default()
    };
    let mut output = Vec::new();
    write_message(&mut output, &message, &pending, &lost).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.starts_with("t_ms=9 key vk=other down=true injected=false swallowed=false action=Some(Open) ctrl_async=false scan=0x00\n"));
    assert!(text.contains("\nt_ms=3 synthetic\n"));
    assert!(text.ends_with("dropped=2\n"));
    assert_eq!(lost.load(Ordering::Relaxed), 0);
}

struct WriteFailure;
impl Write for WriteFailure {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("synthetic write failure"))
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("must not flush after failed write")
    }
}
struct FlushFailure;
impl Write for FlushFailure {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("synthetic flush failure"))
    }
}
fn failed_output(writer: &mut impl Write) {
    let (send, pending) = mpsc::sync_channel(1);
    send.send((1, "synthetic".into())).unwrap();
    let (ready, _wait) = mpsc::sync_channel(1);
    let thread = AtomicU32::new(7);
    let lost = AtomicU32::new(0);
    let mut reads = 0;
    let error = serve(&thread, ready, || {
        messages(writer, &pending, &lost, |message| {
            reads += 1;
            assert_eq!(reads, 1);
            message.message = TEXT_MESSAGE;
            Ok(true)
        })
    })
    .unwrap_err();
    assert!(error.to_string().contains("synthetic"));
    assert_eq!(reads, 1);
    assert_eq!(thread.load(Ordering::Acquire), 0);
}
#[test]
fn failed_write_stops_before_reading_another_message() {
    failed_output(&mut WriteFailure);
}
#[test]
fn failed_flush_stops_before_reading_another_message() {
    failed_output(&mut FlushFailure);
}

#[test]
fn failed_message_read_clears_publication_without_writing() {
    let (ready, _wait) = mpsc::sync_channel(1);
    let (_send, pending) = mpsc::sync_channel(1);
    let thread = AtomicU32::new(7);
    let lost = AtomicU32::new(0);
    let mut output = Vec::new();
    let result = serve(&thread, ready, || {
        messages(&mut output, &pending, &lost, |_| {
            Err(anyhow!("synthetic read failure"))
        })
    });
    assert!(result.is_err());
    assert!(output.is_empty());
    assert_eq!(thread.load(Ordering::Acquire), 0);
}

#[test]
fn native_private_queue_drains_text_and_stops_on_quit() {
    thread::spawn(|| unsafe {
        let mut message = MSG::default();
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
        let id = GetCurrentThreadId();
        PostThreadMessageW(id, KEY_MESSAGE, WPARAM(0x100), LPARAM(4)).unwrap();
        PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)).unwrap();
        let (send, pending) = mpsc::sync_channel(1);
        send.send((5, "synthetic".into())).unwrap();
        let mut output = Vec::new();
        messages(&mut output, &pending, &AtomicU32::new(0), read_message).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("vk=other"));
        assert!(text.ends_with("t_ms=5 synthetic\n"));
    })
    .join()
    .unwrap();
}
