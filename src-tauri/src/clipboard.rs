use crate::{
    capture_policy::{self, CaptureError, Snapshot},
    model::{encode_hdrop, Clip, FormatData},
};
use anyhow::{anyhow, bail, Result};
use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicU32, Ordering},
        OnceLock,
    },
    thread,
    time::Duration,
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
        System::{DataExchange::*, Memory::*},
        UI::Shell::{DragQueryFileW, HDROP},
    },
};

pub const SUPPORTED: [(&str, u32); 8] = [
    ("CF_UNICODETEXT", 13),
    ("HTML Format", 0),
    ("Rich Text Format", 0),
    ("CF_DIBV5", 17),
    ("CF_DIB", 8),
    ("PNG", 0),
    ("CF_HDROP", 15),
    ("Preferred DropEffect", 0),
];
const MAX_BYTES: usize = crate::storage::crypto::MAX_PLAIN_BYTES;
#[derive(Debug)]
struct CaptureLimit {
    actual: usize,
    limit: usize,
    image: bool,
}
impl std::fmt::Display for CaptureLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(
            &CaptureError::Limit {
                actual: self.actual,
                limit: self.limit,
                image: self.image,
            },
            f,
        )
    }
}
impl std::error::Error for CaptureLimit {}
static TOKEN: OnceLock<[u8; 32]> = OnceLock::new();
static SELF_SEQUENCE: AtomicU32 = AtomicU32::new(0);
// 由单个采集 worker 推进；监听器据此过滤已结束的采集轮次。
pub static LAST_SEQUENCE: AtomicU32 = AtomicU32::new(0);
fn snapshot() -> Snapshot {
    let sequence = unsafe { GetClipboardSequenceNumber() };
    Snapshot {
        sequence,
        self_write: sequence != 0 && sequence == SELF_SEQUENCE.load(Ordering::Acquire),
    }
}
pub(crate) struct Captured {
    pub clip: Clip,
    pub source: crate::capture_source::CaptureSource,
    pub context: crate::owner_flush::CaptureContext,
    pub sequence: u32,
}
#[cfg(test)]
pub fn capture_pending(hwnd: HWND) -> std::result::Result<Option<Captured>, CaptureError> {
    capture_pending_with_settings(hwnd, Vec::new)
}
pub fn capture_pending_with_settings(
    hwnd: HWND,
    ignored_apps: impl Fn() -> Vec<String>,
) -> std::result::Result<Option<Captured>, CaptureError> {
    capture_policy::run(
        &LAST_SEQUENCE,
        snapshot,
        |sequence| {
            capture_at(hwnd, sequence, &ignored_apps()).map_err(|error| {
                if let Some(limit) = error.downcast_ref::<CaptureLimit>() {
                    CaptureError::Limit {
                        actual: limit.actual,
                        limit: limit.limit,
                        image: limit.image,
                    }
                } else {
                    error
                        .downcast_ref::<CaptureError>()
                        .copied()
                        .unwrap_or(CaptureError::Invalid)
                }
            })
        },
        thread::sleep,
        crate::diag::write,
    )
}
const OWNER_MARKER: &str = "ClipboardM0.Owner.v1";
pub fn initialize() -> Result<()> {
    let mut t = [0u8; 32];
    getrandom::fill(&mut t).map_err(|_| anyhow!("随机数不可用"))?;
    TOKEN.set(t).map_err(|_| anyhow!("剪贴板已初始化"))
}
pub fn format_id(name: &str) -> u32 {
    if let Some((_, id)) = SUPPORTED.iter().find(|(n, id)| *n == name && *id != 0) {
        return *id;
    }
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) }
}
struct OpenGuard;
impl Drop for OpenGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}
fn open(hwnd: HWND) -> Result<OpenGuard> {
    for _ in 0..8 {
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            return Ok(OpenGuard);
        }
        thread::sleep(Duration::from_millis(8));
    }
    bail!("剪贴板被其他程序占用")
}
#[cfg(test)]
thread_local! { static DATA_REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

unsafe fn clipboard_data(id: u32) -> windows::core::Result<HANDLE> {
    #[cfg(test)]
    DATA_REQUESTS.with(|count| count.set(count.get() + 1));
    GetClipboardData(id)
}

unsafe fn read_raw(
    id: u32,
    diagnostics: &mut crate::clipboard_diag::CaptureLog,
) -> Result<Option<Vec<u8>>> {
    read_raw_limited(id, MAX_BYTES, diagnostics)
}
unsafe fn read_raw_limited(
    id: u32,
    limit: usize,
    diagnostics: &mut crate::clipboard_diag::CaptureLog,
) -> Result<Option<Vec<u8>>> {
    if IsClipboardFormatAvailable(id).is_err() {
        return Ok(None);
    }
    let h = clipboard_data(id).inspect_err(|_| diagnostics.empty(id))?;
    let global = HGLOBAL(h.0);
    let size = GlobalSize(global);
    if size > limit {
        return Err(CaptureLimit {
            actual: size,
            limit,
            image: matches!(id, 8 | 17),
        }
        .into());
    }
    if size == 0 {
        diagnostics.empty(id);
        bail!("格式大小无效");
    }
    let p = GlobalLock(global);
    if p.is_null() {
        bail!("剪贴板内存不可读");
    }
    let out = std::slice::from_raw_parts(p as *const u8, size).to_vec();
    let _ = GlobalUnlock(global);
    diagnostics.read(id, out.len());
    Ok(Some(out))
}
fn normalize(name: &str, mut bytes: Vec<u8>) -> Vec<u8> {
    // 去除 HGLOBAL 分配填充，使相同文本可以稳定去重。
    if name == "CF_UNICODETEXT" {
        if let Some(i) = bytes.as_chunks::<2>().0.iter().position(|p| *p == [0, 0]) {
            bytes.truncate((i + 1) * 2);
        }
    } else if name == "HTML Format" || name == "Rich Text Format" {
        if let Some(i) = bytes.iter().position(|b| *b == 0) {
            bytes.truncate(i + 1);
        }
    } else if name == "Preferred DropEffect" {
        bytes.truncate(4);
    }
    crate::image_normalize::normalize(name, &mut bytes);
    bytes
}
pub fn may_record(excluded: bool, history: Option<&[u8]>) -> bool {
    !excluded
        && history.is_none_or(|bytes| {
            bytes
                .get(..4)
                .is_some_and(|b| u32::from_le_bytes(b.try_into().unwrap()) != 0)
        })
}

#[cfg(test)]
pub fn capture(hwnd: HWND) -> Result<Option<Clip>> {
    capture_at(hwnd, unsafe { GetClipboardSequenceNumber() }, &[])
        .map(|capture| capture.map(|capture| capture.clip))
}
fn capture_at(hwnd: HWND, expected: u32, ignored_apps: &[String]) -> Result<Option<Captured>> {
    let current = snapshot();
    if current.self_write {
        LAST_SEQUENCE.store(current.sequence, Ordering::Release);
        return Ok(None);
    }
    if current.sequence != expected {
        return Ok(None);
    }
    // 采集只尝试打开一次；退避在 worker 进行，写回保留原有短重试。
    unsafe { OpenClipboard(Some(hwnd)) }.map_err(|_| CaptureError::Busy)?;
    let _guard = OpenGuard;
    let mut diagnostics = crate::clipboard_diag::CaptureLog::begin();
    unsafe {
        // 打开前仍可能换序列，不能把新内容算作旧轮次采集。
        let current = snapshot();
        if current.self_write || current.sequence != expected {
            return Ok(None);
        }
        let formats = offered_formats();
        let owner = crate::clipboard_owner::clipboard_owner();
        let source = crate::capture_source::capture(&owner);
        if crate::capture_source::ignored(source.exe.as_deref(), ignored_apps) {
            crate::diag::write(format!(
                "capture sequence={expected} result=skipped reason=ignored_app"
            ));
            return Ok(None);
        }
        let bitmap_offered = formats.as_ref().is_none_or(|ids| {
            [2, 8, 17, format_id("PNG"), format_id("image/png")]
                .iter()
                .any(|id| ids.contains(id))
        });
        let excluded =
            IsClipboardFormatAvailable(format_id("ExcludeClipboardContentFromMonitorProcessing"))
                .is_ok();
        if excluded {
            return Ok(None);
        }
        let history = match read_raw(format_id("CanIncludeInClipboardHistory"), &mut diagnostics) {
            Ok(history) => history,
            Err(_) => {
                eprintln!("剪贴板隐私标记不可读，已跳过记录");
                return Ok(None);
            }
        };
        if !may_record(excluded, history.as_deref()) {
            return Ok(None);
        }
        if let Some(b) = read_raw(format_id(OWNER_MARKER), &mut diagnostics).unwrap_or_else(|_| {
            eprintln!("剪贴板所有权标记不可读，已跳过该标记");
            None
        }) {
            if TOKEN.get().is_some_and(|t| b.starts_with(t)) {
                // 延迟渲染等操作可能推进序列，所有权标记也可确认自写回。
                SELF_SEQUENCE.store(current.sequence, Ordering::Release);
                return Ok(None);
            }
        }
        collect_formats_with_budget(
            |name, budget| read_format_limited(name, budget, &mut diagnostics),
            MAX_BYTES,
        )
        .map(|clip| {
            clip.map(|clip| Captured {
                clip,
                source,
                sequence: expected,
                context: crate::owner_flush::CaptureContext {
                    record_id: None,
                    formats,
                    owner,
                    fingerprint: None,
                    bitmap_offered,
                },
            })
        })
    }
}
/// 调用方持有剪贴板；读取载荷之前保留完整的已提供格式集合。
fn offered_formats() -> Option<std::collections::BTreeSet<u32>> {
    use windows::Win32::Foundation::{GetLastError, SetLastError, ERROR_SUCCESS};
    let mut formats = std::collections::BTreeSet::new();
    let mut id = 0;
    loop {
        unsafe {
            SetLastError(ERROR_SUCCESS);
        }
        id = unsafe { EnumClipboardFormats(id) };
        if id == 0 {
            return (unsafe { GetLastError() } == ERROR_SUCCESS).then_some(formats);
        }
        if !formats.insert(id) {
            return None;
        }
    }
}
#[cfg(test)]
unsafe fn read_format(name: &str) -> Result<Option<Vec<u8>>> {
    read_format_limited(name, MAX_BYTES, &mut Default::default())
}
unsafe fn read_format_limited(
    name: &str,
    budget: usize,
    diagnostics: &mut crate::clipboard_diag::CaptureLog,
) -> Result<Option<Vec<u8>>> {
    let id = format_id(name);
    if name != "CF_HDROP" {
        return read_raw_limited(id, budget, diagnostics).map_err(|error| {
            if let Some(limit) = error.downcast_ref::<CaptureLimit>() {
                CaptureLimit {
                    actual: limit.actual,
                    limit: limit.limit,
                    image: matches!(name, "CF_DIBV5" | "CF_DIB" | "PNG"),
                }
                .into()
            } else {
                error
            }
        });
    }
    if IsClipboardFormatAvailable(id).is_err() {
        return Ok(None);
    }
    let handle = clipboard_data(id).inspect_err(|_| diagnostics.empty(id))?;
    let h = HDROP(handle.0);
    let count = DragQueryFileW(h, u32::MAX, None);
    if count == 0 {
        diagnostics.empty(id);
    }
    if count == 0 || count > 4096 {
        bail!("文件数量无效或超出限制");
    }
    let mut paths = Vec::new();
    for i in 0..count {
        let len = DragQueryFileW(h, i, None) as usize;
        if len == 0 || len > 32768 {
            bail!("文件路径长度无效或过长");
        }
        let mut buf = vec![0u16; len + 1];
        if DragQueryFileW(h, i, Some(&mut buf)) as usize != len {
            bail!("文件路径读取失败");
        }
        paths.push(String::from_utf16_lossy(&buf[..len]));
    }
    let bytes = encode_hdrop(&paths);
    diagnostics.read(id, bytes.len());
    Ok(Some(bytes))
}
/// 无 IO 的格式选择；读取前用 size=0 排除冗余 DIB。
#[derive(Debug, PartialEq, Eq)]
enum FormatChoice {
    Keep,
    Redundant,
}
fn select_format(
    name: &str,
    size: usize,
    total: usize,
    has_v5: bool,
    limit: usize,
) -> std::result::Result<FormatChoice, CaptureLimit> {
    if name == "CF_DIB" && has_v5 {
        return Ok(FormatChoice::Redundant);
    }
    if size > limit || total.saturating_add(size) > limit {
        return Err(CaptureLimit {
            actual: total.saturating_add(size),
            limit,
            image: matches!(name, "CF_DIBV5" | "CF_DIB" | "PNG"),
        });
    }
    Ok(FormatChoice::Keep)
}
#[cfg(test)]
fn collect_formats(
    mut read: impl FnMut(&str) -> Result<Option<Vec<u8>>>,
    limit: usize,
) -> Result<Option<Clip>> {
    collect_formats_with_budget(|name, _| read(name), limit)
}
fn collect_formats_with_budget(
    mut read: impl FnMut(&str, usize) -> Result<Option<Vec<u8>>>,
    limit: usize,
) -> Result<Option<Clip>> {
    let mut clip = Clip::default();
    let mut total = 0usize;
    let mut invalid = false;
    let mut exceeded: Option<CaptureLimit> = None;
    for (name, _) in SUPPORTED {
        let has_v5 = clip.get("CF_DIBV5").is_some();
        if matches!(
            select_format(name, 0, total, has_v5, limit),
            Ok(FormatChoice::Redundant)
        ) {
            crate::diag::write(format!("capture skip format={name} reason=redundant_dib"));
            continue;
        }
        let bytes = match read(name, limit.saturating_sub(total)) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(e) if e.is::<CaptureLimit>() => {
                let mut skipped = e.downcast::<CaptureLimit>().unwrap();
                crate::diag::write(format!(
                    "capture skip format={name} bytes={} total={total} limit={limit} reason=read_limit",
                    skipped.actual
                ));
                skipped.actual = total.saturating_add(skipped.actual);
                skipped.limit = limit;
                if exceeded
                    .as_ref()
                    .is_none_or(|old| old.actual < skipped.actual)
                {
                    exceeded = Some(skipped);
                }
                continue;
            }
            Err(_) => {
                invalid |= name != "Preferred DropEffect";
                // 只记录固定格式名，不输出内容、路径或底层错误文本。
                eprintln!("剪贴板格式读取失败，已跳过：{name}");
                continue;
            }
        };
        if let Err(skipped) = select_format(name, bytes.len(), total, has_v5, limit) {
            crate::diag::write(format!("capture skip format={name} bytes={} total={total} limit={limit} reason=capture_limit", bytes.len()));
            if exceeded
                .as_ref()
                .is_none_or(|old| old.actual < skipped.actual)
            {
                exceeded = Some(skipped);
            }
            continue;
        }
        if bytes.is_empty() || (name == "Preferred DropEffect" && bytes.len() < 4) {
            invalid |= name != "Preferred DropEffect";
            eprintln!("剪贴板格式长度无效，已跳过：{name}");
            continue;
        }
        total += bytes.len();
        clip.formats.push(FormatData {
            name: name.into(),
            bytes: normalize(name, bytes),
        });
    }
    if clip.get("CF_HDROP").is_some()
        && clip.get("Preferred DropEffect").is_none()
        && total.saturating_add(4) <= limit
    {
        clip.formats.push(FormatData {
            name: "Preferred DropEffect".into(),
            bytes: 1u32.to_le_bytes().to_vec(),
        });
    }
    if clip
        .formats
        .iter()
        .all(|f| f.name == "Preferred DropEffect")
    {
        if let Some(exceeded) = exceeded {
            return Err(exceeded.into());
        }
        if invalid {
            return Err(CaptureError::Invalid.into());
        }
        return Ok(None);
    }
    remove_redundant_dib(&mut clip.formats);
    Ok(Some(clip))
}

/// 采集规范化发生在哈希之前；不改变旧记录的写回格式集合。
fn remove_redundant_dib(formats: &mut Vec<FormatData>) {
    if formats.iter().any(|f| f.name == "CF_DIBV5") {
        formats.retain(|f| f.name != "CF_DIB");
    }
}

struct Allocation(HGLOBAL);
impl Drop for Allocation {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            unsafe {
                let _ = GlobalFree(Some(self.0));
            }
        }
    }
}
fn allocate(bytes: &[u8]) -> Result<Allocation> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let owned = Allocation(h);
        let p = GlobalLock(h);
        if p.is_null() {
            bail!("剪贴板内存分配失败");
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
        let _ = GlobalUnlock(h);
        Ok(owned)
    }
}
fn write_formats(clip: &Clip, plain: bool) -> Result<Vec<&FormatData>> {
    if plain {
        return clip
            .formats
            .iter()
            .find(|f| f.name == "CF_UNICODETEXT")
            .map(|f| vec![f])
            .ok_or_else(|| anyhow!("该记录没有纯文本格式"));
    }
    let redundant_dib = clip.get("PNG").is_some() && clip.get("CF_DIBV5").is_some();
    Ok(clip
        .formats
        .iter()
        .filter(|f| {
            SUPPORTED.iter().any(|(name, _)| *name == f.name)
                && !(f.name == "Preferred DropEffect" && f.bytes.len() < 4)
                && !(redundant_dib && f.name == "CF_DIB")
        })
        .collect())
}
pub fn write(hwnd: HWND, clip: &Clip, plain: bool) -> Result<()> {
    let mut formats = Vec::new();
    for f in write_formats(clip, plain)? {
        // 普通格式直接借用解密结果，只向 HGLOBAL 拷贝一次；文件列表仍需规范化。
        let bytes = if f.name == "CF_HDROP" {
            Cow::Owned(encode_hdrop(
                &crate::model::decode_hdrop(&f.bytes).ok_or_else(|| anyhow!("文件记录无效"))?,
            ))
        } else {
            Cow::Borrowed(f.bytes.as_slice())
        };
        crate::diag::write(format!(
            "clipboard_write format={} bytes={}",
            f.name,
            bytes.len()
        ));
        formats.push((format_id(&f.name), allocate(&bytes)?));
    }
    let effect_id = format_id("Preferred DropEffect");
    if formats.iter().any(|(id, _)| *id == 15) && !formats.iter().any(|(id, _)| *id == effect_id) {
        crate::diag::write("clipboard_write format=Preferred DropEffect bytes=4".into());
        formats.push((effect_id, allocate(&1u32.to_le_bytes())?));
    }
    if formats.is_empty() {
        bail!("没有可写回的格式");
    }
    formats.insert(
        0,
        (
            format_id(OWNER_MARKER),
            allocate(TOKEN.get().ok_or_else(|| anyhow!("剪贴板未初始化"))?)?,
        ),
    );
    let guard = open(hwnd)?;
    unsafe {
        EmptyClipboard()?;
        // 写入成功后，HGLOBAL 所有权转交系统；失败时 RAII 释放未交付部分。
        for (id, mut data) in formats {
            SetClipboardData(id, Some(HANDLE(data.0 .0)))?;
            data.0 = HGLOBAL::default();
        }
    }
    SELF_SEQUENCE.store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
    drop(guard);
    Ok(())
}

// 以下为测试代码：在私有窗口站用 Win32 调用 capture/write，不接管键鼠、不改变用户桌面剪贴板。
#[cfg(test)]
mod system_tests {
    use super::*;
    use crate::model::{decode_hdrop, Clip, FormatData};
    use std::sync::{Mutex, MutexGuard};

    static CLIPBOARD: Mutex<()> = Mutex::new(());

    /// 切换进程窗口站是全局操作，系统剪贴板测试必须串行。
    fn lock() -> (
        crate::test_support::PrivateClipboard,
        MutexGuard<'static, ()>,
    ) {
        let lock = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
        let isolated = crate::test_support::PrivateClipboard::new();
        SELF_SEQUENCE.store(0, Ordering::Release);
        // 元组按字段顺序析构：先恢复原窗口站，再释放串行锁。
        (isolated, lock)
    }

    fn init() {
        // 允许重复调用：OnceLock 已初始化时返回 Err，忽略即可。
        let _ = initialize();
    }

    /// 无论测试成功还是 panic，都释放测试窗口站中的合成内容。
    struct Cleanup;

    impl Drop for Cleanup {
        fn drop(&mut self) {
            unsafe {
                if OpenClipboard(None).is_ok() {
                    let _ = EmptyClipboard();
                    let _ = CloseClipboard();
                }
            }
        }
    }

    fn text_bytes(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    /// 模拟其它程序写剪贴板：直接调用 Win32，不经过产品代码。
    fn set_formats(items: &[(&str, &[u8])]) {
        set_formats_with_owner(HWND::default(), items);
    }

    fn set_formats_with_owner(owner: HWND, items: &[(&str, &[u8])]) {
        unsafe {
            let guard = open(owner).expect("打开剪贴板失败");
            EmptyClipboard().expect("清空剪贴板失败");
            for (name, bytes) in items {
                let mut data = allocate(bytes).expect("分配剪贴板内存失败");
                SetClipboardData(format_id(name), Some(HANDLE(data.0 .0)))
                    .unwrap_or_else(|_| panic!("写入格式 {name} 失败"));
                // 所有权已交给系统，避免析构时释放。
                data.0 = HGLOBAL::default();
            }
            drop(guard);
        }
    }

    fn read_format(name: &str) -> Option<Vec<u8>> {
        unsafe {
            let _guard = open(HWND::default()).ok()?;
            read_raw(format_id(name), &mut Default::default())
                .ok()
                .flatten()
        }
    }

    /// 用系统 API 解析当前 CF_HDROP，等同于资源管理器/聊天软件读取文件列表。
    fn read_hdrop_paths() -> Vec<String> {
        unsafe {
            let _guard = open(HWND::default()).expect("打开剪贴板失败");
            let handle = GetClipboardData(15).expect("CF_HDROP 缺失");
            let drop = HDROP(handle.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            let mut out = Vec::new();
            for i in 0..count {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(drop, i, Some(&mut buf));
                out.push(String::from_utf16_lossy(&buf[..len]));
            }
            out
        }
    }

    #[test]
    fn ignored_owner_skips_all_payload_reads_and_settings_change_applies_next_capture() {
        use windows::{
            core::w,
            Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
            },
        };
        let _lock = lock();
        let _cleanup = Cleanup;
        struct Window(HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        let window = Window(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("合成来源窗口"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .unwrap()
        });
        let bytes = text_bytes("合成忽略测试");
        set_formats_with_owner(window.0, &[("CF_UNICODETEXT", &bytes)]);
        let owner = crate::clipboard_owner::clipboard_owner();
        assert!(owner.is_real());
        let apps = std::cell::RefCell::new(vec![owner.exe.to_uppercase()]);
        LAST_SEQUENCE.store(0, Ordering::Release);
        DATA_REQUESTS.with(|count| count.set(0));
        assert!(
            capture_pending_with_settings(HWND::default(), || apps.borrow().clone())
                .unwrap()
                .is_none()
        );
        assert_eq!(DATA_REQUESTS.with(|count| count.get()), 0);
        assert_eq!(LAST_SEQUENCE.load(Ordering::Acquire), unsafe {
            GetClipboardSequenceNumber()
        });
        // 已忽略的序列不补采；移除设置后下一次复制立即恢复，无需重启。
        apps.borrow_mut().clear();
        assert!(
            capture_pending_with_settings(HWND::default(), || apps.borrow().clone())
                .unwrap()
                .is_none()
        );
        set_formats_with_owner(window.0, &[("CF_UNICODETEXT", &bytes)]);
        let captured = capture_pending_with_settings(HWND::default(), || apps.borrow().clone())
            .unwrap()
            .unwrap();
        assert!(DATA_REQUESTS.with(|count| count.get()) > 0);
        assert_eq!(captured.source.exe.as_deref(), Some(owner.exe.as_str()));
        assert_eq!(captured.source.title.as_deref(), Some("合成来源窗口"));
        assert!(captured.context.owner.same_process_window(&owner));
        apps.borrow_mut().push(owner.exe);
        set_formats_with_owner(window.0, &[("CF_UNICODETEXT", &bytes)]);
        DATA_REQUESTS.with(|count| count.set(0));
        assert!(
            capture_pending_with_settings(HWND::default(), || apps.borrow().clone())
                .unwrap()
                .is_none()
        );
        assert_eq!(DATA_REQUESTS.with(|count| count.get()), 0);
    }

    #[test]
    fn owner_flush_private_clipboard_skips_without_touch_and_acknowledges_sequence() {
        use crate::{owner_flush::tests::dib, storage::Storage};
        use windows::{
            core::w,
            Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
            },
        };
        let _lock = lock();
        let _cleanup = Cleanup;
        LAST_SEQUENCE.store(0, Ordering::Release);
        // 只有消息窗口，不显示 UI、不激活窗口、不接管键鼠。
        struct Window(HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        let window = Window(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("owner-flush-test"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .unwrap()
        });
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!(
                "owner-flush-private-{}-{}",
                std::process::id(),
                crate::storage::unix_millis()
            ));
        let storage = Mutex::new(Storage::open(&path, &[89; 32]).unwrap());
        let context = Mutex::new(None);
        let first_dib = dib(19, 17, 24, false, None);
        let second_dib = dib(19, 17, 32, true, None);
        let text = text_bytes("合成文件附带文本");
        set_formats_with_owner(
            window.0,
            &[("CF_DIB", &first_dib), ("CF_UNICODETEXT", &text)],
        );
        let first = capture_pending(HWND::default()).unwrap().unwrap();
        assert!(first.context.owner.is_real());
        assert_eq!(first.context.owner.window, Some(window.0 .0 as isize));
        assert_eq!(
            crate::runtime::save_capture(&storage, &context, first).unwrap(),
            Some(true)
        );
        let before = storage.lock().unwrap().list().unwrap();
        assert_eq!(before.len(), 1);

        // Win32 的实际行为在私有窗口站验证，不假定 NULL 写入必然可用。
        set_formats(&[("CF_DIB", &second_dib)]);
        let second = capture_pending(HWND::default()).unwrap().unwrap();
        assert!(second.context.owner.is_ownerless());
        let sequence = second.sequence;
        assert_eq!(
            crate::runtime::save_capture(&storage, &context, second).unwrap(),
            None
        );
        assert_eq!(LAST_SEQUENCE.load(Ordering::Acquire), sequence);
        assert!(capture_pending(HWND::default()).unwrap().is_none());
        let after = storage.lock().unwrap().list().unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap()
        );

        // 清空上下文后，同样的无 owner 固化内容重新进入正常保存路径。
        *context.lock().unwrap() = None;
        storage
            .lock()
            .unwrap()
            .delete(&[before[0].id.clone()])
            .unwrap();
        set_formats(&[("CF_DIB", &second_dib)]);
        let after_delete = capture_pending(HWND::default()).unwrap().unwrap();
        assert_eq!(
            crate::runtime::save_capture(&storage, &context, after_delete).unwrap(),
            Some(true)
        );
        // 无 owner 连续提供不同图像时都应保存。
        let mut different = second_dib;
        different[40] ^= 1;
        set_formats(&[("CF_DIB", &different)]);
        let different = capture_pending(HWND::default()).unwrap().unwrap();
        assert_eq!(
            crate::runtime::save_capture(&storage, &context, different).unwrap(),
            Some(true)
        );
        assert_eq!(storage.lock().unwrap().list().unwrap().len(), 2);
        drop(storage);
        // 仅删除本测试创建且位于项目 .test-data 下的目录。
        let target = path.canonicalize().unwrap();
        let test_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .canonicalize()
            .unwrap();
        assert!(target.starts_with(&test_root));
        assert!(target
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("owner-flush-private-"));
        std::fs::remove_dir_all(target).unwrap();
    }

    #[test]
    fn multi_format_text_roundtrip() {
        let _lock = lock();
        init();
        let _cleanup = Cleanup;

        let text = "合成文本 A1";
        let html: &[u8] = b"Version:1.0\r\nStartHTML:0000000097\r\nEndHTML:0000000200\r\n<html><body><!--StartFragment--><b>synthetic</b><!--EndFragment--></body></html>";
        let rtf: &[u8] = b"{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Segoe UI;}}\\f0\\fs20 synthetic}";
        let text_raw = text_bytes(text);
        set_formats(&[
            ("CF_UNICODETEXT", text_raw.as_slice()),
            ("HTML Format", html),
            ("Rich Text Format", rtf),
        ]);

        let clip = capture(HWND::default())
            .expect("采集真实剪贴板失败")
            .expect("应采集到内容");
        assert_eq!(clip.kind(), "text");
        assert_eq!(clip.text().as_deref(), Some(text));
        assert_eq!(
            clip.get("CF_UNICODETEXT").unwrap().len(),
            (text.encode_utf16().count() + 1) * 2,
            "HGLOBAL 填充必须被裁掉，否则相同文本会产生不同哈希"
        );

        let captured_html = clip.get("HTML Format").expect("应保留 HTML Format");
        assert!(captured_html.starts_with(html));
        assert!(
            captured_html.len() <= html.len() + 1,
            "填充后的零字节应被裁掉"
        );
        let captured_rtf = clip.get("Rich Text Format").expect("应保留 RTF");
        assert!(captured_rtf.starts_with(rtf));

        // 同一剪贴板连续采集两次必须得到相同内容身份（去重前提）。
        let again = capture(HWND::default())
            .expect("采集真实剪贴板失败")
            .expect("应采集到内容");
        assert_eq!(clip.hash(), again.hash(), "相同内容两次采集的哈希必须一致");
    }

    #[test]
    fn files_roundtrip_through_real_clipboard() {
        let _lock = lock();
        init();
        let _cleanup = Cleanup;

        let paths = vec![
            r"C:\测试 数据\示例一.txt".to_string(),
            r"D:\另一处\二.png".to_string(),
        ];
        let hdrop = encode_hdrop(&paths);
        let copy = 2u32.to_le_bytes(); // DROPEFFECT_COPY
        set_formats(&[
            ("CF_HDROP", hdrop.as_slice()),
            ("Preferred DropEffect", copy.as_slice()),
        ]);

        let clip = capture(HWND::default())
            .expect("采集真实剪贴板失败")
            .expect("应采集到文件记录");
        assert_eq!(clip.kind(), "files");
        assert_eq!(
            decode_hdrop(clip.get("CF_HDROP").unwrap()).as_deref(),
            Some(paths.as_slice())
        );
        assert_eq!(
            clip.get("Preferred DropEffect"),
            Some(copy.as_slice()),
            "DropEffect 必须原样保留"
        );

        // 写回后必须能被系统 API 解析成真实文件列表。
        write(HWND::default(), &clip, false).expect("写回文件记录失败");
        assert_eq!(
            read_hdrop_paths(),
            paths,
            "写回后的 CF_HDROP 必须是合法文件列表"
        );
        assert_eq!(
            read_format("Preferred DropEffect").as_deref(),
            Some(copy.as_slice()),
            "写回时必须重建 Preferred DropEffect"
        );
        assert!(read_format(OWNER_MARKER).is_some(), "应写入自写回标记");

        // 自写回的内容不得被再次采集。
        assert!(
            capture(HWND::default())
                .expect("采集真实剪贴板失败")
                .is_none(),
            "自写回内容必须被识别并跳过"
        );
    }

    #[test]
    fn privacy_markers_stop_real_capture() {
        let _lock = lock();
        init();
        let _cleanup = Cleanup;
        let text = text_bytes("合成隐私标记内容");
        let empty: &[u8] = &[];

        set_formats(&[
            ("CF_UNICODETEXT", text.as_slice()),
            ("ExcludeClipboardContentFromMonitorProcessing", empty),
        ]);
        assert!(
            capture(HWND::default())
                .expect("采集真实剪贴板失败")
                .is_none(),
            "ExcludeClipboardContentFromMonitorProcessing 出现时必须整条忽略"
        );

        let zero = 0u32.to_le_bytes();
        set_formats(&[
            ("CF_UNICODETEXT", text.as_slice()),
            ("CanIncludeInClipboardHistory", zero.as_slice()),
        ]);
        assert!(
            capture(HWND::default())
                .expect("采集真实剪贴板失败")
                .is_none(),
            "CanIncludeInClipboardHistory=0 时必须忽略"
        );

        let one = 1u32.to_le_bytes();
        set_formats(&[
            ("CF_UNICODETEXT", text.as_slice()),
            ("CanIncludeInClipboardHistory", one.as_slice()),
        ]);
        let clip = capture(HWND::default())
            .expect("采集真实剪贴板失败")
            .expect("CanIncludeInClipboardHistory=1 时应正常记录");
        assert_eq!(clip.text().as_deref(), Some("合成隐私标记内容"));
        assert!(
            clip.get("CanIncludeInClipboardHistory").is_none(),
            "控制标记本身不应进入记录"
        );
        assert!(clip
            .get("ExcludeClipboardContentFromMonitorProcessing")
            .is_none());

        // 标记格式内容损坏（不足 4 字节）时按“不记录”处理（fail closed）。
        let short = [1u8];
        set_formats(&[
            ("CF_UNICODETEXT", text.as_slice()),
            ("CanIncludeInClipboardHistory", short.as_slice()),
        ]);
        assert!(
            capture(HWND::default())
                .expect("采集真实剪贴板失败")
                .is_none(),
            "标记内容不足 4 字节时必须不记录"
        );
    }

    #[test]
    fn dibv5_only_roundtrips_through_private_clipboard() {
        let _lock = lock();
        init();
        let _cleanup = Cleanup;
        let mut v5 = vec![0u8; 140];
        for (offset, value) in [
            (0, 124u32),
            (4, 2),
            (8, 2),
            (16, 3),
            (20, 16),
            (40, 0xff0000),
            (44, 0xff00),
            (48, 0xff),
            (52, 0xff000000),
            (56, 0x73524742),
        ] {
            v5[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        v5[12] = 1;
        v5[14] = 32;
        for pixel in v5[124..].as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[3, 2, 1, 128]);
        }
        let mut dib = v5[..40].to_vec();
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[16..20].copy_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&v5[124..]);
        set_formats(&[("CF_DIB", &dib), ("CF_DIBV5", &v5)]);
        let captured = capture(HWND::default()).unwrap().unwrap();
        assert_eq!(captured.get("CF_DIBV5").unwrap(), v5);
        assert!(captured.get("CF_DIB").is_none());
        write(HWND::default(), &captured, false).unwrap();
        assert_eq!(normalize("CF_DIBV5", read_format("CF_DIBV5").unwrap()), v5);
        unsafe {
            let _guard = open(HWND::default()).unwrap();
            assert!(GetClipboardData(8).is_ok(), "Windows 应自动合成 CF_DIB");
            assert!(GetClipboardData(2).is_ok(), "Windows 应自动合成 CF_BITMAP");
        }
        assert!(capture(HWND::default()).unwrap().is_none());
    }

    #[test]
    fn v5_and_dib_never_read_redundant_dib_from_private_clipboard() {
        let _lock = lock();
        init();
        let _cleanup = Cleanup;
        let mut v5 = vec![0u8; 128];
        for (offset, value) in [(0, 124u32), (4, 1), (8, 1), (20, 4)] {
            v5[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        v5[12] = 1;
        v5[14] = 32;
        let mut dib = vec![0u8; 44];
        dib[..40].copy_from_slice(&v5[..40]);
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        set_formats(&[("CF_DIBV5", &v5), ("CF_DIB", &dib)]);
        let _guard = open(HWND::default()).unwrap();
        let mut dib_reads = 0;
        let mut v5_reads = 0;
        let clip = collect_formats(
            |name| {
                dib_reads += usize::from(name == "CF_DIB");
                v5_reads += usize::from(name == "CF_DIBV5");
                unsafe { super::read_format(name) }
            },
            MAX_BYTES,
        )
        .unwrap()
        .unwrap();
        assert_eq!(v5_reads, 1);
        assert_eq!(dib_reads, 0);
        assert!(clip.get("CF_DIBV5").is_some());
        assert!(clip.get("CF_DIB").is_none());
    }

    #[test]
    fn plain_write_requires_text_and_write_is_owned() {
        let _lock = lock();
        init();
        let _cleanup = Cleanup;

        let files = Clip {
            formats: vec![FormatData {
                name: "CF_HDROP".into(),
                bytes: encode_hdrop(&[r"C:\x.txt".to_string()]),
            }],
        };
        // 纯文本粘贴需要 CF_UNICODETEXT，文件记录应报错而不是写入空内容。
        assert!(
            write(HWND::default(), &files, true).is_err(),
            "没有纯文本格式时必须返回错误"
        );

        let text = Clip {
            formats: vec![FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: text_bytes("写回测试"),
            }],
        };
        write(HWND::default(), &files.clone(), false).expect("写回文件记录失败");
        assert!(capture(HWND::default()).expect("采集失败").is_none());
        write(HWND::default(), &text, false).expect("写回文本失败");
        assert!(capture(HWND::default()).expect("采集失败").is_none());
        // 内容确实写进了剪贴板，但被自写回标记挡住。
        let raw = read_format("CF_UNICODETEXT").expect("剪贴板应包含文本格式");
        assert_eq!(
            Clip {
                formats: vec![FormatData {
                    name: "CF_UNICODETEXT".into(),
                    bytes: raw,
                }]
            }
            .text()
            .as_deref(),
            Some("写回测试")
        );
    }

    #[test]
    fn short_drop_effect_from_real_clipboard_defaults_to_copy() {
        let _lock = lock();
        init();
        let _cleanup = Cleanup;
        let hdrop = encode_hdrop(&[r"C:\synthetic.txt".to_string()]);
        // 模拟某些程序只写 2 字节 DropEffect：采集时必须丢弃而不是零填充成“剪切”。
        set_formats(&[
            ("CF_HDROP", hdrop.as_slice()),
            ("Preferred DropEffect", &[2u8, 0]),
        ]);
        assert_eq!(
            read_format("Preferred DropEffect").as_deref(),
            Some([2u8, 0].as_slice()),
            "GlobalSize 应返回实际请求的 2 字节，否则本测试的前提不成立"
        );
        let clip = capture(HWND::default())
            .expect("采集真实剪贴板失败")
            .expect("应采集到文件记录");
        assert_eq!(
            clip.get("Preferred DropEffect"),
            Some(1u32.to_le_bytes().as_slice()),
            "不足 4 字节的 DropEffect 必须被丢弃并默认复制，不能把填充零读成 DROPEFFECT_MOVE"
        );
        assert_eq!(
            decode_hdrop(clip.get("CF_HDROP").unwrap()).as_deref(),
            Some([r"C:\synthetic.txt".to_string()].as_slice())
        );
    }

    #[test]
    fn stored_short_drop_effect_is_not_written_back() {
        let _lock = lock();
        init();
        let _cleanup = Cleanup;
        for len in 0..4 {
            let clip = Clip {
                formats: vec![
                    FormatData {
                        name: "CF_HDROP".into(),
                        bytes: encode_hdrop(&[r"C:\synthetic.txt".into()]),
                    },
                    FormatData {
                        name: "Preferred DropEffect".into(),
                        bytes: vec![2; len],
                    },
                ],
            };
            write(HWND::default(), &clip, false).unwrap();
            assert_eq!(
                read_format("Preferred DropEffect").as_deref(),
                Some(1u32.to_le_bytes().as_slice())
            );
        }
    }

    #[test]
    fn marker_of_another_process_is_not_treated_as_self_write() {
        let _lock = lock();
        init();
        let _cleanup = Cleanup;
        // 其它程序如果恰好也写了同名格式（内容不同），不得影响采集。
        let foreign: &[u8] = b"another-app-token";
        let text = text_bytes("外来标记内容");
        set_formats(&[("CF_UNICODETEXT", text.as_slice()), (OWNER_MARKER, foreign)]);
        let clip = capture(HWND::default())
            .expect("采集真实剪贴板失败")
            .expect("外来标记不应阻止采集");
        assert_eq!(clip.text().as_deref(), Some("外来标记内容"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn privacy_markers_fail_closed() {
        assert!(may_record(false, None));
        assert!(!may_record(true, None));
        assert!(!may_record(false, Some(&[0, 0, 0, 0])));
        assert!(!may_record(false, Some(&[1])));
        assert!(may_record(false, Some(&[1, 0, 0, 0])));
    }
    #[test]
    fn text_padding_does_not_change_identity() {
        assert_eq!(
            normalize("CF_UNICODETEXT", vec![65, 0, 0, 0, 9, 9]),
            vec![65, 0, 0, 0]
        );
        assert_eq!(
            normalize("Rich Text Format", b"fixture\0padding".to_vec()),
            b"fixture\0"
        );
    }

    #[test]
    fn capture_normalizes_image_padding_before_hashing() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mut dib = vec![0u8; 44];
        dib[..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        for (name, bytes) in [("PNG", png.into_inner()), ("CF_DIB", dib)] {
            let capture = |padding| {
                collect_formats(
                    |requested| {
                        if requested != name {
                            return Ok(None);
                        }
                        let mut value = bytes.clone();
                        if padding {
                            value.extend([19; 71]);
                        }
                        Ok(Some(value))
                    },
                    MAX_BYTES,
                )
                .unwrap()
                .unwrap()
            };
            assert_eq!(capture(false).hash(), capture(true).hash());
            assert_eq!(capture(true).get(name), Some(bytes.as_slice()));
        }
    }

    #[test]
    fn failed_format_keeps_other_formats() {
        let clip = collect_formats(
            |name| match name {
                "HTML Format" | "CF_HDROP" => Err(anyhow!("合成读取失败，不应输出此错误文本")),
                "CF_UNICODETEXT" => Ok(Some(vec![65, 0, 0, 0])),
                "Rich Text Format" => Ok(Some(b"{\\rtf1 test}\0".to_vec())),
                _ => Ok(None),
            },
            MAX_BYTES,
        )
        .unwrap()
        .unwrap();
        assert_eq!(clip.text().as_deref(), Some("A"));
        assert_eq!(clip.formats.len(), 2);
        assert!(clip.get("Rich Text Format").is_some());
        assert!(clip.get("HTML Format").is_none());
        assert!(clip.get("CF_HDROP").is_none());
    }
    #[test]
    fn all_content_formats_failed_reports_invalid() {
        assert!(collect_formats(|_| Err(anyhow!("合成读取失败")), MAX_BYTES)
            .unwrap_err()
            .is::<CaptureError>());
        assert!(collect_formats(
            |name| {
                if name == "Preferred DropEffect" {
                    Ok(Some(vec![1, 0, 0, 0]))
                } else {
                    Err(anyhow!("合成读取失败"))
                }
            },
            MAX_BYTES
        )
        .unwrap_err()
        .is::<CaptureError>());
    }
    #[test]
    fn first_format_failure_keeps_remaining_formats() {
        // CF_UNICODETEXT 是循环里的第一个格式；它读取失败时，后面的格式仍必须保存。
        let clip = collect_formats(
            |name| match name {
                "CF_UNICODETEXT" => Err(anyhow!("合成读取失败，不应输出此错误文本")),
                "PNG" => Ok(Some(vec![0x89, b'P', b'N', b'G'])),
                "Rich Text Format" => Ok(Some(b"{\\rtf1 test}\0".to_vec())),
                _ => Ok(None),
            },
            MAX_BYTES,
        )
        .unwrap()
        .unwrap();
        assert!(clip.get("CF_UNICODETEXT").is_none());
        assert_eq!(clip.get("PNG"), Some([0x89, b'P', b'N', b'G'].as_slice()));
        assert!(clip.get("Rich Text Format").is_some());
        assert_eq!(clip.kind(), "image");
    }
    #[test]
    fn capture_limit_after_success_keeps_other_formats() {
        // 超限格式不应阻止已有的有效内容。
        let error = collect_formats(
            |name| match name {
                "CF_UNICODETEXT" => Ok(Some(vec![65, 0, 0, 0])),
                "HTML Format" => Err(CaptureLimit {
                    actual: MAX_BYTES + 1,
                    limit: MAX_BYTES,
                    image: false,
                }
                .into()),
                _ => Ok(None),
            },
            MAX_BYTES,
        )
        .unwrap()
        .unwrap();
        assert_eq!(error.text().as_deref(), Some("A"));
    }
    #[test]
    fn total_limit_keeps_formats_within_budget() {
        // 小预算验证边界，避免为测试分配 512 MiB。
        let read = |name: &str| Ok(matches!(name, "HTML Format" | "PNG").then(|| vec![1; 4]));
        assert!(collect_formats(read, 8).unwrap().is_some());
        assert_eq!(collect_formats(read, 7).unwrap().unwrap().formats.len(), 1);
        assert!(collect_formats(
            |_| Err(CaptureLimit {
                actual: MAX_BYTES + 1,
                limit: MAX_BYTES,
                image: false
            }
            .into()),
            MAX_BYTES
        )
        .unwrap_err()
        .is::<CaptureLimit>());
    }
    #[test]
    fn short_drop_effect_is_discarded_and_files_default_to_copy() {
        for len in 0..4 {
            let clip = collect_formats(
                |name| {
                    Ok(match name {
                        "CF_HDROP" => Some(encode_hdrop(&[r"C:\synthetic.txt".into()])),
                        "Preferred DropEffect" => Some(vec![2; len]),
                        _ => None,
                    })
                },
                MAX_BYTES,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                clip.get("Preferred DropEffect"),
                Some(1u32.to_le_bytes().as_slice())
            );
            assert!(collect_formats(
                |name| Ok((name == "Preferred DropEffect").then(|| vec![2; len])),
                MAX_BYTES
            )
            .unwrap()
            .is_none());
        }
    }
    #[test]
    fn valid_drop_effect_retains_semantics_and_trims_padding() {
        for effect in [1u32, 2] {
            let mut bytes = effect.to_le_bytes().to_vec();
            bytes.extend_from_slice(&[9, 9]);
            let clip = collect_formats(
                |name| {
                    Ok(match name {
                        "CF_HDROP" => Some(encode_hdrop(&[r"C:\synthetic.txt".into()])),
                        "Preferred DropEffect" => Some(bytes.clone()),
                        _ => None,
                    })
                },
                MAX_BYTES,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                clip.get("Preferred DropEffect"),
                Some(effect.to_le_bytes().as_slice())
            );
        }
    }
}

#[cfg(test)]
mod write_selection_tests {
    use super::*;
    fn clip(names: &[&str]) -> Clip {
        Clip {
            formats: names
                .iter()
                .map(|name| FormatData {
                    name: (*name).into(),
                    bytes: vec![1; 4],
                })
                .collect(),
        }
    }
    fn names(clip: &Clip, plain: bool) -> Vec<&str> {
        write_formats(clip, plain)
            .unwrap()
            .iter()
            .map(|f| f.name.as_str())
            .collect()
    }
    #[test]
    fn png_and_dibv5_omit_dib_without_changing_stored_formats() {
        let clip = clip(&["CF_DIB", "PNG", "CF_DIBV5"]);
        assert_eq!(names(&clip, false), ["PNG", "CF_DIBV5"]);
        assert_eq!(clip.formats.len(), 3);
        assert!(std::ptr::eq(
            write_formats(&clip, false).unwrap()[0],
            &clip.formats[1]
        ));
    }
    #[test]
    fn dib_is_preserved_when_png_or_dibv5_is_missing() {
        for formats in [
            &["CF_DIB"][..],
            &["PNG", "CF_DIB"][..],
            &["CF_DIBV5", "CF_DIB"][..],
        ] {
            assert_eq!(names(&clip(formats), false), formats);
        }
    }
    #[test]
    fn plain_write_selects_only_text_or_fails() {
        assert_eq!(
            names(
                &clip(&["PNG", "CF_DIBV5", "CF_DIB", "CF_UNICODETEXT"]),
                true
            ),
            ["CF_UNICODETEXT"]
        );
        assert!(write_formats(&clip(&["PNG"]), true).is_err());
    }
    #[test]
    fn selection_filters_unsupported_and_short_drop_effect() {
        let mut clip = clip(&["unknown", "CF_UNICODETEXT", "Preferred DropEffect"]);
        clip.formats[2].bytes.truncate(3);
        assert_eq!(names(&clip, false), ["CF_UNICODETEXT"]);
    }
    fn image_formats(names: &[&str]) -> Vec<FormatData> {
        names
            .iter()
            .map(|name| FormatData {
                name: (*name).into(),
                bytes: vec![1, 2, 3, 4],
            })
            .collect()
    }
    #[test]
    fn redundant_dib_filter_preserves_other_formats_and_order() {
        let mut formats = image_formats(&["CF_DIB", "PNG", "CF_DIBV5", "HTML Format"]);
        let expected = image_formats(&["PNG", "CF_DIBV5", "HTML Format"]);
        remove_redundant_dib(&mut formats);
        assert_eq!(formats, expected);
        remove_redundant_dib(&mut formats);
        assert_eq!(formats, expected);
        for names in [&["CF_DIB"][..], &["CF_DIBV5"], &["PNG", "CF_DIB"], &[]] {
            let mut formats = image_formats(names);
            let original = formats.clone();
            remove_redundant_dib(&mut formats);
            assert_eq!(formats, original);
        }
    }
    #[test]
    fn capture_removes_dib_before_identity_is_calculated() {
        let capture =
            |both| {
                collect_formats(
                    |name| {
                        Ok((name == "CF_DIBV5" || (both && name == "CF_DIB"))
                            .then(|| vec![1, 2, 3, 4]))
                    },
                    MAX_BYTES,
                )
                .unwrap()
                .unwrap()
            };
        let both = capture(true);
        assert_eq!(both.formats, image_formats(&["CF_DIBV5"]));
        assert_eq!(both.hash(), capture(false).hash());
        assert_eq!(both.dedup_key(), capture(false).dedup_key());
    }
    #[test]
    fn dibv5_only_write_and_legacy_format_lists() {
        for (names, expected) in [
            (vec!["CF_DIBV5"], vec!["CF_DIBV5"]),
            (vec!["CF_DIBV5", "CF_DIB"], vec!["CF_DIBV5", "CF_DIB"]),
            (vec!["PNG", "CF_DIBV5", "CF_DIB"], vec!["PNG", "CF_DIBV5"]),
        ] {
            let clip = Clip {
                formats: image_formats(&names),
            };
            let selected = write_formats(&clip, false).unwrap();
            assert_eq!(
                selected.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[cfg(test)]
mod large_capture_tests {
    use super::*;
    fn choose(entries: &[(&str, usize)], limit: usize) -> Vec<String> {
        let mut total = 0;
        let mut v5 = false;
        entries
            .iter()
            .filter_map(|&(name, size)| {
                if !matches!(
                    select_format(name, size, total, v5, limit),
                    Ok(FormatChoice::Keep)
                ) {
                    return None;
                }
                total += size;
                v5 |= name == "CF_DIBV5";
                Some(name.to_owned())
            })
            .collect()
    }
    #[test]
    fn pure_selection_prefers_v5() {
        assert_eq!(
            choose(&[("CF_DIBV5", 14), ("CF_DIB", 10)], 20),
            ["CF_DIBV5"]
        );
    }
    #[test]
    fn pure_selection_falls_back_to_dib() {
        assert_eq!(choose(&[("CF_DIBV5", 21), ("CF_DIB", 10)], 20), ["CF_DIB"]);
    }
    #[test]
    fn pure_selection_keeps_text_when_images_exceed_limit() {
        assert_eq!(
            choose(
                &[("CF_UNICODETEXT", 4), ("CF_DIBV5", 21), ("CF_DIB", 22)],
                20
            ),
            ["CF_UNICODETEXT"]
        );
    }
    #[test]
    fn all_oversize_reports_actual_size() {
        let mib = 1024 * 1024;
        assert!(choose(&[("CF_DIBV5", 600 * mib), ("CF_DIB", 550 * mib)], MAX_BYTES).is_empty());
        let pure_error = select_format("CF_DIBV5", 600 * mib, 0, false, MAX_BYTES).unwrap_err();
        assert_eq!(
            pure_error.to_string(),
            "图片过大（约 600MB），超过 512MB 上限，未记录"
        );
        let error = collect_formats(
            |name| match name {
                "CF_DIBV5" | "CF_DIB" => Err(CaptureLimit {
                    actual: if name == "CF_DIBV5" {
                        600 * mib
                    } else {
                        550 * mib
                    },
                    limit: MAX_BYTES,
                    image: true,
                }
                .into()),
                _ => Ok(None),
            },
            MAX_BYTES,
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<CaptureLimit>().unwrap().actual,
            600 * mib
        );
        assert_eq!(
            error.to_string(),
            "图片过大（约 600MB），超过 512MB 上限，未记录"
        );
    }
    #[test]
    fn oversize_v5_read_error_falls_back_before_normalizing() {
        let clip = collect_formats(
            |name| match name {
                "CF_DIBV5" => Err(CaptureLimit {
                    actual: 21,
                    limit: 20,
                    image: true,
                }
                .into()),
                "CF_DIB" => Ok(Some(vec![0; 10])),
                _ => Ok(None),
            },
            20,
        )
        .unwrap()
        .unwrap();
        assert!(clip.get("CF_DIBV5").is_none());
        assert_eq!(clip.get("CF_DIB").unwrap().len(), 10);
    }
    #[test]
    fn remaining_budget_is_passed_without_reading_redundant_dib() {
        let mut calls = Vec::new();
        let clip = collect_formats_with_budget(
            |name, budget| {
                calls.push((name.to_owned(), budget));
                Ok(match name {
                    "CF_UNICODETEXT" => Some(vec![65, 0, 0, 0]),
                    "CF_DIBV5" => Some(vec![0; 10]),
                    _ => None,
                })
            },
            20,
        )
        .unwrap()
        .unwrap();
        assert_eq!(clip.formats.len(), 2);
        assert!(calls.contains(&("CF_DIBV5".into(), 16)));
        assert!(!calls.iter().any(|(name, _)| name == "CF_DIB"));
    }
    #[test]
    fn default_drop_effect_respects_total_budget() {
        let paths = encode_hdrop(&[r"C:\synthetic.txt".into()]);
        let clip = collect_formats(
            |name| Ok((name == "CF_HDROP").then(|| paths.clone())),
            paths.len(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(clip.formats.len(), 1);
        assert_eq!(clip.get("CF_HDROP"), Some(paths.as_slice()));
        // 写回层已有默认复制语义，采集层不为合成的四字节越过预算。
        assert!(clip.get("Preferred DropEffect").is_none());
    }
    #[test]
    fn all_image_errors_keep_text() {
        let clip = collect_formats(
            |name| match name {
                "CF_DIBV5" | "CF_DIB" | "PNG" => Err(CaptureLimit {
                    actual: 21,
                    limit: 20,
                    image: true,
                }
                .into()),
                "CF_UNICODETEXT" => Ok(Some(vec![65, 0, 0, 0])),
                _ => Ok(None),
            },
            20,
        )
        .unwrap()
        .unwrap();
        assert_eq!(clip.text().as_deref(), Some("A"));
    }
}
