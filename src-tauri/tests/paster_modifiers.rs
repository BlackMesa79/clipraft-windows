//! 等待修饰键释放的纯逻辑测试，不向系统发送按键。
use clipboard_m0::paster::{release_decision, ReleaseDecision::*};
use std::time::Duration;
#[test]
fn both_sources_must_be_released() {
    assert_eq!(release_decision(0, 0, Duration::ZERO), Ready);
    for bit in 0..8 {
        assert_eq!(release_decision(1 << bit, 0, Duration::ZERO), Wait);
        assert_eq!(release_decision(0, 1 << bit, Duration::ZERO), Wait);
        assert_eq!(release_decision(1 << bit, 1 << bit, Duration::ZERO), Wait);
    }
}
#[test]
fn release_order_and_logical_lag_do_not_allow_early_paste() {
    for (logical, physical) in [(5, 5), (5, 1), (1, 1), (1, 0)] {
        assert_eq!(
            release_decision(logical, physical, Duration::from_millis(500)),
            Wait
        );
    }
    assert_eq!(release_decision(0, 0, Duration::from_millis(505)), Ready);
}
#[test]
fn one_second_is_the_timeout_boundary() {
    assert_eq!(release_decision(0, 4, Duration::from_millis(999)), Wait);
    assert_eq!(release_decision(0, 4, Duration::from_millis(1000)), Timeout);
    assert_eq!(release_decision(1, 0, Duration::from_secs(2)), Timeout);
    assert_eq!(release_decision(0, 0, Duration::from_secs(1)), Ready);
}
#[test]
fn high_bits_are_not_modifier_slots() {
    assert_eq!(release_decision(0xff00, 0xff00, Duration::ZERO), Ready);
}
