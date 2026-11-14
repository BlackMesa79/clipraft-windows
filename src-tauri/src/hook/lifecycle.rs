//! 钩子线程的启动回复、消息循环与原生资源释放；不在键盘回调路径上。
use super::*;
use anyhow::Context;
use std::sync::mpsc::{self, SyncSender};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

struct Registration(HHOOK);

impl Registration {
    fn install() -> Result<Self> {
        let module = match unsafe { GetModuleHandleW(None) } {
            Ok(module) => Some(module.into()),
            Err(error) => {
                crate::app_events::record("hook module lookup failed", Err::<(), _>(error));
                None
            }
        };
        let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(event::callback), module, 0) }
            .context("键盘钩子安装失败")?;
        Ok(Self(hook))
    }

    fn publish_thread(&self) {
        let mut message = MSG::default();
        // PeekMessage 的 false 只表示队列为空；调用本身确保线程消息队列已经建立。
        unsafe {
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
        };
        THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        crate::app_events::record(
            "hook removal failed",
            release_thread(&THREAD, || {
                unsafe { UnhookWindowsHookEx(self.0) }.context("键盘钩子释放失败")
            }),
        );
    }
}

pub fn start(receiver: u32) -> Result<()> {
    RECEIVER.store(receiver, Ordering::Release);
    let (reply, wait) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(move || run(reply))
        .context("键盘线程创建失败")?;
    wait.recv().map_err(|_| anyhow!("键盘线程启动失败"))?
}

fn seed_state() {
    let mut bits = 0;
    for (index, key) in MODIFIER_KEYS.iter().enumerate() {
        if unsafe { GetAsyncKeyState(*key as i32) } < 0 {
            bits |= 1 << index;
        }
    }
    PHYSICAL.store(bits, Ordering::SeqCst);
    MACHINE.with(|slot| {
        let mut state = slot.get();
        state.seed(bits);
        slot.set(state);
    });
}

fn run(reply: SyncSender<Result<()>>) {
    seed_state();
    let registration = match Registration::install() {
        Ok(registration) => registration,
        Err(error) => {
            crate::app_events::record("hook startup error reply failed", reply.send(Err(error)));
            return;
        }
    };
    registration.publish_thread();
    crate::app_events::record(
        "hook thread failed",
        serve(reply, registration, || message_loop(read_message, dispatch)),
    );
}

fn serve<R>(
    reply: SyncSender<Result<()>>,
    _registration: R,
    messages: impl FnOnce() -> Result<()>,
) -> Result<()> {
    reply
        .send(Ok(()))
        .map_err(|_| anyhow!("键盘线程启动回复接收方已退出"))?;
    messages()
}

fn read_message(message: &mut MSG) -> Result<bool> {
    match unsafe { GetMessageW(message, None, 0, 0) }.0 {
        -1 => Err(windows::core::Error::from_win32()).context("键盘消息读取失败"),
        0 => Ok(false),
        _ => Ok(true),
    }
}

fn message_loop(
    mut read: impl FnMut(&mut MSG) -> Result<bool>,
    mut dispatch: impl FnMut(&MSG),
) -> Result<()> {
    let mut message = MSG::default();
    while read(&mut message)? {
        dispatch(&message);
    }
    Ok(())
}

fn dispatch(message: &MSG) {
    if handle_thread_message(message.message) {
        return;
    }
    unsafe {
        // TranslateMessage=false 表示没有生成字符消息，不是消息处理失败。
        let _ = TranslateMessage(message);
        DispatchMessageW(message);
    }
}

fn release_thread(thread_id: &AtomicU32, remove: impl FnOnce() -> Result<()>) -> Result<()> {
    thread_id.store(0, Ordering::Release);
    remove()
}

pub fn stop() {
    READY.store(false, Ordering::Release);
    crate::app_events::record(
        "hook quit post failed",
        post_thread(THREAD.load(Ordering::Acquire), WM_QUIT),
    );
}

pub(super) fn post_thread(thread_id: u32, message: u32) -> windows::core::Result<()> {
    post_if_running(thread_id, message, |id, message| unsafe {
        PostThreadMessageW(id, message, WPARAM(0), LPARAM(0))
    })
}

fn post_if_running(
    thread_id: u32,
    message: u32,
    post: impl FnOnce(u32, u32) -> windows::core::Result<()>,
) -> windows::core::Result<()> {
    // id=0 会投递到调用线程自身，不能作为“未启动”的退出目标。
    if thread_id == 0 {
        return Ok(());
    }
    post(thread_id, message)
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
