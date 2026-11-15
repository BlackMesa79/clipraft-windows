//! 钩子单事件编排；只使用固定大小状态、原子操作和原有非阻塞消息投递。
use super::*;

struct HookEvent {
    key: u32,
    scan: u32,
    down: bool,
    injected: bool,
    logical: u16,
    trigger_sample: Option<(u32, bool)>,
}

pub(super) unsafe extern "system" fn callback(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, w, l);
    }
    let keyboard = &*(l.0 as *const KBDLLHOOKSTRUCT);
    let event = HookEvent {
        key: keyboard.vkCode,
        scan: keyboard.scanCode,
        down: w.0 == WM_KEYDOWN as usize || w.0 == WM_SYSKEYDOWN as usize,
        injected: keyboard.flags.contains(LLKHF_INJECTED),
        logical: crate::paster::logical(),
        trigger_sample: trigger_sample(keyboard.vkCode, keyboard.flags.contains(LLKHF_INJECTED)),
    };
    PHYSICAL.store(
        corrected_modifiers(
            PHYSICAL.load(Ordering::SeqCst),
            event.logical,
            event.key,
            event.down,
            event.injected,
        ),
        Ordering::SeqCst,
    );
    let swallow = MACHINE.with(|slot| {
        let mut state = slot.get();
        let swallow = process_event(&mut state, &event);
        slot.set(state);
        swallow
    });
    if swallow {
        return LRESULT(1);
    }
    CallNextHookEx(None, code, w, l)
}

fn trigger_sample(key: u32, injected: bool) -> Option<(u32, bool)> {
    if injected
        || !MODIFIER_KEYS
            .iter()
            .any(|modifier| u32::from(*modifier) == key)
    {
        return None;
    }
    let trigger = read_shortcut(&SHORTCUT).key;
    Some((trigger, unsafe { GetAsyncKeyState(trigger as i32) } < 0))
}

fn process_event(state: &mut Machine, event: &HookEvent) -> bool {
    if CANCEL_PANEL.swap(false, Ordering::AcqRel) {
        state.cancel();
    }
    let reserved = claim_event(&CONFIG_GATE);
    if reserved && !state.active {
        state.shortcut = read_shortcut(&SHORTCUT);
    }
    let session = RECORD_SESSION.load(Ordering::Acquire);
    let recording = recording() && session == RECORD_SESSION.load(Ordering::Acquire);
    if !recording {
        reconcile_trigger(state, event);
    }
    let was_down = event.key < 256 && state.down[event.key as usize];
    let active_before = state.active;
    let ready = READY.load(Ordering::Relaxed);
    let trace_trigger = diagnostic_candidate(state, event);
    let mut decision = state.step_with_logical(
        event.key,
        event.down,
        event.injected,
        reserved && !recording && ready,
        event.logical,
    );
    if recording {
        if let Some(recorded) = record_event(state, event, was_down, session) {
            decision = recorded;
        }
    }
    if trace_trigger && crate::diag::enabled() {
        crate::diag::trigger::record(crate::diag::trigger::Trigger {
            down: event.down,
            repeated: was_down,
            active: active_before,
            ready,
            reserved,
            recording,
            held: held_modifiers(state.modifier_bits()),
            expected: state.shortcut.modifiers,
            action: decision.action,
        });
    }
    finish_event(state, event, &decision, reserved)
}

fn reconcile_trigger(state: &mut Machine, event: &HookEvent) {
    if let Some((key, down)) = event.trigger_sample {
        if state.reconcile_trigger_release(key, down) {
            crate::diag::trigger::reconciled();
        }
    }
}

fn diagnostic_candidate(state: &Machine, event: &HookEvent) -> bool {
    !event.injected
        && event.key == state.shortcut.key
        && (event.logical != 0 || state.modifier_bits() != 0 || state.active)
}

fn finish_event(
    state: &mut Machine,
    event: &HookEvent,
    decision: &Decision,
    reserved: bool,
) -> bool {
    crate::diag::panel(true, state.active);
    let swallow = dispatch_action(state, event.key, decision, post_action);
    crate::diag::panel(true, state.active);
    crate::diag::keyboard(
        event.key,
        event.scan,
        event.down,
        event.injected,
        swallow,
        decision.action,
    );
    if reserved && !state.active {
        CONFIG_GATE.store(0, Ordering::Release);
    }
    swallow
}

fn record_event(
    state: &mut Machine,
    event: &HookEvent,
    repeated: bool,
    session: u64,
) -> Option<Decision> {
    let packed = state.record_event(event.key, event.down, event.injected, repeated)?;
    let decision = Decision {
        swallow: true,
        action: None,
    };
    publish_recording(session, packed);
    Some(decision)
}

fn dispatch_action(
    state: &mut Machine,
    key: u32,
    decision: &Decision,
    mut post: impl FnMut(Action) -> bool,
) -> bool {
    let Some(action) = decision.action else {
        return decision.swallow;
    };
    if post(action) {
        return decision.swallow;
    }
    state.reject_action(key);
    false
}

fn post_action(action: Action) -> bool {
    // 新 Open 在钩子观察时立即使旧粘贴过期，不等待 broker 或 UI。
    let generation = if action == Action::Open {
        crate::runtime::new_paste_intent()
    } else {
        crate::runtime::paste_generation()
    };
    let accepted = unsafe {
        PostThreadMessageW(
            RECEIVER.load(Ordering::Relaxed),
            ACTION_MESSAGE,
            WPARAM(action as usize),
            LPARAM(generation as isize),
        )
        .is_ok()
    };
    crate::diag::trigger::dispatch(action, generation, accepted);
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_candidates_exclude_plain_typing_and_injected_events() {
        let state = Machine::default();
        let mut event = HookEvent {
            key: 0x56,
            scan: 0,
            down: true,
            injected: false,
            logical: 0,
            trigger_sample: None,
        };
        assert!(!diagnostic_candidate(&state, &event));
        event.logical = 1;
        assert!(diagnostic_candidate(&state, &event));
        event.key = 0x41;
        assert!(!diagnostic_candidate(&state, &event));
        event.key = 0x56;
        event.injected = true;
        assert!(!diagnostic_candidate(&state, &event));
    }

    #[test]
    fn modifier_probe_repairs_only_matching_released_trigger() {
        let mut state = Machine::default();
        state.step(0xA4, true, false, true);
        state.step(0x56, true, false, true);
        let mut event = HookEvent {
            key: 0xA2,
            scan: 0,
            down: true,
            injected: false,
            logical: 1,
            trigger_sample: Some((0x41, false)),
        };
        reconcile_trigger(&mut state, &event);
        assert!(state.down[0x56]);
        event.trigger_sample = Some((0x56, false));
        reconcile_trigger(&mut state, &event);
        assert!(!state.down[0x56]);
    }

    fn opened() -> Machine {
        let mut state = Machine::default();
        state.step(0xA2, true, false, true);
        state.step(0xA0, true, false, true);
        assert_eq!(
            state.step(0x56, true, false, true).action,
            Some(Action::Open)
        );
        state
    }

    #[test]
    fn stale_recording_result_still_swallows_and_overrides_panel_action() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let previous = RECORD_ARMED.swap(123, Ordering::AcqRel);
        let mut state = opened();
        let event = HookEvent {
            key: 0x1B,
            scan: 0,
            down: true,
            injected: false,
            logical: 5,
            trigger_sample: None,
        };
        let panel =
            state.step_with_logical(event.key, event.down, event.injected, false, event.logical);
        let recorded = record_event(&mut state, &event, false, 124);
        let armed = RECORD_ARMED.swap(previous, Ordering::AcqRel);
        assert_eq!(panel.action, Some(Action::Cancel));
        assert_eq!(
            recorded,
            Some(Decision {
                swallow: true,
                action: None
            })
        );
        assert_eq!(armed, 123);
        assert!(state.step(0x1B, false, false, false).swallow);
    }

    #[test]
    fn failed_open_passes_current_repeat_and_paired_release() {
        let mut state = Machine::default();
        state.seed(1 | 4);
        let decision = state.step(0x56, true, false, true);
        let mut posted = Vec::new();
        assert!(!dispatch_action(&mut state, 0x56, &decision, |action| {
            posted.push(action);
            false
        }));
        assert_eq!(posted, [Action::Open]);
        assert!(!state.active);
        assert!(!state.step(0x56, true, false, true).swallow);
        assert!(!state.step(0x56, false, false, true).swallow);
    }

    #[test]
    fn failed_navigation_only_clears_its_own_swallowed_pair() {
        let mut state = opened();
        let decision = state.step(0x28, true, false, true);
        assert_eq!(decision.action, Some(Action::Next));
        assert!(!dispatch_action(&mut state, 0x28, &decision, |_| false));
        assert!(!state.active);
        assert!(!state.step(0x28, false, false, true).swallow);
        assert!(state.step(0x56, false, false, true).swallow);
    }

    #[test]
    fn successful_and_absent_actions_keep_original_decision() {
        let mut state = opened();
        let decision = state.step(0x28, true, false, true);
        assert!(dispatch_action(&mut state, 0x28, &decision, |action| {
            assert_eq!(action, Action::Next);
            true
        }));
        assert!(state.active);
        let released = state.step(0x28, false, false, true);
        assert_eq!(released.action, None);
        assert!(dispatch_action(&mut state, 0x28, &released, |_| panic!(
            "no action must not post"
        )));
        let ignored = state.step(256, true, false, true);
        assert!(!dispatch_action(&mut state, 256, &ignored, |_| panic!(
            "invalid key must not post"
        )));
    }

    #[test]
    fn failed_commit_does_not_drop_the_other_key_release() {
        let mut state = opened();
        let decision = state.step(0xA0, false, false, true);
        assert_eq!(decision.action, Some(Action::Commit));
        assert!(!dispatch_action(&mut state, 0xA0, &decision, |_| false));
        assert!(state.step(0x56, false, false, true).swallow);
        assert_eq!(state.step(0xA2, false, false, true).action, None);
    }
}
