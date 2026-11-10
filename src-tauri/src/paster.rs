use crate::{
    diag,
    hook::{MODIFIER_KEYS, PHYSICAL},
    platform,
};
use anyhow::{bail, Result};
use std::{
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::CloseHandle,
    Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
    System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

fn elevated(raw: isize) -> Result<bool> {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(platform::hwnd(raw), Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?;
        let mut token = Default::default();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        let _ = CloseHandle(process);
        opened?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        let _ = CloseHandle(token);
        result?;
        Ok(elevation.TokenIsElevated != 0)
    }
}
pub fn prepare(target: isize, main: isize, restore_focus: bool) -> Result<()> {
    if !platform::valid_external(target) {
        bail!("原目标窗口已失效");
    }
    if elevated(target)? && !elevated(main)? {
        bail!("目标窗口以管理员权限运行，当前进程不能向其发送按键");
    }
    if restore_focus {
        unsafe {
            if !SetForegroundWindow(platform::hwnd(target)).as_bool() {
                bail!("Windows 拒绝恢复目标窗口焦点");
            }
        }
    }
    for _ in 0..10 {
        if platform::foreground() == target {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    bail!("前台窗口已变化，已取消粘贴")
}
fn key(vk: u16, down: bool) -> INPUT {
    let extended = matches!(vk, 0xA3 | 0xA5 | 0x5B | 0x5C);
    let mut flags = if down {
        KEYBD_EVENT_FLAGS(0)
    } else {
        KEYEVENTF_KEYUP
    };
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0x434d3031,
            },
        },
    }
}
fn send(events: &[INPUT], purpose: &str) -> u32 {
    let sent = unsafe { SendInput(events, std::mem::size_of::<INPUT>() as i32) };
    diag::write(format!(
        "send_input purpose={purpose} requested={} returned={sent}",
        events.len()
    ));
    sent
}
pub fn logical() -> u16 {
    let mut bits = 0;
    for (i, k) in MODIFIER_KEYS.iter().enumerate() {
        if unsafe { GetAsyncKeyState(*k as i32) } < 0 {
            bits |= 1 << i;
        }
    }
    bits
}
#[derive(Debug, PartialEq, Eq)]
pub enum ReleaseDecision {
    Ready,
    Wait,
    Timeout,
}
pub fn release_decision(logical: u16, physical: u16, elapsed: Duration) -> ReleaseDecision {
    if (logical | physical) & 0xff == 0 {
        ReleaseDecision::Ready
    } else if elapsed >= Duration::from_secs(1) {
        ReleaseDecision::Timeout
    } else {
        ReleaseDecision::Wait
    }
}
pub fn wait_for_release() -> Result<()> {
    let start = Instant::now();
    loop {
        match release_decision(logical(), PHYSICAL.load(Ordering::SeqCst), start.elapsed()) {
            ReleaseDecision::Ready => return Ok(()),
            ReleaseDecision::Timeout => bail!("请松开修饰键后重试"),
            ReleaseDecision::Wait => thread::sleep(Duration::from_millis(5)),
        }
    }
}
pub fn mask_events() -> [INPUT; 2] {
    [key(0xE8, true), key(0xE8, false)]
}
pub fn mask_shortcut() -> Result<()> {
    let sent = send(&mask_events(), "mask");
    if sent == 1 {
        let _ = send(&[key(0xE8, false)], "mask_partial_release");
    }
    if sent != 2 {
        bail!("快捷键掩码发送失败，请松开修饰键后重试");
    }
    Ok(())
}
pub fn paste_events() -> [INPUT; 4] {
    [
        key(0xA2, true),
        key(0x56, true),
        key(0x56, false),
        key(0xA2, false),
    ]
}
pub fn inject(target: isize) -> Result<()> {
    // 等待在写回剪贴板前完成；这里再次检查，避免准备期间用户又按下修饰键。
    if release_decision(logical(), PHYSICAL.load(Ordering::SeqCst), Duration::ZERO)
        != ReleaseDecision::Ready
    {
        bail!("请松开修饰键后重试");
    }
    if platform::foreground() != target {
        bail!("前台窗口已变化，已取消按键注入");
    }
    diag::snapshot("before_send");
    let sent = send(&paste_events(), "paste");
    // 部分发送时仅释放本批次已按下的键，不重按物理修饰键、不做状态修复循环。
    if sent > 0 && sent < 4 {
        let cleanup = if sent == 2 {
            vec![key(0x56, false), key(0xA2, false)]
        } else {
            vec![key(0xA2, false)]
        };
        let _ = send(&cleanup, "partial_release");
    }
    diag::snapshot("after_send");
    if sent != 4 {
        bail!("SendInput 未完整发送；可能受到目标权限或桌面限制");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mask_is_an_unassigned_paired_key() {
        for (event, up) in mask_events().iter().zip([false, true]) {
            assert_eq!(event.r#type, INPUT_KEYBOARD);
            let keyboard = unsafe { event.Anonymous.ki };
            assert_eq!(keyboard.wVk.0, 0xE8);
            assert_eq!(keyboard.dwFlags.contains(KEYEVENTF_KEYUP), up);
            assert!(!keyboard.dwFlags.contains(KEYEVENTF_EXTENDEDKEY));
            assert_eq!(keyboard.dwExtraInfo, 0x434d3031);
        }
    }
    #[test]
    fn paste_is_only_one_balanced_control_v() {
        let actual: Vec<_> = paste_events()
            .iter()
            .map(|event| {
                let keyboard = unsafe { event.Anonymous.ki };
                (keyboard.wVk.0, !keyboard.dwFlags.contains(KEYEVENTF_KEYUP))
            })
            .collect();
        assert_eq!(
            actual,
            [(0xA2, true), (0x56, true), (0x56, false), (0xA2, false)]
        );
    }
}
