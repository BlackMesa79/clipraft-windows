//! 诊断线程独占文件；写入失败即停用，不能递归向自己的日志通道报告。
use super::*;
use anyhow::Context;
use std::sync::mpsc::{Receiver, SyncSender};

struct Publication<'a>(&'a AtomicU32);
impl Drop for Publication<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Release);
    }
}

pub(super) fn run(
    mut file: std::fs::File,
    pending: Receiver<(u64, String)>,
    ready: SyncSender<()>,
) {
    let mut message = MSG::default();
    // false 表示队列为空；调用本身建立线程队列。
    unsafe {
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
    }
    THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    let result = serve(&THREAD, ready, || {
        messages(&mut file, &pending, &LOST, read_message)
    });
    if let Err(error) = result {
        eprintln!("diagnostic logger failed: {error:#}");
    }
}

fn serve(
    thread: &AtomicU32,
    ready: SyncSender<()>,
    run: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let _publication = Publication(thread);
    ready
        .send(())
        .map_err(|_| anyhow!("诊断启动回复接收方已退出"))?;
    run()
}

fn read_message(message: &mut MSG) -> Result<bool> {
    match unsafe { GetMessageW(message, None, 0, 0) }.0 {
        -1 => Err(windows::core::Error::from_win32()).context("诊断消息读取失败"),
        0 => Ok(false),
        _ => Ok(true),
    }
}

fn messages(
    writer: &mut impl Write,
    pending: &Receiver<(u64, String)>,
    lost: &AtomicU32,
    mut read: impl FnMut(&mut MSG) -> Result<bool>,
) -> Result<()> {
    let mut message = MSG::default();
    while read(&mut message)? {
        write_message(writer, &message, pending, lost)?;
    }
    Ok(())
}

fn write_message(
    writer: &mut impl Write,
    message: &MSG,
    pending: &Receiver<(u64, String)>,
    lost: &AtomicU32,
) -> Result<()> {
    if message.message == KEY_MESSAGE {
        write_key(writer, message)?;
    }
    while let Ok((time, text)) = pending.try_recv() {
        writeln!(writer, "t_ms={time} {text}")?;
    }
    let lost = lost.swap(0, Ordering::Relaxed);
    if lost != 0 {
        writeln!(writer, "t_ms={} dropped={lost}", millis())?;
    }
    writer.flush()?;
    Ok(())
}

fn write_key(writer: &mut impl Write, message: &MSG) -> std::io::Result<()> {
    let packed = message.wParam.0;
    let vk = packed & 0x1ff;
    let name = if vk == 0x100 {
        "other".into()
    } else {
        format!("0x{vk:02X}")
    };
    let action = hook::Action::from_raw((packed >> 12) & 7);
    writeln!(writer,
        "t_ms={} key vk={name} down={} injected={} swallowed={} action={action:?} ctrl_async={} scan=0x{:02X}",
        message.lParam.0, packed & 512 != 0, packed & 1024 != 0, packed & 2048 != 0,
        packed & (1 << 15) != 0, (packed >> 16) & 0xff)
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
