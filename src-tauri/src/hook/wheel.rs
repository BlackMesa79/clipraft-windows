//! 与键盘共用线程和选择消息；活动会话接管真实垂直滚轮和右键取消。
use super::*;

const WHEEL_NOTCH: i32 = 120;
thread_local! { static REMAINDER: Cell<i32> = const { Cell::new(0) }; }
thread_local! { static RIGHT_BUTTON_SWALLOWED: Cell<bool> = const { Cell::new(false) }; }

pub(super) fn reset() {
    REMAINDER.with(|remainder| remainder.set(0));
}

pub(super) unsafe extern "system" fn callback(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 || !matches!(w.0 as u32, WM_MOUSEWHEEL | WM_RBUTTONDOWN | WM_RBUTTONUP) {
        return CallNextHookEx(None, code, w, l);
    }
    let mouse = &*(l.0 as *const MSLLHOOKSTRUCT);
    if mouse.flags & LLMHF_INJECTED != 0 {
        return CallNextHookEx(None, code, w, l);
    }
    let swallow = match w.0 as u32 {
        WM_MOUSEWHEEL => scroll((mouse.mouseData >> 16) as i16),
        WM_RBUTTONDOWN => RIGHT_BUTTON_SWALLOWED.with(|paired| press_right(paired, cancel_active)),
        WM_RBUTTONUP => RIGHT_BUTTON_SWALLOWED.with(|paired| paired.replace(false)),
        _ => false,
    };
    if swallow {
        return LRESULT(1);
    }
    CallNextHookEx(None, code, w, l)
}

fn scroll(delta: i16) -> bool {
    if !eligible() {
        reset();
        return false;
    }
    REMAINDER.with(|remainder| {
        let mut pending = remainder.get();
        let accepted = navigate(&mut pending, delta, event::post_action);
        remainder.set(pending);
        accepted
    })
}

fn press_right(paired: &Cell<bool>, cancel: impl FnOnce() -> bool) -> bool {
    if !paired.get() {
        paired.set(cancel());
    }
    paired.get()
}

fn cancel_active() -> bool {
    if !eligible() {
        return false;
    }
    let accepted = MACHINE.with(|slot| {
        let mut state = slot.get();
        let accepted = cancel_selection(&mut state, event::post_action);
        slot.set(state);
        accepted
    });
    reset();
    let _ = CONFIG_GATE.compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire);
    crate::diag::panel(true, false);
    accepted
}

fn cancel_selection(state: &mut Machine, post: impl FnOnce(Action) -> bool) -> bool {
    state.cancel();
    post(Action::Cancel)
}

fn eligible() -> bool {
    READY.load(Ordering::Acquire)
        && !CANCEL_PANEL.load(Ordering::Acquire)
        && !recording()
        && MACHINE.with(|slot| session_held(&slot.get(), crate::paster::logical()))
}

fn session_held(state: &Machine, logical: u16) -> bool {
    state.active && held_modifiers(logical) & state.shortcut.modifiers == state.shortcut.modifiers
}

fn navigate(remainder: &mut i32, delta: i16, mut post: impl FnMut(Action) -> bool) -> bool {
    *remainder += i32::from(delta);
    let mut dispatched = false;
    while remainder.abs() >= WHEEL_NOTCH {
        let action = if *remainder > 0 {
            Action::Previous
        } else {
            Action::Next
        };
        if !post(action) {
            *remainder = 0;
            return dispatched;
        }
        dispatched = true;
        *remainder -= remainder.signum() * WHEEL_NOTCH;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_accumulates_high_resolution_deltas_and_preserves_direction() {
        let mut remainder = 0;
        let mut actions = Vec::new();
        for delta in [40, 40, 40, -240, 120] {
            assert!(navigate(&mut remainder, delta, |action| {
                actions.push(action);
                true
            }));
        }
        assert_eq!(
            actions,
            [
                Action::Previous,
                Action::Next,
                Action::Next,
                Action::Previous
            ]
        );
        assert_eq!(remainder, 0);
    }

    #[test]
    fn keyboard_and_wheel_share_selection_until_release_or_cancel() {
        let mut machine = Machine::default();
        machine.seed(1 | 4);
        assert_eq!(
            machine.step(0x56, true, false, true).action,
            Some(Action::Open)
        );
        let mut position = initial_selection(4);
        let mut remainder = 0;
        for delta in [-120, 120] {
            assert!(session_held(&machine, 1 | 4));
            navigate(&mut remainder, delta, |action| {
                position = move_selection(position, 4, action == Action::Next);
                true
            });
            let action = machine.step(0x28, true, false, true).action.unwrap();
            position = move_selection(position, 4, action == Action::Next);
        }
        assert_eq!(position, 3);
        assert!(!session_held(&machine, 1));
        assert_eq!(
            machine.step(0x1b, true, false, true).action,
            Some(Action::Cancel)
        );
        assert!(!session_held(&machine, 1 | 4));
    }

    #[test]
    fn right_click_cancels_once_and_pairs_release_after_close() {
        let mut machine = Machine::default();
        let paired = Cell::new(false);
        machine.seed(1 | 4);
        assert_eq!(
            machine.step(0x56, true, false, true).action,
            Some(Action::Open)
        );
        assert!(press_right(&paired, || {
            cancel_selection(&mut machine, |action| {
                assert_eq!(action, Action::Cancel);
                true
            })
        }));
        assert!(!machine.active);
        assert_eq!(machine.step(0xA0, false, false, true).action, None);
        assert!(press_right(&paired, || panic!("右键释放前不重复取消")));
        assert!(paired.replace(false));
        assert!(!press_right(&paired, || false));
        assert!(!paired.replace(false));
    }

    #[test]
    fn failed_right_click_dispatch_never_commits_on_modifier_release() {
        let mut machine = Machine::default();
        let paired = Cell::new(false);
        machine.seed(1 | 4);
        assert_eq!(
            machine.step(0x56, true, false, true).action,
            Some(Action::Open)
        );
        let accepted = press_right(&paired, || cancel_selection(&mut machine, |_| false));
        assert!(!accepted);
        assert!(!paired.replace(false));
        assert_eq!(machine.step(0xA2, false, false, true).action, None);
    }

    #[test]
    fn failed_dispatch_passes_through_and_clears_partial_delta() {
        let mut remainder = -80;
        assert!(!navigate(&mut remainder, -120, |_| false));
        assert_eq!(remainder, 0);
    }

    #[test]
    fn partial_dispatch_does_not_also_scroll_the_target_application() {
        let mut remainder = 0;
        let mut attempts = 0;
        assert!(navigate(&mut remainder, 240, |_| {
            attempts += 1;
            attempts == 1
        }));
        assert_eq!(attempts, 2);
        assert_eq!(remainder, 0);
    }
}
