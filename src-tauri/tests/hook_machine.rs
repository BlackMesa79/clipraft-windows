//! 键盘状态机（`hook::Machine`）行为测试。
//!
//! 只驱动纯状态机，不安装真实钩子、不调用 Win32，因此可确定性地覆盖各种按键顺序。
//! 对应设计目标：README「已实现范围」第 1、3 条与 docs 第三节「快速粘贴面板交互设计」。

use clipraft_windows::hook::{initial_selection, move_selection, Action, Decision, Machine};

const LCTRL: u32 = 0xA2;
const RCTRL: u32 = 0xA3;
const LSHIFT: u32 = 0xA0;
const RSHIFT: u32 = 0xA1;
const VK_V: u32 = 0x56;
const VK_UP: u32 = 0x26;
const VK_DOWN: u32 = 0x28;
const VK_ESC: u32 = 0x1B;
const VK_A: u32 = 0x41;

fn press(m: &mut Machine, key: u32) -> Decision {
    m.step(key, true, false, true)
}

fn release(m: &mut Machine, key: u32) -> Decision {
    m.step(key, false, false, true)
}

/// Ctrl+Shift+V：打开面板，并把 V 的按下-释放成对吞掉。
fn open_panel(m: &mut Machine) {
    let _ = press(m, LCTRL);
    let _ = press(m, LSHIFT);
    let d = press(m, VK_V);
    assert_eq!(d.action, Some(Action::Open));
    assert!(
        d.swallow,
        "触发键按下必须被吞掉，否则会向目标程序泄漏一个 v"
    );
    assert!(release(m, VK_V).swallow, "触发键释放必须成对吞掉");
}

#[test]
fn left_and_right_modifiers_open_equally() {
    let mut left = Machine::default();
    open_panel(&mut left);
    assert!(left.active);

    let mut right = Machine::default();
    let _ = press(&mut right, RCTRL);
    let _ = press(&mut right, RSHIFT);
    assert_eq!(press(&mut right, VK_V).action, Some(Action::Open));
    assert!(right.active);
}

#[test]
fn dual_control_or_dual_shift_keeps_panel_alive() {
    let mut m = Machine::default();
    let _ = press(&mut m, LCTRL);
    let _ = press(&mut m, RCTRL);
    let _ = press(&mut m, LSHIFT);
    let _ = press(&mut m, RSHIFT);
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Open));

    // 松开一个 Ctrl：另一个 Ctrl 仍按住，面板不应提交。
    let d = release(&mut m, LCTRL);
    assert_eq!(d.action, None);
    assert!(m.active);
    // 松开一个 Shift：另一个 Shift 仍按住，仍然不提交。
    assert_eq!(release(&mut m, RSHIFT).action, None);
    assert!(m.active);
    // 最后一个 Shift 松开，必需组合首次不满足 → 提交一次。
    assert_eq!(release(&mut m, LSHIFT).action, Some(Action::Commit));
    assert!(!m.active);
    // 剩下的 Ctrl 松开不再重复提交。
    assert_eq!(release(&mut m, RCTRL).action, None);
}

#[test]
fn releasing_shift_first_commits_exactly_once() {
    let mut m = Machine::default();
    open_panel(&mut m);
    // README 场景：先松 Shift。
    assert_eq!(release(&mut m, LSHIFT).action, Some(Action::Commit));
    assert!(!m.active);
    assert_eq!(release(&mut m, LCTRL).action, None);
}

#[test]
fn releasing_ctrl_first_commits_exactly_once() {
    let mut m = Machine::default();
    open_panel(&mut m);
    assert_eq!(release(&mut m, LCTRL).action, Some(Action::Commit));
    assert_eq!(release(&mut m, LSHIFT).action, None);
}

#[test]
fn releasing_trigger_key_alone_does_not_commit() {
    let mut m = Machine::default();
    let _ = press(&mut m, LCTRL);
    let _ = press(&mut m, LSHIFT);
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Open));
    // 只松开 V、修饰键仍按住：面板保持打开，不提交。
    let d = release(&mut m, VK_V);
    assert_eq!(d.action, None);
    assert!(m.active);
    assert_eq!(release(&mut m, LSHIFT).action, Some(Action::Commit));
}

#[test]
fn arrow_autorepeat_emits_repeated_moves_and_stays_paired() {
    let mut m = Machine::default();
    open_panel(&mut m);

    let first = press(&mut m, VK_DOWN);
    assert_eq!(first.action, Some(Action::Next));
    assert!(first.swallow);
    // 自动重复（同键连续 keydown）每一次都应继续选择，不重复打开面板。
    for _ in 0..4 {
        let repeat = press(&mut m, VK_DOWN);
        assert_eq!(repeat.action, Some(Action::Next));
        assert!(repeat.swallow, "重复按下也必须吞掉，否则方向键会泄漏到目标");
    }
    assert!(release(&mut m, VK_DOWN).swallow);

    let up = press(&mut m, VK_UP);
    assert_eq!(up.action, Some(Action::Previous));
    assert!(up.swallow);
    assert!(release(&mut m, VK_UP).swallow);
    assert!(m.active);
}

#[test]
fn direction_reversal_and_dedup_of_selection_math() {
    // 方向键来回切换：同一按键的按下被识别为重复后仍按方向处理。
    let mut m = Machine::default();
    open_panel(&mut m);
    assert_eq!(press(&mut m, VK_DOWN).action, Some(Action::Next));
    assert_eq!(press(&mut m, VK_UP).action, Some(Action::Previous));
    assert_eq!(press(&mut m, VK_UP).action, Some(Action::Previous));
    assert_eq!(press(&mut m, VK_DOWN).action, Some(Action::Next));
}

#[test]
fn escape_cancels_and_never_commits() {
    let mut m = Machine::default();
    open_panel(&mut m);
    let esc = press(&mut m, VK_ESC);
    assert_eq!(esc.action, Some(Action::Cancel));
    assert!(
        esc.swallow,
        "Esc 不应传到目标程序（否则会关闭目标的对话框）"
    );
    assert!(!m.active);
    // Esc 释放成对吞掉。
    assert!(release(&mut m, VK_ESC).swallow);
    // 再按 Esc 是普通按键，透传。
    let again = press(&mut m, VK_ESC);
    assert_eq!(again.action, None);
    assert!(!again.swallow);
    // 取消后松修饰键不产生提交。
    assert_eq!(release(&mut m, LSHIFT).action, None);
    assert_eq!(release(&mut m, LCTRL).action, None);
}

#[test]
fn escape_autorepeat_stays_paired_until_release() {
    // 触发键始终按住（自动重复）时按 Esc，随后该键的重复与释放都必须成对吞掉。
    let mut m = Machine::default();
    let _ = press(&mut m, LCTRL);
    let _ = press(&mut m, LSHIFT);
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Open));
    assert!(press(&mut m, VK_ESC).swallow);
    assert!(press(&mut m, VK_ESC).swallow);
    let v_repeat = press(&mut m, VK_V);
    assert_eq!(v_repeat.action, None, "取消后按键重复不应重新打开面板");
    assert!(v_repeat.swallow);
    assert!(release(&mut m, VK_V).swallow);
    assert!(release(&mut m, VK_ESC).swallow);
    assert!(!press(&mut m, VK_ESC).swallow);
    assert!(!m.active);
}

#[test]
fn selection_helpers_handle_empty_and_single_item() {
    // 空列表：默认选择 0，移动始终为 0，不 panic、不取模除零。
    assert_eq!(initial_selection(0), 0);
    assert_eq!(move_selection(0, 0, true), 0);
    assert_eq!(move_selection(0, 0, false), 0);
    assert_eq!(move_selection(7, 0, true), 0);

    // 只有一条记录：默认选第一条（不是设计里的「第二条」）。
    assert_eq!(initial_selection(1), 0);
    assert_eq!(move_selection(0, 1, true), 0);
    assert_eq!(move_selection(0, 1, false), 0);

    // 多条：默认第二条，向前/向后循环。
    assert_eq!(initial_selection(2), 1);
    assert_eq!(initial_selection(9), 1);
    assert_eq!(move_selection(0, 3, false), 2);
    assert_eq!(move_selection(2, 3, true), 0);
    assert_eq!(move_selection(1, 3, true), 2);
}

#[test]
fn trigger_requires_both_modifiers() {
    let mut only_ctrl = Machine::default();
    let _ = press(&mut only_ctrl, LCTRL);
    let d = press(&mut only_ctrl, VK_V);
    assert_eq!(d.action, None);
    assert!(!d.swallow, "组合不满足时 V 必须透传，不能吃掉用户输入");

    let mut only_shift = Machine::default();
    let _ = press(&mut only_shift, LSHIFT);
    assert_eq!(press(&mut only_shift, VK_V).action, None);

    let mut none = Machine::default();
    assert_eq!(press(&mut none, VK_V).action, None);
}

#[test]
fn presses_before_the_modifier_do_not_open() {
    let mut m = Machine::default();
    assert_eq!(press(&mut m, VK_V).action, None);
    let _ = press(&mut m, LCTRL);
    let _ = press(&mut m, LSHIFT);
    // V 仍处于按下状态，此时再收到 keydown 属于重复，不应打开面板。
    assert_eq!(press(&mut m, VK_V).action, None);
    let _ = release(&mut m, VK_V);
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Open));
}

#[test]
fn injected_events_never_open_and_do_not_change_state() {
    let mut m = Machine::default();
    // 模拟 SendInput 注入的修饰键：钩子已在上游过滤，状态机也不应记账。
    assert!(!m.step(LCTRL, true, true, true).swallow);
    assert!(!m.step(LSHIFT, true, true, true).swallow);
    assert_eq!(m.step(VK_V, true, false, true).action, None);
    assert!(!m.active);
    // 注入的 V 同样被忽略。
    let _ = press(&mut m, LCTRL);
    let _ = press(&mut m, LSHIFT);
    assert_eq!(m.step(VK_V, true, true, true).action, None);
    assert!(!m.active);
}

#[test]
fn disabled_machine_passes_everything_through() {
    let mut m = Machine::default();
    let _ = m.step(LCTRL, true, false, false);
    let _ = m.step(LSHIFT, true, false, false);
    let d = m.step(VK_V, true, false, false);
    assert_eq!(d.action, None);
    assert!(!d.swallow);
    // 重新启用后需要等 V 抬起再按下才会触发（避免刚打开就误开面板）。
    assert_eq!(m.step(VK_V, true, false, true).action, None);
    let _ = m.step(VK_V, false, false, true);
    assert_eq!(m.step(VK_V, true, false, true).action, Some(Action::Open));
}

#[test]
fn seed_marks_already_held_modifiers() {
    // seed 用于钩子安装时把已按住的修饰键记为按下，避免要求重新按一次。
    let mut m = Machine::default();
    m.seed(0b0000_0101); // LCTRL + LSHIFT
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Open));

    let mut only_shift_seeded = Machine::default();
    only_shift_seeded.seed(0b0000_0100);
    assert_eq!(press(&mut only_shift_seeded, VK_V).action, None);
}

#[test]
fn unrelated_keys_pass_through_while_panel_open() {
    // 设计取舍：面板期间只拦截 V/方向键/Esc，其余按键仍交给目标程序，输入法不受影响。
    let mut m = Machine::default();
    open_panel(&mut m);
    let d = press(&mut m, VK_A);
    assert_eq!(d.action, None);
    assert!(!d.swallow);
    assert!(!release(&mut m, VK_A).swallow);
    assert!(m.active);
    assert_eq!(release(&mut m, LSHIFT).action, Some(Action::Commit));
}

#[test]
fn trigger_repeat_does_not_advance_selection() {
    let mut m = Machine::default();
    let _ = press(&mut m, LCTRL);
    let _ = press(&mut m, LSHIFT);
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Open));
    // 首次呼出后保持按住，以及重新按下后的重复，都不能前进。
    for _ in 0..4 {
        let repeat = press(&mut m, VK_V);
        assert_eq!(repeat.action, None);
        assert!(repeat.swallow);
    }
    assert!(release(&mut m, VK_V).swallow);
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Next));
    assert_eq!(press(&mut m, VK_V).action, None);
    assert!(release(&mut m, VK_V).swallow);
    assert!(m.active);
}

#[test]
fn trigger_repeat_never_advances_across_multiple_presses() {
    // D-3 回归：只有离散按下前进一格；连续多轮「按下 + 自动重复」都只前进一次。
    let mut m = Machine::default();
    let _ = press(&mut m, LCTRL);
    let _ = press(&mut m, LSHIFT);
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Open));

    for round in 0..3 {
        assert!(
            release(&mut m, VK_V).swallow,
            "第 {round} 轮 V 释放必须与按下成对吞掉"
        );
        let advance = press(&mut m, VK_V);
        assert_eq!(
            advance.action,
            Some(Action::Next),
            "第 {round} 轮离散按下应恰好前进一格"
        );
        assert!(advance.swallow);
        for _ in 0..5 {
            let repeat = press(&mut m, VK_V);
            assert_eq!(repeat.action, None, "第 {round} 轮自动重复不得前进");
            assert!(
                repeat.swallow,
                "第 {round} 轮自动重复仍要吞掉，否则 v 会泄漏到目标程序"
            );
        }
    }
    assert!(release(&mut m, VK_V).swallow);
    // 全程只有 3 次前进，松开修饰键提交一次。
    assert_eq!(release(&mut m, LSHIFT).action, Some(Action::Commit));
    assert!(!m.active);
}

#[test]
fn keys_already_delivered_keep_their_release() {
    // 面板打开前已透传给目标的方向键，其释放必须继续透传，否则目标程序漏收 keyup。
    let mut m = Machine::default();
    assert!(!press(&mut m, VK_DOWN).swallow);
    open_panel(&mut m);
    assert!(press(&mut m, VK_DOWN).swallow);
    assert!(!release(&mut m, VK_DOWN).swallow);
}

#[test]
fn commit_then_swallowed_key_release_stays_paired() {
    let mut m = Machine::default();
    open_panel(&mut m);
    assert!(press(&mut m, VK_DOWN).swallow);
    // 按住方向键的同时松开 Shift → 提交，随后方向键的释放仍要被吞掉（成对）。
    assert_eq!(release(&mut m, LSHIFT).action, Some(Action::Commit));
    assert!(release(&mut m, VK_DOWN).swallow);
    // 新的一次按下则透传。
    assert!(!press(&mut m, VK_DOWN).swallow);
    assert!(!release(&mut m, VK_DOWN).swallow);
}

#[test]
fn out_of_range_keycodes_are_ignored() {
    let mut m = Machine::default();
    for key in [256u32, 257, 0x1_0000, u32::MAX] {
        let d = m.step(key, true, false, true);
        assert_eq!(d.action, None);
        assert!(!d.swallow);
    }
}

#[test]
fn reopen_after_cancel_and_after_commit() {
    let mut m = Machine::default();
    open_panel(&mut m);
    let _ = press(&mut m, VK_ESC);
    let _ = release(&mut m, VK_ESC);
    // 修饰键仍按住时再次按 V 应能重新打开。
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Open));
    let _ = release(&mut m, VK_V);
    assert_eq!(release(&mut m, LSHIFT).action, Some(Action::Commit));
    // 只按住 Ctrl、没有 Shift 时按 V 不应打开。
    assert_eq!(press(&mut m, VK_V).action, None);
    let _ = release(&mut m, VK_V);
    let _ = press(&mut m, LSHIFT);
    assert_eq!(press(&mut m, VK_V).action, Some(Action::Open));
}

#[test]
fn missed_main_hotkey_release_is_reconciled_before_first_quick_press() {
    let mut baseline = Machine::default();
    baseline.step(0xA4, true, false, true);
    baseline.step(0x56, true, false, true);
    baseline.step(0xA4, false, false, true);
    baseline.step(0xA2, true, false, true);
    baseline.step(0xA0, true, false, true);
    assert_eq!(
        baseline.step(0x56, true, false, true).action,
        None,
        "未校正时重现日志中的首次无反应"
    );
    let mut machine = Machine::default();
    machine.step(0xA4, true, false, true);
    assert_eq!(machine.step(0x56, true, false, true).action, None);
    machine.step(0xA4, false, false, true);
    // 模拟 Alt+V 的真实 V-up 未进入状态机；新的 Ctrl 事件确认系统V已松开。
    assert!(machine.reconcile_trigger_release(0x56, false));
    machine.step(0xA2, true, false, true);
    machine.step(0xA0, true, false, true);
    assert_eq!(
        machine.step(0x56, true, false, true).action,
        Some(Action::Open)
    );
}

#[test]
fn held_or_swallowed_trigger_is_never_reconciled_as_released() {
    let mut machine = Machine::default();
    machine.step(0x56, true, false, true);
    assert!(!machine.reconcile_trigger_release(0x56, true));
    assert!(!machine.reconcile_trigger_release(0x41, false));
    machine.step(0x56, false, false, true);
    machine.step(0xA2, true, false, true);
    machine.step(0xA0, true, false, true);
    assert_eq!(
        machine.step(0x56, true, false, true).action,
        Some(Action::Open)
    );
    assert!(!machine.reconcile_trigger_release(0x56, false));
    machine.step(0x1B, true, false, true);
    assert!(!machine.reconcile_trigger_release(0x56, false));
    assert_eq!(machine.step(0x56, true, false, true).action, None);
    assert!(machine.step(0x56, false, false, true).swallow);
}
