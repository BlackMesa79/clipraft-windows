//! 等待修饰键释放的纯逻辑测试，不向系统发送按键。
use clipboard_m0::paster::{release_decision, ReleaseDecision::*};
use std::time::Duration;
#[test]
fn only_logical_modifiers_block_paste() {
    assert_eq!(release_decision(0, Duration::ZERO), Ready);
    for bit in 0..8 {
        assert_eq!(release_decision(1 << bit, Duration::ZERO), Wait);
        assert_eq!(release_decision(1 << bit, Duration::from_secs(1)), Timeout);
    }
}
#[test]
fn logical_lag_does_not_allow_early_paste() {
    for logical in [5, 1] {
        assert_eq!(release_decision(logical, Duration::from_millis(500)), Wait);
    }
    assert_eq!(release_decision(0, Duration::from_millis(505)), Ready);
}
#[test]
fn one_second_is_the_timeout_boundary() {
    assert_eq!(release_decision(4, Duration::from_millis(999)), Wait);
    assert_eq!(release_decision(4, Duration::from_millis(1000)), Timeout);
    assert_eq!(release_decision(1, Duration::from_secs(2)), Timeout);
    assert_eq!(release_decision(0, Duration::from_secs(1)), Ready);
}
#[test]
fn high_bits_are_not_modifier_slots() {
    assert_eq!(release_decision(0xff00, Duration::ZERO), Ready);
}
