//! 可选诊断：钩子仅投递脱敏定长消息，文件写入由专用线程完成。
use crate::{hook, paster, platform};
use anyhow::{anyhow, Result};
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        OnceLock,
    },
    thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyboardLayout, VK_CONTROL},
        WindowsAndMessaging::*,
    },
};
const KEY_MESSAGE: u32 = WM_APP + 91;
const TEXT_MESSAGE: u32 = WM_APP + 92;
static THREAD: AtomicU32 = AtomicU32::new(0);
static LOST: AtomicU32 = AtomicU32::new(0);
static CLOCK: OnceLock<Instant> = OnceLock::new();
static TEXT: OnceLock<SyncSender<(u64, String)>> = OnceLock::new();
static MAIN_OPEN: AtomicBool = AtomicBool::new(false);
static QUICK_OPEN: AtomicBool = AtomicBool::new(false);
static UNTIL: AtomicU64 = AtomicU64::new(0);
fn millis() -> u64 {
    CLOCK
        .get()
        .map_or(0, |clock| clock.elapsed().as_millis() as u64 + 1)
}
pub fn enabled() -> bool {
    THREAD.load(Ordering::Relaxed) != 0
}
fn open_log(root: &Path) -> std::io::Result<std::fs::File> {
    // 诊断先于主密钥初始化，新数据目录可能尚不存在。
    std::fs::create_dir_all(root)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("diag.log"))
}
pub fn init(root: &Path) -> Result<()> {
    if std::env::var("CLIPBOARD_M0_DIAG").as_deref() != Ok("1") {
        return Ok(());
    }
    let mut file = open_log(root)?;
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    writeln!(
        file,
        "session unix_ms={epoch} modifier_order=A2,A3,A0,A1,A4,A5,5B,5C"
    )?;
    CLOCK.get_or_init(Instant::now);
    let (tx, rx) = mpsc::sync_channel::<(u64, String)>(1024);
    TEXT.set(tx).map_err(|_| anyhow!("诊断日志已初始化"))?;
    let (ready, wait) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("diagnostic-log".into())
        .spawn(move || writer::run(file, rx, ready))?;
    wait.recv()?;
    Ok(())
}
pub fn panel(quick: bool, open: bool) {
    if !enabled() {
        return;
    }
    let state = if quick { &QUICK_OPEN } else { &MAIN_OPEN };
    if state.swap(open, Ordering::Relaxed) && !open {
        UNTIL.store(millis() + 2000, Ordering::Relaxed);
    }
}
fn in_window(main: bool, quick: bool, now: u64, until: u64) -> bool {
    main || quick || (until != 0 && now <= until)
}
fn safe_vk(vk: u32) -> usize {
    if matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C | 0x56 | 0x25..=0x28 | 0x1B) {
        vk as usize
    } else {
        0x100
    }
}
fn safe_scan(vk: u32, scan: u32) -> usize {
    if safe_vk(vk) == 0x100 {
        0
    } else {
        (scan & 0xff) as usize
    }
}
pub fn keyboard(
    vk: u32,
    scan: u32,
    down: bool,
    injected: bool,
    swallowed: bool,
    action: Option<hook::Action>,
) {
    let id = THREAD.load(Ordering::Relaxed);
    if id == 0 {
        return;
    }
    let now = millis();
    if !in_window(
        MAIN_OPEN.load(Ordering::Relaxed),
        QUICK_OPEN.load(Ordering::Relaxed),
        now,
        UNTIL.load(Ordering::Relaxed),
    ) {
        return;
    }
    // 脱敏在跨线程前完成；普通键码不会进入消息队列。
    let packed = safe_vk(vk)
        | ((down as usize) << 9)
        | ((injected as usize) << 10)
        | ((swallowed as usize) << 11)
        | (action.map_or(0, |a| a as usize) << 12)
        | ((unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } < 0) as usize) << 15
        | (safe_scan(vk, scan) << 16);
    if unsafe { PostThreadMessageW(id, KEY_MESSAGE, WPARAM(packed), LPARAM(now as isize)) }.is_err()
    {
        LOST.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn write(text: String) {
    let id = THREAD.load(Ordering::Relaxed);
    if id == 0 {
        return;
    }
    if TEXT
        .get()
        .is_some_and(|tx| tx.try_send((millis(), text)).is_err())
    {
        LOST.fetch_add(1, Ordering::Relaxed);
    }
    // 使用同一次采样，退出清零不能把唤醒误投给调用线程自身。
    if unsafe { PostThreadMessageW(id, TEXT_MESSAGE, WPARAM(0), LPARAM(0)) }.is_err() {
        LOST.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn foreground() -> String {
    unsafe {
        let hwnd = platform::hwnd(platform::foreground());
        let mut class = [0u16; 256];
        let count = GetClassNameW(hwnd, &mut class).max(0) as usize;
        let mut pid = 0;
        let tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let owner = crate::clipboard_owner::window_process(hwnd, write);
        let mut gui = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let gui_ok = GetGUIThreadInfo(tid, &mut gui).is_ok();
        let layout = GetKeyboardLayout(tid);
        format!(
            "class={:?} hkl={:X} exe={:?} pid={} hwnd=0x{:X} focus=0x{:X} active=0x{:X} gui_ok={}",
            String::from_utf16_lossy(&class[..count]),
            layout.0 as usize,
            owner.exe,
            pid,
            hwnd.0 as usize,
            gui.hwndFocus.0 as usize,
            gui.hwndActive.0 as usize,
            gui_ok
        )
    }
}
pub fn snapshot(stage: &str) {
    modifier_snapshot("paste", stage);
}
pub fn modifier_snapshot(context: &str, stage: &str) {
    if !enabled() {
        return;
    }
    let logical = paster::logical();
    let physical = hook::PHYSICAL.load(Ordering::SeqCst);
    let keys: Vec<_> = hook::MODIFIER_KEYS
        .iter()
        .enumerate()
        .map(|(i, k)| {
            format!(
                "{k:02X}:async={},physical={}",
                (logical >> i) & 1,
                (physical >> i) & 1
            )
        })
        .collect();
    write(format!(
        "{context} stage={stage} {} modifiers=[{}]",
        foreground(),
        keys.join(" ")
    ));
}
/// 进程即将退出，直接写入并刷盘，避免异步日志队列尚未消费就 exit。
pub fn startup_failure(message: &str) {
    let result = (|| -> Result<()> {
        let root = crate::platform::data_root()?;
        let mut file = open_log(&root)?;
        writeln!(file, "startup failure: {message}")?;
        file.sync_data()?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("startup diagnostic failed: {error:#}");
    }
}

mod writer;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_character_scan_codes_are_redacted() {
        assert_eq!(safe_scan(0x41, 30), 0);
        assert_eq!(safe_scan(0x31, 2), 0);
        assert_eq!(safe_scan(0x56, 47), 47);
        assert_eq!(safe_scan(0xA2, 29), 29);
    }
    #[test]
    fn log_creates_new_data_directory_and_preserves_existing_log() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "diag-init-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        assert!(!root.exists());
        writeln!(open_log(&root).unwrap(), "first").unwrap();
        writeln!(open_log(&root).unwrap(), "second").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("diag.log")).unwrap(),
            "first\nsecond\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn ordinary_keys_are_redacted_before_posting() {
        for vk in 0..=255 {
            let allowed = [
                0x10, 0x11, 0x12, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C, 0x56, 0x25, 0x26,
                0x27, 0x28, 0x1B,
            ]
            .contains(&vk);
            assert_eq!(safe_vk(vk), if allowed { vk as usize } else { 0x100 });
        }
        assert_eq!(safe_vk(0xE8), 0x100);
    }
    #[test]
    fn events_only_within_panel_or_two_second_tail() {
        assert!(!in_window(false, false, 0, 0));
        assert!(in_window(true, false, 5000, 0));
        assert!(in_window(false, true, 5000, 0));
        assert!(in_window(false, false, 3000, 3000));
        assert!(!in_window(false, false, 3001, 3000));
    }
}
