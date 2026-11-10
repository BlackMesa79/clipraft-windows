use anyhow::{anyhow, Result};
use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering},
    thread,
};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Input::KeyboardAndMouse::GetAsyncKeyState, WindowsAndMessaging::*},
};

// 修改这些常量可配置快速面板快捷键；M0 支持 Ctrl、Shift 组合。
pub const TRIGGER_VK: u32 = 0x56;
pub const REQUIRE_CTRL: bool = true;
pub const REQUIRE_SHIFT: bool = true;
pub const ACTION_MESSAGE: u32 = WM_APP + 80;
pub const MODIFIER_KEYS: [u16; 8] = [0xA2, 0xA3, 0xA0, 0xA1, 0xA4, 0xA5, 0x5B, 0x5C];
pub static PHYSICAL: AtomicU16 = AtomicU16::new(0);
pub static READY: AtomicBool = AtomicBool::new(false);
static RECEIVER: AtomicU32 = AtomicU32::new(0);
static THREAD: AtomicU32 = AtomicU32::new(0);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Action {
    Open = 1,
    Next = 2,
    Previous = 3,
    Commit = 4,
    Cancel = 5,
}
impl Action {
    pub fn from_raw(n: usize) -> Option<Self> {
        match n {
            1 => Some(Self::Open),
            2 => Some(Self::Next),
            3 => Some(Self::Previous),
            4 => Some(Self::Commit),
            5 => Some(Self::Cancel),
            _ => None,
        }
    }
}
#[derive(Clone, Copy)]
pub struct Machine {
    pub active: bool,
    down: [bool; 256],
    swallowed: [bool; 256],
}
impl Default for Machine {
    fn default() -> Self {
        Self {
            active: false,
            down: [false; 256],
            swallowed: [false; 256],
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Decision {
    pub swallow: bool,
    pub action: Option<Action>,
}
impl Machine {
    pub fn step(&mut self, key: u32, down: bool, injected: bool, enabled: bool) -> Decision {
        let mut out = Decision {
            swallow: false,
            action: None,
        };
        if injected || key > 255 {
            return out;
        }
        let i = key as usize;
        let repeated = self.down[i];
        self.down[i] = down;
        if !down {
            out.swallow = self.swallowed[i];
            self.swallowed[i] = false;
        }
        let ctrl = self.down[0xA2] || self.down[0xA3] || self.down[0x11];
        let shift = self.down[0xA0] || self.down[0xA1] || self.down[0x10];
        let modifiers = (!REQUIRE_CTRL || ctrl) && (!REQUIRE_SHIFT || shift);
        if self.active {
            if !modifiers {
                self.active = false;
                out.action = Some(Action::Commit);
            } else if down {
                out.action = match key {
                    0x1B => {
                        self.active = false;
                        Some(Action::Cancel)
                    }
                    k if k == TRIGGER_VK && !repeated => Some(Action::Next),
                    0x28 => Some(Action::Next),
                    0x26 => Some(Action::Previous),
                    _ => None,
                };
                out.swallow = out.action.is_some();
            }
        } else if enabled && down && !repeated && key == TRIGGER_VK && modifiers {
            self.active = true;
            out.action = Some(Action::Open);
            out.swallow = true;
        }
        // 已吞掉首次按下的键，关闭面板后仍吞掉其重复按下，直到配对释放。
        if down && self.swallowed[i] {
            out.swallow = true;
        }
        // 面板打开前已传给目标的按下，必须让后续释放继续传给目标。
        if down && out.swallow && !repeated {
            self.swallowed[i] = true;
        }
        out
    }
    pub fn seed(&mut self, bits: u16) {
        for (i, k) in MODIFIER_KEYS.iter().enumerate() {
            self.down[*k as usize] = bits & (1 << i) != 0;
        }
    }
}
thread_local! {static MACHINE:Cell<Machine>=Cell::new(Machine::default());}
unsafe extern "system" fn callback(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, w, l);
    }
    let data = &*(l.0 as *const KBDLLHOOKSTRUCT);
    let down = w.0 == WM_KEYDOWN as usize || w.0 == WM_SYSKEYDOWN as usize;
    if data.flags.contains(LLKHF_INJECTED) {
        crate::diag::keyboard(data.vkCode, down, true, false, None);
        return CallNextHookEx(None, code, w, l);
    }
    if let Some(i) = MODIFIER_KEYS.iter().position(|k| *k as u32 == data.vkCode) {
        if down {
            PHYSICAL.fetch_or(1 << i, Ordering::SeqCst);
        } else {
            PHYSICAL.fetch_and(!(1 << i), Ordering::SeqCst);
        }
    }
    // 线程局部固定大小状态，无分配、锁、IO、UI 或同步等待。
    let swallow = MACHINE.with(|slot| {
        let mut state = slot.get();
        let decision = state.step(data.vkCode, down, false, READY.load(Ordering::Relaxed));
        let mut swallow = decision.swallow;
        crate::diag::panel(true, state.active);
        if let Some(action) = decision.action {
            if PostThreadMessageW(
                RECEIVER.load(Ordering::Relaxed),
                ACTION_MESSAGE,
                WPARAM(action as usize),
                LPARAM(0),
            )
            .is_err()
            {
                state.active = false;
                // 投递失败时让本次按下和后续释放都透传，避免留下不成对按键。
                if (data.vkCode as usize) < state.swallowed.len() {
                    state.swallowed[data.vkCode as usize] = false;
                }
                swallow = false;
            }
        }
        crate::diag::panel(true, state.active);
        crate::diag::keyboard(data.vkCode, down, false, swallow, decision.action);
        slot.set(state);
        swallow
    });
    if swallow {
        LRESULT(1)
    } else {
        CallNextHookEx(None, code, w, l)
    }
}
pub fn start(receiver: u32) -> Result<()> {
    RECEIVER.store(receiver, Ordering::Release);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(move || unsafe {
            let mut bits = 0;
            for (i, k) in MODIFIER_KEYS.iter().enumerate() {
                if GetAsyncKeyState(*k as i32) < 0 {
                    bits |= 1 << i;
                }
            }
            PHYSICAL.store(bits, Ordering::SeqCst);
            MACHINE.with(|s| {
                let mut m = s.get();
                m.seed(bits);
                s.set(m);
            });
            let module = GetModuleHandleW(None).ok().map(|m| m.into());
            match SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), module, 0) {
                Ok(hook) => {
                    let mut msg = MSG::default();
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                    THREAD.store(GetCurrentThreadId(), Ordering::Release);
                    let _ = tx.send(Ok(()));
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWindowsHookEx(hook);
                }
                Err(_) => {
                    let _ = tx.send(Err(anyhow!("键盘钩子安装失败")));
                }
            }
        })?;
    rx.recv().map_err(|_| anyhow!("键盘线程启动失败"))?
}
pub fn stop() {
    READY.store(false, Ordering::Release);
    unsafe {
        let _ = PostThreadMessageW(
            THREAD.load(Ordering::Acquire),
            WM_QUIT,
            WPARAM(0),
            LPARAM(0),
        );
    }
}
pub fn initial_selection(len: usize) -> usize {
    usize::from(len > 1)
}
pub fn move_selection(index: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        0
    } else if forward {
        (index + 1) % len
    } else {
        (index + len - 1) % len
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn open(m: &mut Machine) {
        m.step(0xA2, true, false, true);
        m.step(0xA0, true, false, true);
        assert_eq!(
            m.step(TRIGGER_VK, true, false, true).action,
            Some(Action::Open)
        );
    }
    #[test]
    fn cycle_commit_and_paired_release() {
        let mut m = Machine::default();
        open(&mut m);
        assert!(m.step(TRIGGER_VK, false, false, true).swallow);
        assert_eq!(m.step(0x28, true, false, true).action, Some(Action::Next));
        assert_eq!(
            m.step(0xA0, false, false, true).action,
            Some(Action::Commit)
        );
        assert!(m.step(0x28, false, false, true).swallow);
        assert!(!m.step(0x41, true, false, true).swallow);
    }
    #[test]
    fn cancel_does_not_paste() {
        let mut m = Machine::default();
        open(&mut m);
        assert_eq!(m.step(0x1B, true, false, true).action, Some(Action::Cancel));
        assert_eq!(m.step(0xA2, false, false, true).action, None);
    }
    #[test]
    fn injected_ignored_and_not_ready() {
        let mut m = Machine::default();
        m.step(0xA2, true, true, true);
        m.step(0xA0, true, true, true);
        assert_eq!(m.step(TRIGGER_VK, true, false, true).action, None);
        let mut m = Machine::default();
        m.step(0xA2, true, false, false);
        m.step(0xA0, true, false, false);
        assert_eq!(m.step(TRIGGER_VK, true, false, false).action, None);
    }
    #[test]
    fn right_modifiers_and_dual_control() {
        let mut m = Machine::default();
        m.step(0xA3, true, false, true);
        m.step(0xA1, true, false, true);
        assert_eq!(
            m.step(TRIGGER_VK, true, false, true).action,
            Some(Action::Open)
        );
        m.step(0xA2, true, false, true);
        assert_eq!(m.step(0xA3, false, false, true).action, None);
        assert_eq!(
            m.step(0xA2, false, false, true).action,
            Some(Action::Commit)
        );
    }
    #[test]
    fn selection_bounds() {
        assert_eq!(initial_selection(0), 0);
        assert_eq!(initial_selection(1), 0);
        assert_eq!(initial_selection(9), 1);
        assert_eq!(move_selection(0, 9, false), 8);
        assert_eq!(move_selection(8, 9, true), 0);
    }
    #[test]
    fn cancelled_key_repeats_stay_paired() {
        let mut m = Machine::default();
        open(&mut m);
        assert!(m.step(0x1B, true, false, true).swallow);
        assert!(m.step(0x1B, true, false, true).swallow);
        assert!(m.step(TRIGGER_VK, true, false, true).swallow);
        assert!(m.step(TRIGGER_VK, false, false, true).swallow);
        assert!(m.step(0x1B, false, false, true).swallow);
        assert!(!m.step(0x1B, true, false, true).swallow);
    }
    #[test]
    fn previously_held_arrow_release_passes_through() {
        let mut m = Machine::default();
        assert!(!m.step(0x28, true, false, true).swallow);
        open(&mut m);
        assert!(m.step(0x28, true, false, true).swallow);
        assert!(!m.step(0x28, false, false, true).swallow);
    }
}
