use crate::{
    clipboard, diag,
    hook::{self, Action},
    model::ItemView,
    paster, platform,
    settings::{self, Hotkey, HotkeyRegistrar, Settings},
    storage::{RetentionPolicy, Storage},
};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::{
    collections::HashSet,
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

const SETTINGS_MESSAGE: u32 = WM_APP + 81;
struct BrokerRequest {
    hotkey: Hotkey,
    reply: Sender<Result<(), String>>,
    cancelled: Arc<AtomicBool>,
}
static HOTKEY_REQUESTS: OnceLock<Sender<BrokerRequest>> = OnceLock::new();
struct NativeRegistrar(HWND);
impl HotkeyRegistrar for NativeRegistrar {
    fn unregister(&mut self) -> Result<()> {
        unsafe {
            UnregisterHotKey(Some(self.0), 1).map_err(|e| anyhow!("旧快捷键注销失败：{e}"))
        }
    }
    fn register(&mut self, hotkey: Hotkey) -> Result<()> {
        let shortcut = hotkey.shortcut();
        let mut flags = MOD_NOREPEAT.0;
        for (bit, flag) in [(1, MOD_CONTROL), (2, MOD_SHIFT), (4, MOD_ALT), (8, MOD_WIN)] {
            if shortcut.modifiers & bit != 0 {
                flags |= flag.0;
            }
        }
        unsafe {
            RegisterHotKey(Some(self.0), 1, HOT_KEY_MODIFIERS(flags), shortcut.key)?;
        }
        Ok(())
    }
}
fn change_main_hotkey(hotkey: Hotkey) -> Result<()> {
    let (reply, wait) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    HOTKEY_REQUESTS
        .get()
        .ok_or_else(|| anyhow!("热键线程未启动"))?
        .send(BrokerRequest {
            hotkey,
            reply,
            cancelled: cancelled.clone(),
        })
        .map_err(|_| anyhow!("热键线程已退出"))?;
    if let Err(error) = unsafe {
        PostThreadMessageW(
            BROKER.load(Ordering::Acquire),
            SETTINGS_MESSAGE,
            WPARAM(0),
            LPARAM(0),
        )
    } {
        cancelled.store(true, Ordering::Release);
        return Err(error.into());
    }
    wait.recv()
        .map_err(|_| anyhow!("热键线程已退出"))?
        .map_err(anyhow::Error::msg)
}
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
    HistoryChanged,
    UpdateSettings(Settings, Sender<Result<Settings, String>>),
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
    // 删除、保存与清理统一先锁此上下文，再锁 storage。
    pub capture_context: Mutex<Option<crate::owner_flush::CaptureContext>>,
    pub retention_policy: Mutex<RetentionPolicy>,
    pub settings: Mutex<Settings>,
    pub startup_error: Mutex<Option<String>>,
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
    let storage = state.storage.lock().unwrap();
    let items = storage.list()?;
    let enabled = !state.smoke && !items.is_empty() && state.ready.load(Ordering::Acquire);
    *state.items.lock().unwrap() = items;
    hook::READY.store(enabled, Ordering::Release);
    drop(storage);
    let _ = state.control.send(Control::HistoryChanged);
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
            crate::clipboard_diag::clipboard_update();
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
fn start_broker(initial: Hotkey) -> Result<(u32, Option<String>)> {
    let (requests, pending) = mpsc::channel::<BrokerRequest>();
    HOTKEY_REQUESTS
        .set(requests)
        .map_err(|_| anyhow!("热键控制通道已启动"))?;
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

                SetTimer(Some(h), 1, 250, None);
                LISTENER.store(h.0 as isize, Ordering::Release);
                let id = GetCurrentThreadId();
                BROKER.store(id, Ordering::Release);
                Ok((h, id))
            })();
            match init {
                Ok((h, id)) => {
                    let mut registrar = NativeRegistrar(h);
                    let mut registered = None;
                    let startup_error =
                        settings::replace_hotkey(&mut registrar, &mut registered, initial)
                            .err()
                            .map(|e| format!("{}：{e}。请在设置中更换快捷键。", initial.label()));
                    if let Some(error) = &startup_error {
                        diag::write(format!("startup hotkey: {error}"));
                    }
                    let _ = tx.send(Ok((id, startup_error)));
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                        if msg.message == SETTINGS_MESSAGE {
                            while let Ok(request) = pending.try_recv() {
                                if request.cancelled.load(Ordering::Acquire) {
                                    continue;
                                }
                                let result = settings::replace_hotkey(
                                    &mut registrar,
                                    &mut registered,
                                    request.hotkey,
                                )
                                .map_err(|e| e.to_string());
                                let _ = request.reply.send(result);
                            }
                        } else if msg.message == hook::ACTION_MESSAGE {
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
// 选中记录消失时取消，绝不把原下标解释为另一条记录。
fn refresh_panel_snapshot(
    snapshot: &[ItemView],
    existing_ids: &HashSet<String>,
    selected_id: Option<&str>,
) -> Option<(Vec<ItemView>, usize)> {
    let selected_id = selected_id?;
    let items: Vec<_> = snapshot
        .iter()
        .filter(|item| existing_ids.contains(&item.id))
        .cloned()
        .collect();
    let selected = items.iter().position(|item| item.id == selected_id)?;
    Some((items, selected))
}

fn hide_panel(app: &AppHandle, state: &State, panel: &mut Panel) {
    hook::cancel_panel();
    panel.visible = false;
    diag::panel(true, false);
    publish(app, state, panel);
    if panel.error.is_none() {
        let quick = state.quick.load(Ordering::Acquire);
        on_main(app, move || platform::hide(quick));
    }
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
/// 只在新记录插入后调用；时长使用单调时钟，避免系统时间调整影响节流。
fn cleanup_due(elapsed: Duration, inserted: u32) -> bool {
    inserted > 0 && (elapsed >= Duration::from_secs(30) || inserted >= 20)
}

/// 指纹在所有共享锁之外计算。成功保存（包括既有记录）之后才更新上下文。
pub(crate) fn save_capture(
    storage: &Mutex<Storage>,
    context: &Mutex<Option<crate::owner_flush::CaptureContext>>,
    mut captured: clipboard::Captured,
) -> Result<Option<bool>> {
    captured.context.fingerprint =
        crate::owner_flush::fingerprint(&captured.clip, captured.context.bitmap_offered);
    // 同时需要两把锁时一律先 capture_context 后 storage，与删除/自动清理一致。
    let mut previous = context.lock().unwrap();
    if crate::owner_flush::should_skip(previous.as_ref(), &captured.context) {
        diag::write(format!(
            "capture sequence={} result=skipped reason=owner_flush",
            captured.sequence
        ));
        return Ok(None);
    }
    if captured.context.owner.is_ownerless() {
        if let Some(reason) = crate::owner_flush::miss_reason(previous.as_ref(), &captured.context)
        {
            diag::write(format!(
                "capture owner_flush_miss sequence={} reason={}",
                captured.sequence,
                reason.as_str()
            ));
        }
    }
    let outcome =
        Storage::save_shared_with_source(storage, &captured.clip, Some(&captured.source))?;
    captured.context.record_id = Some(outcome.id);
    *previous = Some(captured.context);
    Ok(Some(outcome.inserted))
}

pub fn start(app: AppHandle, mut storage: Storage, smoke: bool) -> Result<Arc<State>> {
    let settings = storage.load_settings()?;
    let policy = settings.retention_policy();
    hook::configure(settings.quick_hotkey.shortcut());
    let vacuumed = match storage.cleanup(&policy, crate::storage::unix_millis()) {
        Ok(report) => report.vacuumed,
        Err(e) => {
            diag::write(format!("startup cleanup failed: {e}"));
            false
        }
    };
    if !vacuumed {
        if let Err(e) = storage.vacuum_on_startup() {
            diag::write(format!("startup vacuum failed: {e}"));
        }
    }
    let (jobs, rx) = mpsc::sync_channel(32);
    let (control, controls) = mpsc::channel();
    let items = storage.list()?;
    let state = Arc::new(State {
        items: Mutex::new(items),
        storage: Mutex::new(storage),
        capture_context: Mutex::new(None),
        retention_policy: Mutex::new(policy),
        settings: Mutex::new(settings.clone()),
        startup_error: Mutex::new(None),
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
            let mut last_cleanup = Instant::now();
            let mut inserted_since_cleanup = 0u32;
            while let Ok(job) = rx.recv() {
                match job {
                    Job::Capture => {
                        match clipboard::capture_pending_with_settings(
                            platform::hwnd(LISTENER.load(Ordering::Acquire)),
                            || worker.settings.lock().unwrap().ignored_apps.clone(),
                        ) {
                            Ok(clip) => {
                                if let Some(clip) = clip {
                                    let result = save_capture(
                                        &worker.storage,
                                        &worker.capture_context,
                                        clip,
                                    );
                                    match result {
                                        Err(_) => report(&a, &worker, "记录保存失败"),
                                        Ok(None) => {}
                                        Ok(Some(inserted)) => {
                                            if inserted {
                                                inserted_since_cleanup =
                                                    inserted_since_cleanup.saturating_add(1);
                                                if cleanup_due(
                                                    last_cleanup.elapsed(),
                                                    inserted_since_cleanup,
                                                ) {
                                                    let policy =
                                                        worker.retention_policy.lock().unwrap();
                                                    // 与保存/手动删除一致：先上下文锁，后存储锁；禁止反向嵌套。
                                                    let mut context =
                                                        worker.capture_context.lock().unwrap();
                                                    let result =
                                                        worker.storage.lock().unwrap().cleanup(
                                                            &policy,
                                                            crate::storage::unix_millis(),
                                                        );
                                                    if let Ok(report) = &result {
                                                        crate::owner_flush::invalidate_deleted(
                                                            &mut context,
                                                            &report.deleted_ids,
                                                        );
                                                    }
                                                    drop(context);
                                                    if let Err(e) = result {
                                                        diag::write(format!(
                                                            "capture cleanup failed: {e}"
                                                        ));
                                                    }
                                                    // 失败也节流，避免故障时每次采集重复做昂贵维护。
                                                    last_cleanup = Instant::now();
                                                    inserted_since_cleanup = 0;
                                                }
                                            }
                                            // 保存与清理共用一次刷新；去重更新仍需要重排列表。
                                            if refresh(&a, &worker).is_err() {
                                                report(&a, &worker, "历史列表刷新失败");
                                            }
                                        }
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
                    Control::UpdateSettings(settings, reply) => {
                        let result = apply_settings(&a, &s, settings, panel.visible).map_err(|e| e.to_string());
                        let _ = reply.send(result);
                    }
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
                        hook::cancel_panel();
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
                                    if shown.is_ok() { let _ = window.emit("main-shown", ()); }
                                    diag::panel(false, shown.is_ok());
                                    diag::write(format!("toggle visible={visible} minimized={minimized} foreground={foreground} decision=show before={before:?} restore_ok={} show_ok={} raise_ok={} set_foreground_ok={foreground_ok} after={}", restored.is_ok(), shown.is_ok(), raised.is_ok(), diag::foreground()));
                                }
                            }
                        });
                    }
                    Control::HistoryChanged => {
                        if panel.visible {
                            let ids: Vec<_> = panel.items.iter().map(|item| item.id.as_str()).collect();
                            let existing = s.storage.lock().unwrap().existing_panel_ids(&ids);
                            let refreshed = match existing {
                                Ok(existing) => refresh_panel_snapshot(
                                    &panel.items,
                                    &existing,
                                    panel.items.get(panel.selected).map(|item| item.id.as_str()),
                                ),
                                Err(error) => {
                                    diag::write(format!("panel snapshot lookup failed: {error}"));
                                    None
                                }
                            };
                            if let Some((items, selected)) = refreshed {
                                panel.items = items;
                                panel.selected = selected;
                                publish(&a, &s, &panel);
                            } else {
                                // 查询失败或选中项被删除都取消；后续 Commit 无可粘贴的记录。
                                panel.items.clear();
                                panel.selected = 0;
                                panel.error = None;
                                notice.cancel();
                                hide_panel(&a, &s, &mut panel);
                            }
                        }
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
                                hook::cancel_panel();
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
                                hook::cancel_panel();
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
                            hide_panel(&a, &s, &mut panel);
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
        let (receiver, error) = start_broker(settings.main_hotkey)?;
        *state.startup_error.lock().unwrap() = error;
        hook::start(receiver)?;
    }
    Ok(state)
}
// 控制通道串行化设置与面板动作；数据库等待不会占用 broker 或 hook 线程。
fn apply_settings(
    app: &AppHandle,
    state: &State,
    next: Settings,
    panel_visible: bool,
) -> Result<Settings> {
    let next = next.normalized()?;
    let mut current = state.settings.lock().unwrap();
    let hotkeys_changed =
        current.main_hotkey != next.main_hotkey || current.quick_hotkey != next.quick_hotkey;
    let retry_hotkey = state.startup_error.lock().unwrap().is_some();
    let reservation = if hotkeys_changed || retry_hotkey {
        if panel_visible {
            return Err(anyhow!("快速面板正在激活，请关闭面板后重试"));
        }
        Some(hook::reserve_configuration_when_hidden()?)
    } else {
        None
    };
    let change_main = current.main_hotkey != next.main_hotkey || retry_hotkey;
    if change_main {
        if let Err(error) = change_main_hotkey(next.main_hotkey) {
            if error.to_string().contains("旧快捷键恢复失败") {
                *state.startup_error.lock().unwrap() = Some(error.to_string());
                diag::write(error.to_string());
                let _ = app.emit("startup-error", error.to_string());
            }
            return Err(error);
        }
    }
    // 持有策略锁直到清理完成，避免 worker 用已失效的旧策略再清一次。
    let mut policy = state.retention_policy.lock().unwrap();
    let mut context = state.capture_context.lock().unwrap();
    let mut storage = state.storage.lock().unwrap();
    if let Err(error) = storage.save_settings(&next) {
        if change_main {
            if let Err(rollback) = change_main_hotkey(current.main_hotkey) {
                let message = format!("设置未保存：{error}；热键恢复失败：{rollback}");
                diag::write(message.clone());
                *state.startup_error.lock().unwrap() = Some(message.clone());
                let _ = app.emit("startup-error", &message);
                return Err(anyhow!(message));
            }
        }
        return Err(anyhow!("设置未保存：{error}"));
    }
    *policy = next.retention_policy();
    hook::configure(next.quick_hotkey.shortcut());
    *current = next.clone();
    drop(current);
    drop(reservation);
    if change_main {
        *state.startup_error.lock().unwrap() = None;
        let _ = app.emit("startup-error", "");
    }
    match storage.cleanup(&policy, crate::storage::unix_millis()) {
        Ok(report) => {
            crate::owner_flush::invalidate_deleted(&mut context, &report.deleted_ids);
            if !report.failed_blobs.is_empty() || !report.maintenance_errors.is_empty() {
                let message =
                    "设置已保存，记录已清理，但部分文件回收或数据库整理失败，将在后续维护重试";
                diag::write(message.into());
                let _ = app.emit("operation-error", message);
            }
        }
        Err(error) => {
            let message = format!("设置已保存，但立即清理失败：{error}");
            diag::write(message.clone());
            let _ = app.emit("operation-error", message);
        }
    }
    drop(storage);
    drop(context);
    drop(policy);
    let _ = app.emit("settings-changed", &next);
    if let Err(error) = refresh(app, state) {
        diag::write(format!("settings refresh failed: {error}"));
    }
    Ok(next)
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
    fn snapshot() -> Vec<ItemView> {
        ["a", "b", "c"]
            .into_iter()
            .map(|id| ItemView {
                id: id.into(),
                summary: format!("快照内容 {id}"),
                ..Default::default()
            })
            .collect()
    }
    fn existing(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).into()).collect()
    }
    #[test]
    fn snapshot_cancels_when_selected_item_is_deleted() {
        assert!(refresh_panel_snapshot(&snapshot(), &existing(&["a", "c"]), Some("b")).is_none());
    }
    #[test]
    fn snapshot_preserves_selected_id_when_other_items_are_deleted() {
        let (items, selected) =
            refresh_panel_snapshot(&snapshot(), &existing(&["b", "c"]), Some("b")).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert_eq!(selected, 0);
        assert_eq!(items[selected].id, "b");
        assert_eq!(items[selected].summary, "快照内容 b");
    }
    #[test]
    fn snapshot_does_not_add_new_items_or_reorder_existing_items() {
        let (items, selected) =
            refresh_panel_snapshot(&snapshot(), &existing(&["new", "c", "b", "a"]), Some("b"))
                .unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(selected, 1);
    }
    #[test]
    fn snapshot_cancels_when_all_items_are_deleted() {
        assert!(refresh_panel_snapshot(&snapshot(), &HashSet::new(), Some("b")).is_none());
    }
    #[test]
    fn snapshot_cancels_when_selection_is_missing() {
        assert!(refresh_panel_snapshot(&snapshot(), &existing(&["a", "b", "c"]), None).is_none());
        assert!(refresh_panel_snapshot(&[], &existing(&["b"]), Some("b")).is_none());
    }
    #[test]
    fn cleanup_throttle_boundaries() {
        assert!(!cleanup_due(Duration::from_secs(300), 0));
        assert!(!cleanup_due(Duration::from_millis(29_999), 19));
        assert!(cleanup_due(Duration::from_secs(30), 1));
        assert!(cleanup_due(Duration::ZERO, 20));
        assert!(cleanup_due(Duration::ZERO, u32::MAX));
    }
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

#[cfg(test)]
mod settings_tests {
    use super::*;

    // Rust 单元测试宿主没有 Tauri 应用的清单；Wry 引用的 TaskDialogIndirect
    // 需要 Common Controls v6，否则测试进程会在进入测试框架前加载失败。
    macro_rules! test_manifest_dependency {
        ($directive:literal) => {
            #[used]
            #[link_section = ".drectve"]
            static TEST_MANIFEST_DEPENDENCY: [u8; $directive.len()] = *$directive;
        };
    }
    test_manifest_dependency!(b" /MANIFESTDEPENDENCY:\"type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'\" ");

    #[derive(Default)]
    struct FakeRegistrar {
        current: Option<Hotkey>,
        calls: Vec<Option<Hotkey>>,
        fail: Vec<Hotkey>,
        unregister_fails: bool,
    }
    impl HotkeyRegistrar for FakeRegistrar {
        fn unregister(&mut self) -> Result<()> {
            self.calls.push(None);
            if self.unregister_fails {
                return Err(anyhow!("synthetic unregister failure"));
            }
            Ok(())
        }
        fn register(&mut self, hotkey: Hotkey) -> Result<()> {
            self.calls.push(Some(hotkey));
            if self.fail.contains(&hotkey) {
                return Err(anyhow!("synthetic registration failure"));
            }
            Ok(())
        }
    }

    struct Fixture {
        root: std::path::PathBuf,
        fake: Arc<Mutex<FakeRegistrar>>,
        quit: Arc<AtomicBool>,
        broker: Option<thread::JoinHandle<()>>,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../.test-data")
                .join(format!("t3-runtime-settings-{}", std::process::id()));
            assert!(!root.exists(), "不能覆盖已有测试数据：{}", root.display());
            Self {
                root,
                fake: Arc::new(Mutex::new(FakeRegistrar {
                    current: Some(Hotkey::AltV),
                    ..Default::default()
                })),
                quit: Arc::new(AtomicBool::new(false)),
                broker: None,
            }
        }
        fn start_fake_broker(&mut self) {
            let (requests, pending) = mpsc::channel::<BrokerRequest>();
            assert!(HOTKEY_REQUESTS.set(requests).is_ok());
            let fake = self.fake.clone();
            let quit = self.quit.clone();
            let (ready, wait) = mpsc::sync_channel(1);
            self.broker = Some(thread::spawn(move || {
                // 只建立本测试线程的消息队列，保证真实 PostThreadMessageW 路径可用。
                // 不创建窗口、不注册热键、不安装钩子，也不发送任何键盘事件。
                unsafe {
                    let mut msg = MSG::default();
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                    BROKER.store(GetCurrentThreadId(), Ordering::Release);
                }
                ready.send(()).unwrap();
                while !quit.load(Ordering::Acquire) {
                    match pending.recv_timeout(Duration::from_millis(20)) {
                        Ok(request) => {
                            if request.cancelled.load(Ordering::Acquire) {
                                continue;
                            }
                            let mut registrar = fake.lock().unwrap();
                            let mut current = registrar.current;
                            let result = settings::replace_hotkey(
                                &mut *registrar,
                                &mut current,
                                request.hotkey,
                            )
                            .map_err(|e| e.to_string());
                            registrar.current = current;
                            let _ = request.reply.send(result);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            }));
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        fn state(&self) -> State {
            let old = Settings::default();
            let mut storage = Storage::open(&self.root, &[72; 32]).unwrap();
            storage.save_settings(&old).unwrap();
            State {
                items: Mutex::new(Vec::new()),
                storage: Mutex::new(storage),
                capture_context: Mutex::new(None),
                retention_policy: Mutex::new(old.retention_policy()),
                settings: Mutex::new(old),
                startup_error: Mutex::new(None),
                jobs: mpsc::sync_channel(1).0,
                control: mpsc::channel().0,
                target: AtomicIsize::new(0),
                main: AtomicIsize::new(0),
                quick: AtomicIsize::new(0),
                ready: AtomicBool::new(false),
                panel: Mutex::new(Panel::default()),
                error: Mutex::new(None),
                smoke: true,
                main_ready: AtomicBool::new(false),
                quick_ready: AtomicBool::new(false),
                thumbnail_ready: AtomicBool::new(false),
                preview_ready: AtomicBool::new(false),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.quit.store(true, Ordering::Release);
            if let Some(broker) = self.broker.take() {
                broker.join().unwrap();
            }
            BROKER.store(0, Ordering::Release);
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    // T3-2/3：调用真实 apply_settings 与 change_main_hotkey；仅替换系统注册结果。
    // 单个测试持有唯一的 Tauri 事件循环及 OnceLock 通道，防止测试间竞争。
    #[test]
    fn settings_update_failures_preserve_database_and_old_registration() {
        let _hook_lock = hook::CONFIG_TEST_LOCK.lock().unwrap();
        // .drectve 由链接器消费；这里只保留符号引用，不读取该节。
        std::hint::black_box(std::ptr::addr_of!(TEST_MANIFEST_DEPENDENCY));
        let mut fixture = Fixture::new();
        fixture.start_fake_broker();
        let app = tauri::Builder::default()
            .any_thread()
            .build(tauri::generate_context!())
            .unwrap();
        let state = fixture.state();
        let old = Settings::default();
        let before = state.storage.lock().unwrap().test_settings_rows();
        let next = Settings {
            main_hotkey: Hotkey::CtrlAltV,
            autostart: true,
            quick_visible_count: 9,
            ..old.clone()
        };
        let assert_unchanged = || {
            let storage = state.storage.lock().unwrap();
            assert_eq!(storage.test_settings_rows(), before);
            assert_eq!(storage.load_settings().unwrap(), old);
            assert_eq!(*state.settings.lock().unwrap(), old);
            assert_eq!(
                *state.retention_policy.lock().unwrap(),
                old.retention_policy()
            );
            assert!(
                hook::reserve_configuration().is_ok(),
                "错误路径必须释放配置门"
            );
        };

        assert!(apply_settings(app.handle(), &state, next.clone(), true)
            .unwrap_err()
            .to_string()
            .contains("快速面板正在激活"));
        assert!(fixture.fake.lock().unwrap().calls.is_empty());
        assert_unchanged();

        // 即使 UI 尚未显示，配置门被占用也必须在注册和落库之前拒绝。
        let reservation = hook::reserve_configuration().unwrap();
        assert!(apply_settings(app.handle(), &state, next.clone(), false)
            .unwrap_err()
            .to_string()
            .contains("快速面板正在激活或处理按键"));
        assert!(fixture.fake.lock().unwrap().calls.is_empty());
        drop(reservation);
        assert_unchanged();

        fixture.fake.lock().unwrap().fail = vec![Hotkey::CtrlAltV];
        assert!(apply_settings(app.handle(), &state, next.clone(), false).is_err());
        {
            let mut fake = fixture.fake.lock().unwrap();
            assert_eq!(
                fake.calls,
                [None, Some(Hotkey::CtrlAltV), Some(Hotkey::AltV)]
            );
            assert_eq!(fake.current, Some(Hotkey::AltV));
            fake.calls.clear();
            fake.fail.clear();
            fake.unregister_fails = true;
        }
        assert_unchanged();

        assert!(apply_settings(app.handle(), &state, next.clone(), false)
            .unwrap_err()
            .to_string()
            .contains("synthetic unregister failure"));
        {
            let mut fake = fixture.fake.lock().unwrap();
            assert_eq!(fake.calls, [None]);
            assert_eq!(fake.current, Some(Hotkey::AltV));
            fake.calls.clear();
            fake.unregister_fails = false;
        }
        assert_unchanged();

        state.storage.lock().unwrap().test_reject_settings_update();
        assert!(apply_settings(app.handle(), &state, next, false)
            .unwrap_err()
            .to_string()
            .contains("设置未保存"));
        {
            let fake = fixture.fake.lock().unwrap();
            assert_eq!(
                fake.calls,
                [None, Some(Hotkey::CtrlAltV), None, Some(Hotkey::AltV)]
            );
            assert_eq!(fake.current, Some(Hotkey::AltV));
        }
        assert_unchanged();
        drop(state);
        assert_eq!(
            Storage::open(&fixture.root, &[72; 32])
                .unwrap()
                .test_settings_rows(),
            before
        );
    }
}
