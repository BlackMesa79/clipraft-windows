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
        atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering},
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
use clipboard::LAST_SEQUENCE;
static LAST_EXTERNAL: AtomicIsize = AtomicIsize::new(0);
static PASTE_GENERATION: AtomicU64 = AtomicU64::new(0);
pub fn paste_generation() -> u64 {
    PASTE_GENERATION.load(Ordering::Acquire)
}
pub fn new_paste_intent() -> u64 {
    PASTE_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1)
}
pub enum Job {
    Capture,
    Paste {
        id: String,
        target: isize,
        restore: bool,
        plain: bool,
        generation: u64,
    },
}
pub enum Control {
    Hook(Action, u64),
    Toggle,
    Hide,
    Report(String),
    PasteReport(String, u64),
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
    let items = state.storage.lock().unwrap().list_page(
        &crate::storage::ListFilter::default(),
        0,
        crate::storage::QUICK_PAGE_SIZE,
    )?;
    let enabled = !items.is_empty() && state.ready.load(Ordering::Acquire);
    *state.items.lock().unwrap() = items;
    hook::READY.store(enabled, Ordering::Release);
    app.emit("history-changed", ())?;
    Ok(())
}
fn queue_capture() {
    if unsafe { GetClipboardSequenceNumber() } == LAST_SEQUENCE.load(Ordering::Acquire) {
        return;
    }
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
            // 先掩码再调度 Toggle，避免 Alt 松开后进入菜单模式。
            let masked = paster::mask_shortcut();
            diag::write(format!("hotkey mask_ok={}", masked.is_ok()));
            diag::modifier_snapshot("toggle", "hotkey_after_mask");
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
                                let generation = msg.lParam.0 as u64;
                                if generation != paste_generation() {
                                    continue;
                                }
                                // 掩码必须先于 UI 调度，由 broker 尽快注入。
                                if action == Action::Open {
                                    if let Err(error) = paster::mask_shortcut() {
                                        if let Some(tx) = CONTROL.get() {
                                            let _ =
                                                tx.send(Control::Hook(Action::Cancel, generation));
                                            let _ = tx.send(Control::Report(error.to_string()));
                                        }
                                        continue;
                                    }
                                }
                                if let Some(tx) = CONTROL.get() {
                                    let _ = tx.send(Control::Hook(action, generation));
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
    let items = storage.list_page(
        &crate::storage::ListFilter::default(),
        0,
        crate::storage::QUICK_PAGE_SIZE,
    )?;
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
                        match clipboard::capture_pending(platform::hwnd(
                            LISTENER.load(Ordering::Acquire),
                        )) {
                            Ok(clip) => {
                                if let Some(clip) = clip {
                                    let result = worker.storage.lock().unwrap().save(&clip);
                                    if result.is_err() {
                                        report(&a, &worker, "记录保存失败");
                                    } else if refresh(&a, &worker).is_err() {
                                        report(&a, &worker, "历史列表刷新失败");
                                    }
                                }
                            }
                            Err(error) => report(&a, &worker, &error.to_string()),
                        }
                        CAPTURE_PENDING.store(false, Ordering::Release);
                        // 等待期间的变化可能被合并，补排尚未处理的新序列。
                        queue_capture();
                    }
                    Job::Paste {
                        id,
                        target,
                        restore,
                        plain,
                        generation,
                    } => {
                        let intent = paster::PasteIntent::new(&PASTE_GENERATION, generation);
                        let mut clip = None;
                        let result = paster::run_paste(&intent, |stage| {
                            match stage {
                                paster::PasteStage::Load => {
                                    let loaded = worker.storage.lock().unwrap().load(&id)?;
                                    for format in &loaded.formats {
                                        // 仅输出固定支持格式的名称和大小，不记录内容或记录标识。
                                        if clipboard::SUPPORTED
                                            .iter()
                                            .any(|(name, _)| *name == format.name)
                                        {
                                            diag::write(format!(
                                                "paste generation={generation} format={} bytes={}",
                                                format.name,
                                                format.bytes.len()
                                            ));
                                        }
                                    }
                                    clip = Some(loaded);
                                }
                                paster::PasteStage::Write => {
                                    let clip =
                                        clip.as_ref().ok_or_else(|| anyhow!("粘贴记录尚未加载"))?;
                                    validate_sources(clip, |p| std::path::Path::new(p).exists())?;
                                    clipboard::write(
                                        platform::hwnd(LISTENER.load(Ordering::Acquire)),
                                        clip,
                                        plain,
                                    )?;
                                }
                                paster::PasteStage::Wait => {
                                    diag::snapshot("before_wait");
                                    return paster::wait_for_release(&intent);
                                }
                                paster::PasteStage::Prepare => {
                                    paster::prepare(
                                        target,
                                        worker.main.load(Ordering::Acquire),
                                        restore,
                                    )?;
                                }
                                paster::PasteStage::Inject => {
                                    return paster::inject(target, &intent)
                                }
                                paster::PasteStage::Touch => {
                                    worker.storage.lock().unwrap().touch(&id)?
                                }
                            }
                            Ok(paster::PasteStatus::Complete)
                        });
                        diag::snapshot(match &result {
                            Ok(paster::PasteStatus::Complete) => "paste_complete",
                            Ok(paster::PasteStatus::Superseded) => "paste_superseded",
                            Err(_) => "paste_cancelled",
                        });
                        let result = result.and_then(|status| {
                            if status == paster::PasteStatus::Complete {
                                refresh(&a, &worker)?;
                            }
                            Ok(())
                        });
                        if let Err(e) = result {
                            if intent.is_current() {
                                let _ = worker
                                    .control
                                    .send(Control::PasteReport(e.to_string(), generation));
                            }
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
                    Control::Report(_) | Control::PasteReport(_, _) => {
                        let (message, generation) = match event {
                            Control::Report(message) => (message, None),
                            Control::PasteReport(message, generation) => (message, Some(generation)),
                            _ => unreachable!(),
                        };
                        if generation.is_some_and(|g| g != paste_generation()) { continue; }
                        *s.error.lock().unwrap() = Some(message.clone());
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
                            if generation.is_some_and(|g| g != paste_generation()) { return; }
                            // 错误提示跟随当前前台位置，以 NOACTIVATE 显示，不恢复旧目标焦点。
                            let _ = platform::show_notice(shared.quick.load(Ordering::Acquire));
                            let mut error = shared.error.lock().unwrap();
                            if error.as_deref() == Some(&message) {
                                *error = None;
                            }
                        });
                    }
                    Control::Toggle => {
                        *s.error.lock().unwrap() = None;
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
                                    new_paste_intent();
                                    *shared.error.lock().unwrap() = None;
                                    shared.target.store(external, Ordering::Release);
                                    diag::modifier_snapshot("toggle", "before_show");
                                    platform::arm_main_menu_guard();
                                    let restored = window.unminimize();
                                    let shown = window.show();
                                    let raised = unsafe { SetWindowPos(platform::hwnd(shared.main.load(Ordering::Acquire)), Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
                                    // tao 的 set_focus 失败回退会注入 Alt，这里只使用原生激活。
                                    let foreground_ok = unsafe { SetForegroundWindow(platform::hwnd(shared.main.load(Ordering::Acquire))).as_bool() };
                                    // 显示过程也受保护，完成后再保留完整的 300ms 时窗。
                                    platform::arm_main_menu_guard();
                                    diag::modifier_snapshot("toggle", "after_show");
                                    diag::panel(false, shown.is_ok());
                                    diag::write(format!("toggle visible={visible} minimized={minimized} foreground={foreground} decision=show before={before:?} restore_ok={} show_ok={} raise_ok={} set_foreground_ok={foreground_ok} after={}", restored.is_ok(), shown.is_ok(), raised.is_ok(), diag::foreground()));
                                }
                            }
                        });
                    }
                    Control::Hide => {
                        let main = s.main.load(Ordering::Acquire);
                        on_main(&a, move || { platform::hide(main); diag::panel(false, false); });
                    }
                    Control::Hook(action, generation) => {
                        // 消息携带产生时的代号，旧 Commit 不能借用新 Open 的代号。
                        if generation != paste_generation() { continue; }
                        match action {
                        Action::Open => {
                            *s.error.lock().unwrap() = None;
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
                                if generation != paste_generation() { return; }
                                if platform::show_quick(quick, target).is_err() {
                                    let _ = shared.control.send(Control::Hook(Action::Cancel, generation));
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
                                            generation,
                                        })
                                        .is_err()
                                    {
                                        report(&a, &s, "任务队列繁忙，请重试");
                                    }
                                }
                            }
                        }
                        _ => {}
                        }
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
fn validate_sources(clip: &crate::model::Clip, mut exists: impl FnMut(&str) -> bool) -> Result<()> {
    if let Some(data) = clip.get("CF_HDROP") {
        for path in crate::model::decode_hdrop(data).ok_or_else(|| anyhow!("文件记录无效"))? {
            if !exists(&path) {
                return Err(anyhow!("源文件已失效，无法粘贴"));
            }
        }
    }
    Ok(())
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

#[cfg(test)]
mod source_tests {
    use super::*;
    use crate::model::{encode_hdrop, Clip, FormatData};
    #[test]
    fn file_sources_are_all_checked_before_write() {
        let clip = Clip {
            formats: vec![FormatData {
                name: "CF_HDROP".into(),
                bytes: encode_hdrop(&["first".into(), "missing".into()]),
            }],
        };
        let mut checked = Vec::new();
        let error = validate_sources(&clip, |p| {
            checked.push(p.to_string());
            p != "missing"
        })
        .unwrap_err();
        assert_eq!(checked, ["first", "missing"]);
        assert_eq!(error.to_string(), "源文件已失效，无法粘贴");
        assert!(validate_sources(&clip, |_| true).is_ok());
    }
    #[test]
    fn malformed_file_record_fails_and_non_file_record_needs_no_paths() {
        let clip = Clip {
            formats: vec![FormatData {
                name: "CF_HDROP".into(),
                bytes: vec![0; 3],
            }],
        };
        assert!(validate_sources(&clip, |_| panic!("无效格式不应访问路径")).is_err());
        assert!(validate_sources(&Clip::default(), |_| panic!("非文件记录无需检查路径")).is_ok());
    }
}
