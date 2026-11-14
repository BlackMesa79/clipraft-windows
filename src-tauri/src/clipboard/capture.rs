//! 单轮原生采集；序列与来源门仍先于任何载荷读取。
use super::{
    collection::collect_formats_with_budget, format_id, may_record, read_format_limited, read_raw,
    snapshot, Captured, OpenGuard, LAST_SEQUENCE, MAX_BYTES, OWNER_MARKER, SELF_SEQUENCE, TOKEN,
};
use crate::capture_policy::CaptureError;
use crate::{clipboard_diag::CaptureLog, owner_flush::CaptureContext};
use anyhow::Result;
use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use windows::Win32::{Foundation::HWND, System::DataExchange::*};

pub(super) fn capture_at(
    hwnd: HWND,
    expected: u32,
    ignored_apps: &[String],
) -> Result<Option<Captured>> {
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
    let mut diagnostics = CaptureLog::begin();
    unsafe { capture_open(expected, ignored_apps, &mut diagnostics) }
}

unsafe fn capture_open(
    expected: u32,
    ignored_apps: &[String],
    diagnostics: &mut CaptureLog,
) -> Result<Option<Captured>> {
    // 打开前仍可能换序列，不能把新内容算作旧轮次采集。
    let current = snapshot();
    if current.self_write || current.sequence != expected {
        return Ok(None);
    }
    let formats = offered_formats();
    let owner = crate::clipboard_owner::clipboard_owner(crate::diag::write);
    let source = crate::capture_source::capture(&owner);
    if crate::capture_source::ignored(source.exe.as_deref(), ignored_apps) {
        crate::diag::write(format!(
            "capture sequence={expected} result=skipped reason=ignored_app"
        ));
        return Ok(None);
    }
    let bitmap_offered = bitmap_offered(formats.as_ref());
    if !accepts_content(current.sequence, diagnostics) {
        return Ok(None);
    }
    let clip = collect_formats_with_budget(
        |name, budget| read_format_limited(name, budget, diagnostics),
        MAX_BYTES,
    )?;
    Ok(clip.map(|clip| Captured {
        clip,
        source,
        sequence: expected,
        context: CaptureContext {
            record_id: None,
            formats,
            owner,
            fingerprint: None,
            bitmap_offered,
        },
    }))
}

unsafe fn accepts_content(sequence: u32, diagnostics: &mut CaptureLog) -> bool {
    if IsClipboardFormatAvailable(format_id("ExcludeClipboardContentFromMonitorProcessing")).is_ok()
    {
        return false;
    }
    let history = match read_raw(format_id("CanIncludeInClipboardHistory"), diagnostics) {
        Ok(history) => history,
        Err(_) => {
            eprintln!("剪贴板隐私标记不可读，已跳过记录");
            return false;
        }
    };
    if !may_record(false, history.as_deref()) {
        return false;
    }
    let marker = read_raw(format_id(OWNER_MARKER), diagnostics).unwrap_or_else(|_| {
        eprintln!("剪贴板所有权标记不可读，已跳过该标记");
        None
    });
    if marker.is_some_and(|bytes| TOKEN.get().is_some_and(|token| bytes.starts_with(token))) {
        // 延迟渲染等操作可能推进序列，所有权标记也可确认自写回。
        SELF_SEQUENCE.store(sequence, Ordering::Release);
        return false;
    }
    true
}

fn bitmap_offered(formats: Option<&BTreeSet<u32>>) -> bool {
    formats.is_none_or(|ids| {
        [2, 8, 17, format_id("PNG"), format_id("image/png")]
            .iter()
            .any(|id| ids.contains(id))
    })
}

/// 调用方持有剪贴板；读取载荷之前保留完整的已提供格式集合。
fn offered_formats() -> Option<BTreeSet<u32>> {
    use windows::Win32::Foundation::{GetLastError, SetLastError, ERROR_SUCCESS};
    let mut formats = BTreeSet::new();
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
