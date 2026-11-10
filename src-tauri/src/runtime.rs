use crate::{
    clipboard, diag,
    hook::{self, Action},
    model::ItemView,
    paster, platform,
    storage::Storage,
};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        mpsc::{self, Sender, SyncSender},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{DataExchange::*, LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

// 主窗口快捷键在这里配置，不需要额外快捷键插件。
pub const MAIN_HOTKEY_MODIFIERS: HOT_KEY_MODIFIERS = HOT_KEY_MODIFIERS(MOD_ALT.0 | MOD_NOREPEAT.0);
pub const MAIN_HOTKEY_VK: u32 = 0x56;
static BROKER: AtomicU32 = AtomicU32::new(0);
static LISTENER: AtomicIsize = AtomicIsize::new(0);
static WORK: OnceLock<SyncSender<Job>> = OnceLock::new();
static CONTROL: OnceLock<Sender<Control>> = OnceLock::new();
static CAPTURE_PENDING: AtomicBool = AtomicBool::new(false);
static LAST_SEQUENCE: AtomicU32 = AtomicU32::new(0);
static LAST_EXTERNAL: AtomicIsize = AtomicIsize::new(0);
pub enum Job {
    Capture,
    Paste {
        id: String,
        target: isize,
        restore: bool,
        plain: bool,
    },
}
pub enum Control {
    Hook(Action),
    Toggle,
    Hide,
    Report(String),
}
#[derive(Clone, Serialize, Default)]
pub struct Panel {
    pub items: Vec<ItemView>,
    pub selected: usize,
    pub visible: bool,
    pub error: Option<String>,
}
pub struct State {
    pub items: Mutex<Vec<ItemView>>,
    pub storage: Mutex<Storage>,
    pub jobs: SyncSender<Job>,
    pub control: Sender<Control>,
    pub target: AtomicIsize,
    pub main: AtomicIsize,
    pub quick: AtomicIsize,
    pub ready: AtomicBool,
    pub panel: Mutex<Panel>,
    pub error: Mutex<Option<String>>,
    pub smoke: bool,
    pub main_ready: AtomicBool,
    pub quick_ready: AtomicBool,
    pub thumbnail_ready: AtomicBool,
    pub preview_ready: AtomicBool,
}
pub fn report(_app: &AppHandle, state: &State, message: &str) {
    *state.error.lock().unwrap() = Some(message.into());
    let _ = state.control.send(Control::Report(message.into()));
}
pub fn refresh(app: &AppHandle, state: &State) -> Result<()> {
    let items = state.storage.lock().unwrap().list()?;
    let enabled = !items.is_empty() && state.ready.load(Ordering::Acquire);
    *state.items.lock().unwrap() = items;
    hook::READY.store(enabled, Ordering::Release);
    app.emit("history-changed", ())?;
    Ok(())
}
fn queue_capture() {
    if !CAPTURE_PENDING.swap(true, Ordering::AcqRel)
        && WORK
            .get()
            .is_none_or(|tx| tx.try_send(Job::Capture).is_err())
    {
        CAPTURE_PENDING.store(false, Ordering::Release);
    }
}
unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            queue_capture();
            LRESULT(0)
        }
        WM_HOTKEY => {
            if let Some(tx) = CONTROL.get() {
                let _ = tx.send(Control::Toggle);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            let current = platform::foreground();
            if platform::valid_external(current) {
                LAST_EXTERNAL.store(current, Ordering::Release);
            }
            if GetClipboardSequenceNumber() != LAST_SEQUENCE.load(Ordering::Acquire) {
                queue_capture();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, w, l),
    }
}
fn start_broker() -> Result<u32> {
    let (tx, rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("clipboard-listener".into())
        .spawn(move || unsafe {
            let init = (|| -> Result<(HWND, u32)> {
                let instance = GetModuleHandleW(None)?;
                let class = WNDCLASSW {
                    lpfnWndProc: Some(wndproc),
                    hInstance: instance.into(),
                    lpszClassName: w!("ClipboardM0Listener"),
                    ..Default::default()
                };
                if RegisterClassW(&class) == 0 {
                    return Err(anyhow!("监听窗口类注册失败"));
                }
                let h = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("ClipboardM0Listener"),
                    w!(""),
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    Some(HWND_MESSAGE),
                    None,
                    Some(instance.into()),
                    None,
                )?;
                AddClipboardFormatListener(h)?;
                RegisterHotKey(Some(h), 1, MAIN_HOTKEY_MODIFIERS, MAIN_HOTKEY_VK)
                    .map_err(|_| anyhow!("Alt+V 注册失败，可能已被其他程序占用"))?;
                SetTimer(Some(h), 1, 250, None);
                LISTENER.store(h.0 as isize, Ordering::Release);
                let id = GetCurrentThreadId();
                BROKER.store(id, Ordering::Release);
                Ok((h, id))
            })();
            match init {
                Ok((h, id)) => {
                    let _ = tx.send(Ok(id));
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                        if msg.message == hook::ACTION_MESSAGE {
                            if let Some(action) = Action::from_raw(msg.wParam.0) {
                                // 掩码必须先于 UI 调度，由 broker 尽快注入。
                                if action == Action::Open {
                                    if let Err(error) = paster::mask_shortcut() {
                                        if let Some(tx) = CONTROL.get() {
                                            let _ = tx.send(Control::Hook(Action::Cancel));
                                            let _ = tx.send(Control::Report(error.to_string()));
                                        }
                                        continue;
                                    }
                                }
                                if let Some(tx) = CONTROL.get() {
                                    let _ = tx.send(Control::Hook(action));
                                }
                            }
                        } else {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                    let _ = RemoveClipboardFormatListener(h);
                    let _ = UnregisterHotKey(Some(h), 1);
                    let _ = KillTimer(Some(h), 1);
                    let _ = DestroyWindow(h);
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        })?;
    rx.recv().map_err(|_| anyhow!("剪贴板监听线程启动失败"))?
}
fn on_main(app: &AppHandle, f: impl FnOnce() + Send + 'static) {
    let _ = app.run_on_main_thread(f);
}
fn publish(app: &AppHandle, state: &State, panel: &Panel) {
    *state.panel.lock().unwrap() = panel.clone();
    let _ = app.emit_to("quick", "panel-state", panel);
}
// 控制线程独占提示期限；新提示替换旧期限，重新打开面板会取消期限。
#[derive(Default)]
struct NoticeDeadline(Option<Instant>);
impl NoticeDeadline {
    fn reset(&mut self, now: Instant) {
        self.0 = Some(now + Duration::from_secs(2));
    }
    fn cancel(&mut self) {
        self.0 = None;
    }
    fn remaining(&self, now: Instant) -> Duration {
        self.0.map_or(Duration::from_secs(86400), |end| {
            end.saturating_duration_since(now)
        })
    }
    fn expire(&mut self, now: Instant) -> bool {
        if self.0.is_some_and(|end| now >= end) {
            self.cancel();
            true
        } else {
            false
        }
    }
}
pub fn start(app: AppHandle, storage: Storage, smoke: bool) -> Result<Arc<State>> {
    let (jobs, rx) = mpsc::sync_channel(32);
    let (control, controls) = mpsc::channel();
    let items = storage.list()?;
    let state = Arc::new(State {
        items: Mutex::new(items),
        storage: Mutex::new(storage),
        jobs: jobs.clone(),
        control: control.clone(),
        target: AtomicIsize::new(0),
        main: AtomicIsize::new(0),
        quick: AtomicIsize::new(0),
        ready: AtomicBool::new(false),
        panel: Mutex::new(Panel::default()),
        error: Mutex::new(None),
        smoke,
        main_ready: AtomicBool::new(false),
        quick_ready: AtomicBool::new(false),
        thumbnail_ready: AtomicBool::new(false),
        preview_ready: AtomicBool::new(false),
    });
    let worker = state.clone();
    let a = app.clone();
    thread::Builder::new()
        .name("clipboard-storage-worker".into())
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                match job {
                    Job::Capture => {
                        let sequence = unsafe { GetClipboardSequenceNumber() };
                        match clipboard::capture(platform::hwnd(LISTENER.load(Ordering::Acquire))) {
                            Ok(clip) => {
                                LAST_SEQUENCE.store(sequence, Ordering::Release);
                                if let Some(clip) = clip {
                                    let result = worker.storage.lock().unwrap().save(&clip);
                                    if result.is_err() {
                                        report(&a, &worker, "记录保存失败");
                                    } else if refresh(&a, &worker).is_err() {
                                        report(&a, &worker, "历史列表刷新失败");
                                    }
                                }
                            }
                            Err(_) => report(
                                &a,
                                &worker,
                                "采集失败：剪贴板被占用、格式无效或超过 128 MiB 限制",
                            ),
                        }
                        CAPTURE_PENDING.store(false, Ordering::Release);
                    }
                    Job::Paste {
                        id,
                        target,
                        restore,
                        plain,
                    } => {
                        diag::snapshot("before_wait");
                        let result = (|| -> Result<()> {
                            paster::wait_for_release()?;
                            let clip = worker.storage.lock().unwrap().load(&id)?;
                            if let Some(data) = clip.get("CF_HDROP") {
                                for p in crate::model::decode_hdrop(data)
                                    .ok_or_else(|| anyhow!("文件记录无效"))?
                                {
                                    if !std::path::Path::new(&p).exists() {
                                        return Err(anyhow!("源文件已失效，无法粘贴"));
                                    }
                                }
                            }
                            paster::prepare(target, worker.main.load(Ordering::Acquire), restore)?;
                            clipboard::write(
                                platform::hwnd(LISTENER.load(Ordering::Acquire)),
                                &clip,
                                plain,
                            )?;
                            paster::inject(target)?;
                            // 自写回不会重新采集，但成功粘贴仍应将已有记录置顶。
                            worker.storage.lock().unwrap().save(&clip)?;
                            refresh(&a, &worker)?;
                            Ok(())
                        })();
                        diag::snapshot(if result.is_ok() {
                            "paste_complete"
                        } else {
                            "paste_cancelled"
                        });
                        if let Err(e) = result {
                            report(&a, &worker, &e.to_string());
                        }
                    }
                }
            }
        })?;
    let s = state.clone();
    let a = app.clone();
    thread::Builder::new()
        .name("panel-controller".into())
        .spawn(move || {
            let mut panel = Panel::default();
            let mut target = 0;
            let mut notice = NoticeDeadline::default();
            loop {
                if notice.expire(Instant::now()) {
                    panel.error = None;
                    publish(&a, &s, &panel);
                    let _ = a.emit("operation-error", "");
                    if !panel.visible {
                        let quick = s.quick.load(Ordering::Acquire);
                        on_main(&a, move || platform::hide(quick));
                    }
                }
                let event = match controls.recv_timeout(notice.remaining(Instant::now())) {
                    Ok(event) => event,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                match event {
                    Control::Report(message) => {
                        if !panel.visible {
                            panel.items.clear();
                            panel.selected = 0;
                        }
                        panel.error = Some(message.clone());
                        publish(&a, &s, &panel);
                        let _ = a.emit("operation-error", &message);
                        notice.reset(Instant::now());
                        let shared = s.clone();
                        on_main(&a, move || {
                            // 错误提示跟随当前前台位置，以 NOACTIVATE 显示，不恢复旧目标焦点。
                            let _ = platform::show_notice(shared.quick.load(Ordering::Acquire));
                            let mut error = shared.error.lock().unwrap();
                            if error.as_deref() == Some(&message) {
                                *error = None;
                            }
                        });
                    }
                    Control::Toggle => {
                        notice.cancel();
                        panel.error = None;
                        let _ = a.emit("operation-error", "");
                        panel.visible = false;
                        diag::panel(true, false);
                        publish(&a, &s, &panel);
                        let current = platform::foreground();
                        let external = if platform::valid_external(current) {
                            current
                        } else {
                            LAST_EXTERNAL.load(Ordering::Acquire)
                        };
                        let app = a.clone();
                        let shared = s.clone();
                        on_main(&a, move || {
                            platform::hide(shared.quick.load(Ordering::Acquire));
                            if let Some(window) = app.get_webview_window("main") {
                                let visible = window.is_visible().unwrap_or(false);
                                let minimized = window.is_minimized().unwrap_or(false);
                                let foreground = platform::foreground() == shared.main.load(Ordering::Acquire);
                                let before = diag::enabled().then(diag::foreground);
                                let hide = toggle_decision(visible, minimized, foreground) == ToggleDecision::Hide;
                                if hide {
                                    let result = window.hide();
                                    diag::panel(false, result.is_err());
                                    diag::write(format!("toggle visible={visible} minimized={minimized} foreground={foreground} decision=hide before={before:?} hide_ok={} after={}", result.is_ok(), diag::foreground()));
                                } else {
                                    shared.target.store(external, Ordering::Release);
                                    let restored = window.unminimize();
                                    let shown = window.show();
                                    let raised = unsafe { SetWindowPos(platform::hwnd(shared.main.load(Ordering::Acquire)), Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
                                    let focused = window.set_focus();
                                    let foreground_ok = unsafe { SetForegroundWindow(platform::hwnd(shared.main.load(Ordering::Acquire))).as_bool() };
                                    diag::panel(false, shown.is_ok());
                                    diag::write(format!("toggle visible={visible} minimized={minimized} foreground={foreground} decision=show before={before:?} restore_ok={} show_ok={} raise_ok={} set_focus_ok={} set_foreground_ok={foreground_ok} after={}", restored.is_ok(), shown.is_ok(), raised.is_ok(), focused.is_ok(), diag::foreground()));
                                }
                            }
                        });
                    }
                    Control::Hide => {
                        let main = s.main.load(Ordering::Acquire);
                        on_main(&a, move || { platform::hide(main); diag::panel(false, false); });
                    }
                    Control::Hook(action) => match action {
                        Action::Open => {
                            target = platform::foreground();
                            if !platform::valid_external(target) {
                                diag::panel(true, false);
                                continue;
                            }
                            panel = Panel {
                                items: s.items.lock().unwrap().iter().take(9).cloned().collect(),
                                selected: 0,
                                visible: true,
                                error: None,
                            };
                            if panel.items.is_empty() {
                                panel.visible = false;
                                diag::panel(true, false);
                                continue;
                            }
                            notice.cancel();
                            let _ = a.emit("operation-error", "");
                            panel.selected = hook::initial_selection(panel.items.len());
                            publish(&a, &s, &panel);
                            let quick = s.quick.load(Ordering::Acquire);
                            let app = a.clone();
                            let shared = s.clone();
                            on_main(&a, move || {
                                if platform::show_quick(quick, target).is_err() {
                                    let _ = shared.control.send(Control::Hook(Action::Cancel));
                                    report(&app, &shared, "前台窗口已变化，面板已取消");
                                }
                            });
                        }
                        Action::Next | Action::Previous if panel.visible => {
                            panel.selected = hook::move_selection(
                                panel.selected,
                                panel.items.len(),
                                action == Action::Next,
                            );
                            publish(&a, &s, &panel);
                        }
                        Action::Commit | Action::Cancel => {
                            let paste = action == Action::Commit && panel.visible;
                            panel.visible = false;
                            diag::panel(true, false);
                            publish(&a, &s, &panel);
                            let quick = s.quick.load(Ordering::Acquire);
                            if panel.error.is_none() {
                                on_main(&a, move || platform::hide(quick));
                            }
                            if paste {
                                if let Some(item) = panel.items.get(panel.selected) {
                                    if s.jobs
                                        .try_send(Job::Paste {
                                            id: item.id.clone(),
                                            target,
                                            restore: false,
                                            plain: false,
                                        })
                                        .is_err()
                                    {
                                        report(&a, &s, "任务队列繁忙，请重试");
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
        })?;
    WORK.set(jobs).map_err(|_| anyhow!("工作线程已启动"))?;
    CONTROL
        .set(control)
        .map_err(|_| anyhow!("控制线程已启动"))?;
    if !smoke {
        clipboard::initialize()?;
        let receiver = start_broker()?;
        hook::start(receiver)?;
    }
    Ok(state)
}
#[derive(Debug, PartialEq, Eq)]
enum ToggleDecision {
    Hide,
    Show,
}
fn toggle_decision(visible: bool, minimized: bool, foreground: bool) -> ToggleDecision {
    if visible && !minimized && foreground {
        ToggleDecision::Hide
    } else {
        ToggleDecision::Show
    }
}
pub fn stop() {
    hook::stop();
    unsafe {
        let id = BROKER.load(Ordering::Acquire);
        if id != 0 {
            let _ = PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggle_only_hides_a_visible_restored_foreground_window() {
        for visible in [false, true] {
            for minimized in [false, true] {
                for foreground in [false, true] {
                    let actual = toggle_decision(visible, minimized, foreground);
                    assert_eq!(
                        actual,
                        if (visible, minimized, foreground) == (true, false, true) {
                            ToggleDecision::Hide
                        } else {
                            ToggleDecision::Show
                        }
                    );
                }
            }
        }
    }
    #[test]
    fn error_notice_expires_once_after_two_seconds() {
        let now = Instant::now();
        let mut notice = NoticeDeadline::default();
        notice.reset(now);
        assert!(!notice.expire(now + Duration::from_millis(1999)));
        assert!(notice.expire(now + Duration::from_secs(2)));
        assert!(!notice.expire(now + Duration::from_secs(3)));
    }
    #[test]
    fn new_error_replaces_old_deadline_and_open_cancels_timeout() {
        let now = Instant::now();
        let mut notice = NoticeDeadline::default();
        notice.reset(now);
        notice.reset(now + Duration::from_secs(1));
        assert!(!notice.expire(now + Duration::from_secs(2)));
        assert_eq!(
            notice.remaining(now + Duration::from_secs(2)),
            Duration::from_secs(1)
        );
        notice.cancel();
        assert!(!notice.expire(now + Duration::from_secs(3)));
    }
    #[test]
    fn notice_without_deadline_never_expires_or_shortens() {
        // 没有待显示的提示时，控制线程把 remaining() 当作 recv_timeout 的超时值。
        // 若默认值退化成 0 秒，控制线程会忙循环；这里固定住“无期限”的语义。
        let now = Instant::now();
        let mut notice = NoticeDeadline::default();
        assert!(notice.remaining(now) >= Duration::from_secs(3600));
        assert!(
            notice.remaining(now + Duration::from_secs(3600)) >= Duration::from_secs(3600),
            "未设置期限时不应随经过时间缩短"
        );
        assert!(!notice.expire(now + Duration::from_secs(86_400)));
        assert!(
            notice.remaining(now) >= Duration::from_secs(3600),
            "expire 不应给未设置期限加出期限"
        );
    }
    #[test]
    fn remaining_counts_down_after_reset_and_is_never_negative() {
        let now = Instant::now();
        let mut notice = NoticeDeadline::default();
        notice.reset(now);
        assert!(notice.remaining(now) <= Duration::from_secs(2));
        assert!(notice.remaining(now) > Duration::from_millis(1900));
        // 期限已过但尚未处理时，remaining 必须饱和为 0 而不是 panic（Instant 相减会溢出）。
        assert_eq!(
            notice.remaining(now + Duration::from_secs(9)),
            Duration::ZERO
        );
    }
}
