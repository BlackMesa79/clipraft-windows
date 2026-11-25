pub(crate) mod archive_commands;
pub mod archive_service;
pub mod archive_task;
mod broker;
mod broker_messages;
pub(crate) mod cleanup_commands;
pub(crate) mod history_gate;
mod history_index;
mod main_window;
mod maintenance;
mod merge_paste;
mod panel_controller;
mod paste_job;
mod settings_update;
pub mod snapshot_task;
#[cfg(test)]
mod test_support;
mod worker;
use settings_update::apply_settings;

#[cfg(test)]
use crate::settings::{self, HotkeyRegistrar};
use crate::{
    clipboard, diag,
    hook::{self, Action},
    model::ItemView,
    paster, platform,
    settings::{Hotkey, Settings},
    storage::{RetentionPolicy, Storage},
};
use anyhow::{anyhow, Result};
use serde::Serialize;
#[cfg(test)]
use std::thread;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Sender, SyncSender},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tauri::Manager;
use tauri::{AppHandle, Emitter};
#[cfg(test)]
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::DataExchange::*,
    UI::WindowsAndMessaging::*,
};

const SETTINGS_MESSAGE: u32 = WM_APP + 81;
enum BrokerRequest {
    Replace {
        hotkey: Hotkey,
        reply: Sender<Result<(), String>>,
        cancelled: Arc<AtomicBool>,
    },
    Check {
        main: Hotkey,
        quick: Hotkey,
        reply: Sender<Result<crate::hotkey_check::Report, String>>,
    },
}
static HOTKEY_REQUESTS: OnceLock<Sender<BrokerRequest>> = OnceLock::new();
fn change_main_hotkey(hotkey: Hotkey) -> Result<()> {
    let (reply, wait) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    HOTKEY_REQUESTS
        .get()
        .ok_or_else(|| anyhow!("热键线程未启动"))?
        .send(BrokerRequest::Replace {
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
pub fn check_hotkeys(main: Hotkey, quick: Hotkey) -> Result<crate::hotkey_check::Report> {
    let (reply, wait) = mpsc::channel();
    HOTKEY_REQUESTS
        .get()
        .ok_or_else(|| anyhow!("热键线程未启动"))?
        .send(BrokerRequest::Check { main, quick, reply })
        .map_err(|_| anyhow!("热键线程已退出"))?;
    unsafe {
        PostThreadMessageW(
            BROKER.load(Ordering::Acquire),
            SETTINGS_MESSAGE,
            WPARAM(0),
            LPARAM(0),
        )
    }?;
    wait.recv_timeout(Duration::from_secs(2))
        .map_err(|_| anyhow!("热键检测未完成，请重试"))?
        .map_err(anyhow::Error::msg)
}
static LISTENER: AtomicIsize = AtomicIsize::new(0);
static WORK: OnceLock<SyncSender<Job>> = OnceLock::new();
static CONTROL: OnceLock<Sender<Control>> = OnceLock::new();
static CAPTURE_GATE: Mutex<()> = Mutex::new(());
static PAUSED: AtomicBool = AtomicBool::new(false);
pub fn paused() -> bool {
    PAUSED.load(Ordering::Acquire)
}
fn capture_paused() -> bool {
    paused() || snapshot_task::active()
}
static CAPTURE_PENDING: AtomicBool = AtomicBool::new(false);
use clipboard::LAST_SEQUENCE;
static LAST_EXTERNAL: AtomicIsize = AtomicIsize::new(0);
static PASTE_GENERATION: AtomicU64 = AtomicU64::new(0);
static PASTE_ESCAPE_REVISION: AtomicU64 = AtomicU64::new(0);
pub(crate) fn record_paste_escape() {
    PASTE_ESCAPE_REVISION.fetch_add(1, Ordering::Release);
}
pub(crate) fn paste_escape_revision() -> u64 {
    PASTE_ESCAPE_REVISION.load(Ordering::Acquire)
}
pub fn paste_generation() -> u64 {
    PASTE_GENERATION.load(Ordering::Acquire)
}
pub fn new_paste_intent() -> u64 {
    PASTE_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1)
}
pub(crate) enum Job {
    Capture,
    PasteSequence {
        ids: Vec<String>,
        target: isize,
        generation: u64,
        escape_revision: u64,
    },
    Paste {
        content: PasteContent,
        target: isize,
        restore: bool,
        plain: bool,
        generation: u64,
    },
}
pub(crate) const MAX_PASTE_SEQUENCE_ITEMS: usize = 50;
pub(crate) enum PasteContent {
    Item(String),
    Merged {
        ids: Vec<String>,
        separator: crate::settings::MergePasteSeparator,
    },
}
pub enum Control {
    Hook(Action, u64),
    Toggle,
    RecordedHotkey(u32, u64),
    OpenSettings,
    TogglePause,
    Hide,
    Report(String),
    PasteReport(String, u64),
    HistoryChanged,
    NeighborhoodReady,
    UpdateSettings(Settings, Sender<Result<Settings, String>>),
}
#[derive(Clone, Serialize, Default)]
pub struct Panel {
    pub items: Vec<ItemView>,
    pub selected: usize,
    pub visible: bool,
    pub error: Option<String>,
    pub serial: u64,
    pub session: u64,
    pub position: usize,
    pub total: usize,
    pub loading: bool,
    pub visible_count: u32,
    #[serde(skip)]
    pub history_revision: u64,
}
pub struct State {
    pub files: crate::file_status::Cache,
    pub settings_requested: AtomicBool,
    pub items: Mutex<Vec<ItemView>>,
    history_index: history_index::Cache,
    history_gate: history_gate::Gate,
    panel_serial: AtomicU64,
    pub storage: Mutex<Storage>,
    // 删除、保存与清理统一先锁此上下文，再锁 storage。
    pub capture_context: Mutex<Option<crate::owner_flush::CaptureContext>>,
    pub retention_policy: Mutex<RetentionPolicy>,
    pub hide_on_blur: AtomicBool,
    pub settings: Mutex<Settings>,
    pub content_rules: std::sync::RwLock<crate::settings::CompiledContentRules>,
    pub startup_error: Mutex<Option<String>>,
    pub jobs: SyncSender<Job>,
    pub control: Sender<Control>,
    pub target: AtomicIsize,
    pub target_focus: Mutex<Option<paster::focus_restore::Target>>,
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
fn update_error(state: &State, error: Option<String>) -> Result<()> {
    *state
        .error
        .lock()
        .map_err(|_| anyhow!("错误提示状态不可用"))? = error;
    Ok(())
}
fn clear_matching_error(state: &State, message: &str) -> Result<()> {
    let mut error = state
        .error
        .lock()
        .map_err(|_| anyhow!("错误提示状态不可用"))?;
    if error.as_deref() == Some(message) {
        *error = None;
    }
    Ok(())
}
pub fn report(_app: &AppHandle, state: &State, message: &str) {
    if let Err(error) = update_error(state, Some(message.into())) {
        diag::write(format!(
            "operation error state unavailable: {error}; {message}"
        ));
        return;
    }
    crate::app_events::record(
        "operation report send failed",
        crate::app_events::send_control(&state.control, Control::Report(message.into())),
    );
}
pub fn refresh(app: &AppHandle, state: &State) -> Result<()> {
    if let Err(error) = refresh_snapshots(state) {
        state.history_index.fail(error.to_string())?;
        return Err(error);
    }
    crate::app_events::record(
        "history control send failed",
        crate::app_events::send_control(&state.control, Control::HistoryChanged),
    );
    app.emit("history-changed", ())?;
    Ok(())
}
fn refresh_snapshots(state: &State) -> Result<()> {
    let storage = state.storage.lock().map_err(|_| anyhow!("存储不可用"))?;
    let mut items = storage.list()?;
    state.files.annotate(&mut items)?;
    state.history_index.refresh(&storage)?;
    let enabled = !state.smoke && !items.is_empty() && state.ready.load(Ordering::Acquire);
    *state.items.lock().map_err(|_| anyhow!("历史快照不可用"))? = items;
    hook::READY.store(enabled, Ordering::Release);
    drop(storage);
    Ok(())
}
fn queue_capture() {
    if capture_paused() {
        return;
    }
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
            // 检测使用独立ID，即使临时注册期间产生消息也不得唤出主面板。
            if w.0 != 1 {
                return LRESULT(0);
            }
            if hook::recording() {
                return LRESULT(0);
            }
            // 先掩码再调度 Toggle，避免 Alt 松开后进入菜单模式。
            let masked = paster::mask_shortcut();
            diag::write(format!("hotkey mask_ok={}", masked.is_ok()));
            diag::modifier_snapshot("toggle", "hotkey_after_mask");
            if let Some(tx) = CONTROL.get() {
                crate::app_events::record(
                    "broker toggle send failed",
                    crate::app_events::send_control(tx, Control::Toggle),
                );
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
fn on_main(app: &AppHandle, f: impl FnOnce() + Send + 'static) -> Result<()> {
    app.run_on_main_thread(f)
        .map_err(|error| anyhow!("主线程任务派发失败：{error}"))
}
// 快照来自已脱敏的内存条目；发布路径不能等待存储锁或执行 SQL。
#[cfg(test)]
fn store_panel_snapshot(state: &State, panel: &Panel) -> Result<()> {
    let stamp = state
        .history_gate
        .0
        .lock()
        .map_err(|_| anyhow!("历史版本不可用"))?;
    let mut panel = panel.clone();
    history_gate::sanitize(&mut panel, *stamp);
    *state.panel.lock().map_err(|_| anyhow!("面板快照不可用"))? = panel;
    Ok(())
}
fn publish(app: &AppHandle, state: &State, panel: &Panel) -> Result<()> {
    let stamp = state
        .history_gate
        .0
        .lock()
        .map_err(|_| anyhow!("历史版本不可用"))?;
    let mut panel = panel.clone();
    history_gate::sanitize(&mut panel, *stamp);
    panel.serial = state.panel_serial.fetch_add(1, Ordering::AcqRel) + 1;
    *state.panel.lock().map_err(|_| anyhow!("面板快照不可用"))? = panel.clone();
    if diag::enabled() {
        diag::write(format!(
            "panel_publish serial={} session={} visible={} loading={} items={} error={}",
            panel.serial,
            panel.session,
            panel.visible,
            panel.loading,
            panel.items.len(),
            panel.error.is_some()
        ));
    }
    app.emit_to("quick", "panel-state", &panel)?;
    Ok(())
}
pub(crate) fn panel_snapshot(state: &State) -> Result<Panel> {
    let stamp = state
        .history_gate
        .0
        .lock()
        .map_err(|_| anyhow!("历史版本不可用"))?;
    let mut panel = state
        .panel
        .lock()
        .map_err(|_| anyhow!("面板快照不可用"))?
        .clone();
    history_gate::sanitize(&mut panel, *stamp);
    Ok(panel)
}
// 按快照顺序替换为最新的完整展示对象，不新增条目。
// 选中记录消失时取消，绝不把原下标解释为另一条记录。
#[cfg(test)]
fn refresh_panel_snapshot(
    snapshot: &[ItemView],
    current: &[ItemView],
    selected_id: Option<&str>,
) -> Option<(Vec<ItemView>, usize)> {
    let selected_id = selected_id?;
    let items: Vec<_> = snapshot
        .iter()
        .filter_map(|old| current.iter().find(|item| item.id == old.id).cloned())
        .collect();
    let selected = items.iter().position(|item| item.id == selected_id)?;
    Some((items, selected))
}

fn hide_panel(app: &AppHandle, state: &State, panel: &mut Panel) -> Result<()> {
    hook::cancel_panel();
    panel.visible = false;
    diag::panel(true, false);
    let published = publish(app, state, panel);
    if panel.error.is_none() {
        let quick = state.quick.load(Ordering::Acquire);
        on_main(app, move || platform::hide(quick))?;
    }
    published
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
    let mut previous = context.lock().map_err(|_| anyhow!("采集上下文不可用"))?;
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
    hook::configure_settings(&settings);
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
    let files_app = app.clone();
    PAUSED.store(settings.pause_on_start, Ordering::Release);
    let state = Arc::new(State {
        files: crate::file_status::Cache::new(move || {
            crate::app_events::record(
                "file status event failed",
                files_app.emit("file-status-changed", ()),
            );
        })?,
        settings_requested: AtomicBool::new(false),
        items: Mutex::new(items),
        history_index: history_index::Cache::default(),
        history_gate: history_gate::Gate::default(),
        panel_serial: AtomicU64::new(0),
        storage: Mutex::new(storage),
        capture_context: Mutex::new(None),
        retention_policy: Mutex::new(policy),
        hide_on_blur: AtomicBool::new(settings.hide_on_blur),
        settings: Mutex::new(settings.clone()),
        content_rules: std::sync::RwLock::new(crate::settings::compile_content_rules(
            &settings.content_ignore_rules,
        )?),
        startup_error: Mutex::new(None),
        jobs: jobs.clone(),
        control: control.clone(),
        target: AtomicIsize::new(0),
        target_focus: Mutex::new(None),
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
    maintenance::start(app.clone(), &state)?;
    worker::start(app.clone(), state.clone(), rx)?;
    panel_controller::start(app.clone(), state.clone(), controls)?;
    WORK.set(jobs).map_err(|_| anyhow!("工作线程已启动"))?;
    CONTROL
        .set(control)
        .map_err(|_| anyhow!("控制线程已启动"))?;
    if !smoke {
        let startup = app
            .try_state::<crate::capture_policy::StartupCapture>()
            .map(|value| *value.inner())
            .unwrap_or_default();
        initialize_native(&state, &settings, startup)?;
    }
    Ok(state)
}
fn initialize_native(
    state: &State,
    settings: &Settings,
    startup: crate::capture_policy::StartupCapture,
) -> Result<()> {
    if let Err(error) = crate::autostart::set(settings.autostart) {
        update_error(state, Some(error.to_string()))?;
    }
    clipboard::initialize()?;
    let (receiver, error) = broker::start(settings.main_hotkey, startup)?;
    update_startup_error(state, error)?;
    hook::start(receiver)
}

fn update_startup_error(state: &State, error: Option<String>) -> Result<()> {
    *state
        .startup_error
        .lock()
        .map_err(|_| anyhow!("启动错误状态不可用"))? = error;
    Ok(())
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
            crate::app_events::record(
                "broker quit post failed",
                PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{poison, Fixture};
    use super::*;

    #[test]
    fn snapshot_store_reports_poison_without_mutating_input() {
        let fixture = Fixture::new("panel-snapshot");
        let state = fixture.state();
        let snapshot = Panel {
            items: vec![ItemView {
                id: "synthetic".into(),
                ..Default::default()
            }],
            visible: true,
            ..Default::default()
        };
        store_panel_snapshot(&state, &snapshot).unwrap();
        assert_eq!(state.panel.try_lock().unwrap().items[0].id, "synthetic");
        poison(&state.panel);
        assert!(store_panel_snapshot(&state, &Panel::default()).is_err());
        assert!(snapshot.visible);
        assert_eq!(snapshot.items[0].id, "synthetic");
    }

    #[test]
    fn old_notice_cannot_clear_new_error_and_startup_poison_is_returned() {
        let fixture = Fixture::new("error-status");
        let state = fixture.state();
        update_error(&state, Some("old".into())).unwrap();
        update_error(&state, Some("new".into())).unwrap();
        clear_matching_error(&state, "old").unwrap();
        assert_eq!(state.error.lock().unwrap().as_deref(), Some("new"));
        clear_matching_error(&state, "new").unwrap();
        assert!(state.error.lock().unwrap().is_none());
        poison(&state.error);
        assert!(update_error(&state, None).is_err());
        assert!(clear_matching_error(&state, "new").is_err());
        update_startup_error(&state, Some("synthetic".into())).unwrap();
        update_startup_error(&state, None).unwrap();
        assert!(state.startup_error.lock().unwrap().is_none());
        poison(&state.startup_error);
        assert!(update_startup_error(&state, None).is_err());
    }

    // AC-T8-2：暂停入口必须在访问系统序列号和入队前返回。
    // 不启动 broker/worker，不读取剪贴板，不发送键鼠事件。
    #[test]
    fn t8_paused_queue_does_not_set_pending_flag() {
        let _gate = CAPTURE_GATE.lock().unwrap();
        let old_paused = PAUSED.swap(true, Ordering::AcqRel);
        let old_pending = CAPTURE_PENDING.swap(false, Ordering::AcqRel);
        queue_capture();
        let pending = CAPTURE_PENDING.load(Ordering::Acquire);
        CAPTURE_PENDING.store(old_pending, Ordering::Release);
        PAUSED.store(old_paused, Ordering::Release);
        assert!(!pending, "暂停期间不得排入采集任务");
    }
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
    fn existing(ids: &[&str]) -> Vec<ItemView> {
        ids.iter()
            .map(|id| ItemView {
                id: (*id).into(),
                summary: format!("快照内容 {id}"),
                ..Default::default()
            })
            .collect()
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
        assert!(refresh_panel_snapshot(&snapshot(), &[], Some("b")).is_none());
    }
    #[test]
    fn snapshot_cancels_when_selection_is_missing() {
        assert!(refresh_panel_snapshot(&snapshot(), &existing(&["a", "b", "c"]), None).is_none());
        assert!(refresh_panel_snapshot(&[], &existing(&["b"]), Some("b")).is_none());
    }
    #[test]
    fn snapshot_replaces_views_without_changing_order_or_selected_id() {
        let mut current = existing(&["new", "c", "b", "a"]);
        for item in &mut current {
            item.summary = format!("最新内容 {}", item.id);
            item.favorite = true;
        }
        let (items, selected) = refresh_panel_snapshot(&snapshot(), &current, Some("b")).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(selected, 1);
        assert_eq!(items[selected].id, "b");
        for item in &items {
            let latest = current.iter().find(|latest| latest.id == item.id).unwrap();
            assert_eq!(
                serde_json::to_value(item).unwrap(),
                serde_json::to_value(latest).unwrap()
            );
        }
    }
    #[test]
    fn snapshot_replaces_sensitive_view_without_plaintext_token() {
        let token = "sk-SyntheticOnly0123456789abcdef";
        let mut old = snapshot();
        old[1].summary = token.into();
        old[1].source_title = Some(token.into());
        old[1].files = vec![token.into()];
        old[1].thumbnail = Some(token.into());
        old[1].preview = Some(token.into());
        let mut current = existing(&["c", "b", "a"]);
        current[1] = ItemView {
            id: "b".into(),
            summary: "sk-S••••••••ef".into(),
            sensitive: true,
            expires_at: Some(600_000),
            ..Default::default()
        };
        let (items, selected) = refresh_panel_snapshot(&old, &current, Some("b")).unwrap();
        assert_eq!(selected, 1);
        assert_eq!(items[selected].id, "b");
        assert!(items[selected].sensitive);
        assert_eq!(items[selected].expires_at, Some(600_000));
        assert_eq!(
            serde_json::to_value(&items[selected]).unwrap(),
            serde_json::to_value(&current[1]).unwrap()
        );
        assert!(!serde_json::to_string(&items).unwrap().contains(token));
    }
    #[test]
    fn snapshot_replaces_unmarked_view_and_removes_deleted_item() {
        let mut old = snapshot();
        old[1].summary = "••••••••".into();
        old[1].sensitive = true;
        old[1].expires_at = Some(600_000);
        let current = existing(&["c", "b"]);
        let (items, selected) = refresh_panel_snapshot(&old, &current, Some("b")).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert!(!items.iter().any(|item| item.id == "a"));
        assert_eq!(selected, 0);
        assert_eq!(items[selected].id, "b");
        assert_eq!(items[selected].summary, "快照内容 b");
        assert!(!items[selected].sensitive);
        assert_eq!(items[selected].expires_at, None);
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
        probe_conflict: bool,
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
                .join(format!(
                    "t3-runtime-settings-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
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
                        Ok(BrokerRequest::Replace {
                            hotkey,
                            reply,
                            cancelled,
                        }) => {
                            if cancelled.load(Ordering::Acquire) {
                                continue;
                            }
                            let mut registrar = fake.lock().unwrap();
                            let mut current = registrar.current;
                            let result =
                                settings::replace_hotkey(&mut *registrar, &mut current, hotkey)
                                    .map_err(|e| e.to_string());
                            registrar.current = current;
                            let _ = reply.send(result);
                        }
                        Ok(BrokerRequest::Check { main, quick, reply }) => {
                            let current = fake.lock().unwrap();
                            let registered = current.current;
                            let conflict = current.probe_conflict;
                            drop(current);
                            let result =
                                crate::hotkey_check::check_pair(main, quick, registered, |_| {
                                    crate::hotkey_check::Finding {
                                        status: if conflict {
                                            crate::hotkey_check::Status::Conflict
                                        } else {
                                            crate::hotkey_check::Status::Available
                                        },
                                        message: if conflict {
                                            "系统已占用".into()
                                        } else {
                                            String::new()
                                        },
                                    }
                                })
                                .map_err(|e| e.to_string());
                            let _ = reply.send(result);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            }));
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        fn state(&self) -> State {
            self.state_at(&self.root)
        }
        fn state_at(&self, root: &std::path::Path) -> State {
            super::test_support::state(root)
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

    use super::test_support::poison;

    #[derive(Clone, Copy, Debug)]
    enum PoisonTarget {
        Settings,
        Startup,
        Policy,
        Context,
        Storage,
    }

    impl PoisonTarget {
        fn poison(self, state: &State) {
            match self {
                Self::Settings => poison(&state.settings),
                Self::Startup => poison(&state.startup_error),
                Self::Policy => poison(&state.retention_policy),
                Self::Context => poison(&state.capture_context),
                Self::Storage => poison(&state.storage),
            }
        }
    }

    fn assert_poisoned_updates_rollback(app: &AppHandle, fixture: &Fixture) {
        for target in [
            PoisonTarget::Settings,
            PoisonTarget::Startup,
            PoisonTarget::Policy,
            PoisonTarget::Context,
            PoisonTarget::Storage,
        ] {
            let root = fixture.root.join(format!("poison-{target:?}"));
            let state = fixture.state_at(&root);
            let old = Settings::default();
            let before = state.storage.lock().unwrap().test_settings_rows();
            fixture.fake.lock().unwrap().calls.clear();
            target.poison(&state);
            let next = Settings {
                main_hotkey: Hotkey::CtrlAltV,
                quick_visible_count: 9,
                ..old.clone()
            };
            assert!(
                apply_settings(app, &state, next, false).is_err(),
                "{target:?}"
            );
            let calls = match target {
                PoisonTarget::Settings | PoisonTarget::Startup => vec![],
                PoisonTarget::Policy | PoisonTarget::Context | PoisonTarget::Storage => {
                    vec![None, Some(Hotkey::CtrlAltV), None, Some(Hotkey::AltV)]
                }
            };
            assert_eq!(fixture.fake.lock().unwrap().calls, calls, "{target:?}");
            assert_eq!(fixture.fake.lock().unwrap().current, Some(Hotkey::AltV));
            if !state.settings.is_poisoned() {
                assert_eq!(*state.settings.try_lock().unwrap(), old);
            }
            if !state.retention_policy.is_poisoned() {
                assert_eq!(
                    *state.retention_policy.try_lock().unwrap(),
                    old.retention_policy()
                );
            }
            if !state.capture_context.is_poisoned() {
                assert!(state.capture_context.try_lock().unwrap().is_none());
            }
            if !state.startup_error.is_poisoned() {
                assert!(state.startup_error.try_lock().unwrap().is_none());
            }
            assert!(
                hook::reserve_configuration().is_ok(),
                "错误路径必须释放配置门"
            );
            drop(state);
            assert_eq!(
                Storage::open(&root, &[72; 32])
                    .unwrap()
                    .test_settings_rows(),
                before
            );
        }
    }

    fn assert_failed_rollback_is_reported(app: &AppHandle, fixture: &Fixture) {
        let root = fixture.root.join("poison-policy-rollback-failure");
        let state = fixture.state_at(&root);
        poison(&state.retention_policy);
        fixture.fake.lock().unwrap().fail = vec![Hotkey::AltV];
        let next = Settings {
            main_hotkey: Hotkey::CtrlAltV,
            ..Settings::default()
        };
        let error = apply_settings(app, &state, next, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("保留策略不可用"));
        assert!(error.contains("热键恢复失败"));
        assert_eq!(
            state.startup_error.lock().unwrap().as_deref(),
            Some(error.as_str())
        );
        assert_eq!(*state.settings.lock().unwrap(), Settings::default());
        assert_eq!(
            state.storage.lock().unwrap().load_settings().unwrap(),
            Settings::default()
        );
        assert!(hook::reserve_configuration().is_ok());
        let mut registrar = fixture.fake.lock().unwrap();
        registrar.fail.clear();
        registrar.current = Some(Hotkey::AltV);
        registrar.calls.clear();
    }

    fn assert_successful_update_commits_and_clears_startup_error(
        app: &AppHandle,
        fixture: &Fixture,
    ) {
        let root = fixture.root.join("successful-settings-update");
        let state = fixture.state_at(&root);
        *state.startup_error.lock().unwrap() = Some("synthetic startup failure".into());
        let next = Settings {
            main_hotkey: Hotkey::CtrlAltV,
            max_items: 50,
            hide_on_blur: true,
            content_ignore_rules: vec![crate::settings::ContentIgnoreRule {
                mode: crate::settings::ContentIgnoreMode::Keyword,
                value: "private marker".into(),
            }],
            ..Settings::default()
        };
        assert_eq!(
            apply_settings(app, &state, next.clone(), false).unwrap(),
            next
        );
        assert_eq!(*state.settings.try_lock().unwrap(), next);
        assert_eq!(
            state.storage.try_lock().unwrap().load_settings().unwrap(),
            next
        );
        assert_eq!(
            *state.retention_policy.try_lock().unwrap(),
            next.retention_policy()
        );
        assert!(state.hide_on_blur.load(Ordering::Acquire));
        assert!(state.startup_error.try_lock().unwrap().is_none());
        assert!(state.capture_context.try_lock().is_ok());
        assert!(state
            .content_rules
            .read()
            .unwrap()
            .matches(Some("PRIVATE MARKER")));
        assert!(hook::reserve_configuration().is_ok());
        assert_eq!(fixture.fake.lock().unwrap().current, Some(Hotkey::CtrlAltV));
        apply_settings(app, &state, Settings::default(), false).unwrap();
        assert_eq!(fixture.fake.lock().unwrap().current, Some(Hotkey::AltV));
        assert!(!state
            .content_rules
            .read()
            .unwrap()
            .matches(Some("PRIVATE MARKER")));
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

        fixture.fake.lock().unwrap().probe_conflict = true;
        assert!(apply_settings(app.handle(), &state, next.clone(), false)
            .unwrap_err()
            .to_string()
            .contains("系统已占用"));
        assert!(fixture.fake.lock().unwrap().calls.is_empty());
        fixture.fake.lock().unwrap().probe_conflict = false;
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
        fixture.fake.lock().unwrap().calls.clear();
        assert!(apply_settings(
            app.handle(),
            &state,
            Settings {
                quick_hotkey: Hotkey::CtrlAltShiftV,
                ..old.clone()
            },
            false
        )
        .unwrap_err()
        .to_string()
        .contains("设置未保存"));
        assert!(fixture.fake.lock().unwrap().calls.is_empty());
        assert_unchanged();
        assert_poisoned_updates_rollback(app.handle(), &fixture);
        assert_failed_rollback_is_reported(app.handle(), &fixture);
        assert_successful_update_commits_and_clears_startup_error(app.handle(), &fixture);
        panel_controller::tests::check_failures(app.handle());
        panel_controller::integration_tests::check_integration(app.handle());
        // AC-6/7：复用本进程唯一的 Tauri 测试应用；不新建窗口或启动监听。
        // 持有存储及采集上下文时，真实 publish / panel_state 仍须完成。
        let fake = "sk-TESTONLY7777777777777777abcd";
        let clip = crate::model::Clip {
            formats: vec![crate::model::FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: fake
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            }],
        };
        let items = {
            let mut storage = state.storage.lock().unwrap();
            storage.save(&clip).unwrap();
            storage.list().unwrap()
        };
        assert!(items[0].sensitive);
        let panel = Panel {
            items,
            selected: 0,
            visible: true,
            error: None,
            history_revision: state.history_gate.stamp().unwrap().revision,
            ..Default::default()
        };
        let mut expected_panel = panel.clone();
        expected_panel.serial = state.panel_serial.load(Ordering::Acquire) + 1;
        let expected = serde_json::to_value(&expected_panel).unwrap();
        let shared = Arc::new(state);
        assert!(app.manage(shared.clone()));
        let context_guard = shared.capture_context.lock().unwrap();
        let storage_guard = shared.storage.lock().unwrap();
        let handle = app.handle().clone();
        let worker_state = shared.clone();
        let (finished, result) = mpsc::sync_channel(1);
        let publisher = thread::spawn(move || {
            publish(&handle, &worker_state, &panel).unwrap();
            let payload = crate::panel_state(handle.state::<Arc<State>>()).unwrap();
            finished
                .send(serde_json::to_value(payload).unwrap())
                .unwrap();
        });
        let response = result.recv_timeout(Duration::from_secs(3));
        // 超时也先释放两把锁，使失败路径能结束后台测试线程。
        drop(storage_guard);
        drop(context_guard);
        publisher.join().unwrap();
        let payload = response.expect("快速面板发布或读取不应等待存储/采集锁");
        assert_eq!(payload, expected);
        assert!(!payload.to_string().contains(fake));
        assert!(crate::get_settings(app.state::<Arc<State>>()).is_ok());
        assert!(crate::startup_error(app.state::<Arc<State>>()).is_ok());
        *shared.error.lock().unwrap() = Some("入口测试错误".into());
        assert_eq!(
            crate::last_error(app.state::<Arc<State>>())
                .unwrap()
                .as_deref(),
            Some("入口测试错误")
        );
        assert_eq!(crate::last_error(app.state::<Arc<State>>()).unwrap(), None);
        super::test_support::poison(&shared.settings);
        super::test_support::poison(&shared.startup_error);
        super::test_support::poison(&shared.panel);
        super::test_support::poison(&shared.error);
        assert_eq!(
            crate::get_settings(app.state::<Arc<State>>()).unwrap_err(),
            "设置不可用"
        );
        assert_eq!(
            crate::startup_error(app.state::<Arc<State>>()).unwrap_err(),
            "启动错误状态不可用"
        );
        assert!(crate::panel_state(app.state::<Arc<State>>()).is_err());
        assert_eq!(
            crate::last_error(app.state::<Arc<State>>()).unwrap_err(),
            "错误状态不可用"
        );
        drop(app);
        drop(shared);
        assert_eq!(
            Storage::open(&fixture.root, &[72; 32])
                .unwrap()
                .test_settings_rows(),
            before
        );
    }
}
