//! 主窗口粘贴恢复到呼出前的输入子窗口；只观察，不强制设置子焦点。
use super::{PasteIntent, PasteStatus};
use crate::{diag, platform};
use anyhow::{bail, Result};
use std::{
    thread,
    time::{Duration, Instant},
};
use windows::Win32::UI::WindowsAndMessaging::*;

const STABLE_FOR: Duration = Duration::from_millis(30);
const MAX_WAIT: Duration = Duration::from_millis(300);
const SAMPLE_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    raw: isize,
    pid: u32,
    tid: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct Target {
    window: isize,
    focus: Identity,
}
impl Target {
    pub fn window(&self) -> isize {
        self.window
    }
}

fn identity(raw: isize) -> Option<Identity> {
    if raw == 0 {
        return None;
    }
    let mut pid = 0;
    let tid = unsafe { GetWindowThreadProcessId(platform::hwnd(raw), Some(&mut pid)) };
    (tid != 0).then_some(Identity { raw, pid, tid })
}
fn focused(target: isize) -> Option<Identity> {
    let target_identity = identity(target)?;
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetGUIThreadInfo(target_identity.tid, &mut info).ok()?;
    }
    identity(info.hwndFocus.0 as isize)
}
pub fn capture(window: isize) -> Option<Target> {
    if platform::foreground() != window {
        return None;
    }
    let target = Target {
        window,
        focus: focused(window)?,
    };
    if !valid(target) {
        return None;
    }
    if diag::enabled() {
        diag::write(format!(
            "focus_restore captured target=0x{window:X} focus=0x{:X} pid={} tid={}",
            target.focus.raw, target.focus.pid, target.focus.tid
        ));
    }
    Some(target)
}
fn valid(target: Target) -> bool {
    identity(target.focus.raw) == Some(target.focus)
        && unsafe { GetAncestor(platform::hwnd(target.focus.raw), GA_ROOT) }
            == platform::hwnd(target.window)
}
struct Sample {
    elapsed: Duration,
    foreground: isize,
    focused: Option<Identity>,
    valid: bool,
}
#[derive(Default)]
struct Stability {
    since: Option<Duration>,
}
impl Stability {
    fn ready(&mut self, matches: bool, elapsed: Duration) -> bool {
        if !matches {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(elapsed);
        elapsed.saturating_sub(since) >= STABLE_FOR
    }
}
fn wait_with(
    target: Target,
    intent: &PasteIntent<'_>,
    mut sample: impl FnMut() -> Sample,
    mut pause: impl FnMut(),
) -> Result<PasteStatus> {
    let mut stability = Stability::default();
    loop {
        if !intent.is_current() {
            return Ok(PasteStatus::Superseded);
        }
        let current = sample();
        if current.foreground != target.window {
            bail!("前台窗口已变化，已取消粘贴");
        }
        if !current.valid {
            bail!("原输入窗口已变化，请重新打开主面板后粘贴");
        }
        if current.elapsed >= MAX_WAIT {
            bail!("原输入框焦点尚未恢复，请重新尝试粘贴");
        }
        if stability.ready(current.focused == Some(target.focus), current.elapsed) {
            return Ok(PasteStatus::Complete);
        }
        pause();
    }
}
pub fn wait_ready(target: Target, intent: &PasteIntent<'_>) -> Result<PasteStatus> {
    let start = Instant::now();
    let result = wait_with(
        target,
        intent,
        || Sample {
            elapsed: start.elapsed(),
            foreground: platform::foreground(),
            focused: focused(target.window),
            valid: valid(target),
        },
        || thread::sleep(SAMPLE_INTERVAL),
    );
    if diag::enabled() {
        let outcome = match &result {
            Ok(PasteStatus::Complete) => "ready",
            Ok(PasteStatus::Superseded) => "superseded",
            Err(_) => "cancelled",
        };
        diag::write(format!("focus_restore generation={} target=0x{:X} expected=0x{:X} elapsed_ms={} result={outcome}",intent.expected,target.window,target.focus.raw,start.elapsed().as_millis()));
    }
    result
}
pub fn verify(target: Target) -> Result<()> {
    if platform::foreground() != target.window
        || !valid(target)
        || focused(target.window) != Some(target.focus)
    {
        bail!("输入框焦点已变化，已取消按键注入");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
