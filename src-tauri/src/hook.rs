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
static CANCEL_PANEL: AtomicBool = AtomicBool::new(false);

// 控制线程的隐藏请求在下一次键盘事件前生效，不注入按键或改动修饰键状态。
pub fn cancel_panel() {
    CANCEL_PANEL.store(true, Ordering::Release);
}
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
    fn cancel(&mut self) {
        // 与 Esc 一致：保留已吞按键的配对释放和重复按下记录。
        self.active = false;
    }
    fn modifier_bits(&self) -> u16 {
        MODIFIER_KEYS.iter().enumerate().fold(0, |bits, (i, key)| {
            bits | (u16::from(self.down[*key as usize]) << i)
        })
    }
    pub fn step_with_logical(
        &mut self,
        key: u32,
        down: bool,
        injected: bool,
        enabled: bool,
        logical: u16,
    ) -> Decision {
        self.seed(corrected_modifiers(
            self.modifier_bits(),
            logical,
            key,
            down,
            injected,
        ));
        self.step(key, down, injected, enabled)
    }
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
        let ctrl = self.down[0xA2] || self.down[0xA3];
        let shift = self.down[0xA0] || self.down[0xA1];
        let modifiers = (!REQUIRE_CTRL || ctrl) && (!REQUIRE_SHIFT || shift);
        if self.active {
            if !modifiers {
                self.active = false;
                out.action = Some(Action::Commit);
            } else if down {
                out.action = match key {
                    0x1B => {
                        self.cancel();
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
// 只清除失效的按下记录，不把系统中的注入按键补记为真实按键。
// 异步状态早于当前事件，因此当前真实事件覆盖采样；注入事件不改变对应键。
pub fn corrected_modifiers(
    recorded: u16,
    logical: u16,
    key: u32,
    down: bool,
    injected: bool,
) -> u16 {
    let mut corrected = recorded & logical & 0xff;
    if let Some(i) = MODIFIER_KEYS.iter().position(|k| u32::from(*k) == key) {
        let bit = 1 << i;
        let held = if injected { recorded & bit != 0 } else { down };
        corrected = (corrected & !bit) | if held { bit } else { 0 };
    }
    corrected
}
thread_local! {static MACHINE:Cell<Machine>=Cell::new(Machine::default());}
unsafe extern "system" fn callback(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, w, l);
    }
    let data = &*(l.0 as *const KBDLLHOOKSTRUCT);
    let down = w.0 == WM_KEYDOWN as usize || w.0 == WM_SYSKEYDOWN as usize;
    let injected = data.flags.contains(LLKHF_INJECTED);
    let logical = crate::paster::logical();
    PHYSICAL.store(
        corrected_modifiers(
            PHYSICAL.load(Ordering::SeqCst),
            logical,
            data.vkCode,
            down,
            injected,
        ),
        Ordering::SeqCst,
    );
    // 线程局部固定大小状态，无分配、锁、IO、UI 或同步等待。
    let swallow = MACHINE.with(|slot| {
        let mut state = slot.get();
        if CANCEL_PANEL.swap(false, Ordering::AcqRel) {
            state.cancel();
        }
        let decision = state.step_with_logical(
            data.vkCode,
            down,
            injected,
            READY.load(Ordering::Relaxed),
            logical,
        );
        let mut swallow = decision.swallow;
        crate::diag::panel(true, state.active);
        if let Some(action) = decision.action {
            // 在钩子观察到新 Open 时立即使旧粘贴过期，不等待 broker 或 UI 调度。
            let generation = if action == Action::Open {
                crate::runtime::new_paste_intent()
            } else {
                crate::runtime::paste_generation()
            };
            if PostThreadMessageW(
                RECEIVER.load(Ordering::Relaxed),
                ACTION_MESSAGE,
                WPARAM(action as usize),
                LPARAM(generation as isize),
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
        crate::diag::keyboard(data.vkCode, down, injected, swallow, decision.action);
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
    fn external_cancel_passes_navigation_while_modifiers_are_held() {
        let mut m = Machine::default();
        open(&mut m);
        m.cancel();
        assert!(!m.active);
        for key in [0x28, 0x26] {
            for down in [true, false] {
                assert_eq!(
                    m.step(key, down, false, true),
                    Decision {
                        swallow: false,
                        action: None
                    }
                );
            }
        }
        assert_eq!(m.step(0xA2, false, false, true).action, None);
        assert_eq!(m.step(0xA0, false, false, true).action, None);
    }
    #[test]
    fn external_cancel_keeps_swallowed_pairs_and_allows_reopening() {
        let mut m = Machine::default();
        open(&mut m);
        assert!(m.step(0x28, true, false, true).swallow);
        m.cancel();
        for key in [TRIGGER_VK, 0x28] {
            for down in [true, false] {
                assert_eq!(
                    m.step(key, down, false, true),
                    Decision {
                        swallow: true,
                        action: None
                    }
                );
            }
        }
        assert!(!m.step(0x28, true, false, true).swallow);
        assert!(!m.step(0x28, false, false, true).swallow);
        assert_eq!(
            m.step(TRIGGER_VK, true, false, true).action,
            Some(Action::Open)
        );
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
    #[test]
    fn lost_releases_do_not_open_on_v_alone() {
        for held in [0b0101, 0b1010, 0b1111] {
            let mut m = Machine::default();
            m.seed(held);
            let d = m.step_with_logical(TRIGGER_VK, true, false, true, 0);
            assert_eq!(d.action, None);
            assert!(!d.swallow);
            assert_eq!(m.modifier_bits(), 0);
        }
    }
    #[test]
    fn lost_releases_commit_on_next_real_event_once() {
        let mut m = Machine::default();
        open(&mut m);
        // Ctrl/Shift 抬起都丢失，下一次 V 抬起仍应提交并配对吞键。
        let d = m.step_with_logical(TRIGGER_VK, false, false, true, 0);
        assert_eq!(d.action, Some(Action::Commit));
        assert!(d.swallow);
        assert!(!m.active);
        assert_eq!(m.step_with_logical(0x41, true, false, true, 0).action, None);
    }
    #[test]
    fn current_modifier_event_overrides_pre_event_sample() {
        for (i, key) in MODIFIER_KEYS.iter().enumerate() {
            assert_eq!(
                corrected_modifiers(0, 0, u32::from(*key), true, false),
                1 << i
            );
            assert_eq!(
                corrected_modifiers(1 << i, 1 << i, u32::from(*key), false, false),
                0
            );
        }
        let mut m = Machine::default();
        m.step_with_logical(0xA2, true, false, true, 0);
        m.step_with_logical(0xA0, true, false, true, 1);
        assert_eq!(
            m.step_with_logical(TRIGGER_VK, true, false, true, 5).action,
            Some(Action::Open)
        );
        assert_eq!(
            m.step_with_logical(0xA0, false, false, true, 5).action,
            Some(Action::Commit)
        );
    }
    #[test]
    fn correction_clears_all_stale_slots_without_promoting_logical_keys() {
        assert_eq!(corrected_modifiers(0xff, 0, TRIGGER_VK, true, false), 0);
        assert_eq!(corrected_modifiers(0, 0xff, TRIGGER_VK, true, false), 0);
        assert_eq!(
            corrected_modifiers(0xff, 0x0a, TRIGGER_VK, true, false),
            0x0a
        );
    }
    #[test]
    fn injected_alt_neither_opens_nor_commits() {
        let mut m = Machine::default();
        open(&mut m);
        for (down, logical) in [(true, 5), (false, 0x15)] {
            let d = m.step_with_logical(0xA4, down, true, true, logical);
            assert_eq!(d.action, None);
            assert!(!d.swallow);
            assert_eq!(m.modifier_bits(), 5);
            assert!(m.active);
        }
        // 注入 Alt 的逻辑按下不能阻止真实 Shift 抬起提交。
        assert_eq!(
            m.step_with_logical(0xA0, false, false, true, 0x15).action,
            Some(Action::Commit)
        );
        let mut idle = Machine::default();
        idle.step_with_logical(0xA4, true, true, true, 0);
        assert_eq!(
            idle.step_with_logical(TRIGGER_VK, true, false, true, 0x10)
                .action,
            None
        );
        assert_eq!(idle.modifier_bits(), 0);
    }
    #[test]
    fn injected_modifier_release_preserves_real_current_key() {
        assert_eq!(corrected_modifiers(0x10, 0, 0xA4, false, true), 0x10);
        assert_eq!(corrected_modifiers(0x10, 0, TRIGGER_VK, true, false), 0);
    }
    #[test]
    fn corrected_dual_modifiers_keep_remaining_side_and_swallow_pairs() {
        let mut m = Machine::default();
        m.seed(0x0f);
        assert_eq!(
            m.step_with_logical(TRIGGER_VK, true, false, true, 0x0f)
                .action,
            Some(Action::Open)
        );
        let d = m.step_with_logical(TRIGGER_VK, false, false, true, 0x0a);
        assert_eq!(d.action, None);
        assert!(d.swallow);
        assert_eq!(m.modifier_bits(), 0x0a);
        assert_eq!(
            m.step_with_logical(0xA3, false, false, true, 0x0a).action,
            Some(Action::Commit)
        );
    }
}
