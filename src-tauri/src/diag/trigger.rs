//! 仅配置触发键的固定数值诊断；钩子不格式化字符串、不查询窗口、不等待文件。
use super::*;
pub(super) const TRIGGER_MESSAGE: u32 = WM_APP + 93;
pub(super) const DISPATCH_MESSAGE: u32 = WM_APP + 94;
pub(super) const RECONCILE_MESSAGE: u32 = WM_APP + 95;

pub struct Trigger {
    pub down: bool,
    pub repeated: bool,
    pub active: bool,
    pub ready: bool,
    pub reserved: bool,
    pub recording: bool,
    pub held: u16,
    pub expected: u16,
    pub action: Option<hook::Action>,
}
fn pack(value: Trigger) -> usize {
    usize::from(value.down)
        | (usize::from(value.repeated) << 1)
        | (usize::from(value.active) << 2)
        | (usize::from(value.ready) << 3)
        | (usize::from(value.reserved) << 4)
        | (usize::from(value.recording) << 5)
        | (usize::from(value.held & 15) << 8)
        | (usize::from(value.expected & 15) << 12)
        | (value.action.map_or(0, |action| action as usize) << 16)
}
fn post(message: u32, value: usize, stamp: u64) {
    let thread = THREAD.load(Ordering::Relaxed);
    if thread == 0 {
        return;
    }
    if unsafe { PostThreadMessageW(thread, message, WPARAM(value), LPARAM(stamp as isize)) }
        .is_err()
    {
        LOST.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn reconciled() {
    post(RECONCILE_MESSAGE, 0, millis());
}
pub fn record(value: Trigger) {
    post(TRIGGER_MESSAGE, pack(value), millis());
}
pub fn dispatch(action: hook::Action, generation: u64, accepted: bool) {
    post(
        DISPATCH_MESSAGE,
        action as usize | (usize::from(accepted) << 8),
        generation,
    );
}
pub(super) fn write(writer: &mut impl Write, message: &MSG) -> std::io::Result<()> {
    if message.message == RECONCILE_MESSAGE {
        return writeln!(
            writer,
            "t_ms={} trigger_reconciled source=modifier_sample cached_down=true system_down=false",
            message.lParam.0
        );
    }
    let value = message.wParam.0;
    if message.message == DISPATCH_MESSAGE {
        return writeln!(
            writer,
            "t_ms={} hook_dispatch generation={} action={:?} accepted={}",
            millis(),
            message.lParam.0,
            hook::Action::from_raw(value & 7),
            value & 256 != 0
        );
    }
    writeln!(writer, "t_ms={} quick_trigger down={} repeated={} active_before={} ready={} gate_reserved={} recording={} held=0x{:X} expected=0x{:X} action={:?}",
        message.lParam.0, value & 1 != 0, value & 2 != 0, value & 4 != 0, value & 8 != 0,
        value & 16 != 0, value & 32 != 0, (value >> 8) & 15, (value >> 12) & 15, hook::Action::from_raw((value >> 16) & 7))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trigger_metadata_contains_no_key_or_content_and_preserves_failed_gate() {
        let value = Trigger {
            down: true,
            repeated: false,
            active: false,
            ready: false,
            reserved: true,
            recording: false,
            held: 3,
            expected: 3,
            action: None,
        };
        let msg = MSG {
            message: TRIGGER_MESSAGE,
            wParam: WPARAM(pack(value)),
            lParam: LPARAM(42),
            ..Default::default()
        };
        let mut bytes = Vec::new();
        write(&mut bytes, &msg).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("t_ms=42 quick_trigger"));
        assert!(text.contains("ready=false gate_reserved=true"));
        assert!(text.contains("held=0x3 expected=0x3 action=None"));
        assert!(!text.contains("vk="));
    }
}
