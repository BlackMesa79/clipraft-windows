use crate::{diag, hook::MODIFIER_KEYS, platform};
pub(crate) mod focus_restore;
use anyhow::{bail, Result};
use std::{
    sync::atomic::{AtomicU64, Ordering},
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
fn validate_target(target: isize, main: isize) -> Result<()> {
    if !platform::valid_external(target) {
        bail!("原目标窗口已失效");
    }
    if elevated(target)? && !elevated(main)? {
        bail!("目标窗口以管理员权限运行，当前进程不能向其发送按键");
    }
    Ok(())
}
pub fn prepare_current_target(target: isize, main: isize) -> Result<()> {
    validate_target(target, main)?;
    wait_for_target(target, platform::foreground, thread::sleep)
}
pub fn prepare_restored_target(target: isize, main: isize) -> Result<()> {
    validate_target(target, main)?;
    if !unsafe { SetForegroundWindow(platform::hwnd(target)) }.as_bool() {
        bail!("Windows 拒绝恢复目标窗口焦点");
    }
    wait_for_target(target, platform::foreground, thread::sleep)
}
fn wait_for_target(
    target: isize,
    mut foreground: impl FnMut() -> isize,
    mut pause: impl FnMut(Duration),
) -> Result<()> {
    for _ in 0..10 {
        if foreground() == target {
            return Ok(());
        }
        pause(Duration::from_millis(10));
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
    Superseded,
}
pub fn release_decision(logical: u16, elapsed: Duration) -> ReleaseDecision {
    // 钩子可能丢失抬起事件；粘贴只以系统逻辑状态为准。
    if logical & 0xff == 0 {
        ReleaseDecision::Ready
    } else if elapsed >= Duration::from_secs(1) {
        ReleaseDecision::Timeout
    } else {
        ReleaseDecision::Wait
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteStatus {
    Complete,
    Superseded,
}
pub struct PasteIntent<'a> {
    generation: &'a AtomicU64,
    expected: u64,
}
impl<'a> PasteIntent<'a> {
    pub fn new(generation: &'a AtomicU64, expected: u64) -> Self {
        Self {
            generation,
            expected,
        }
    }
    pub fn is_current(&self) -> bool {
        self.generation.load(Ordering::Acquire) == self.expected
    }
}
pub fn intent_release_decision(current: bool, logical: u16, elapsed: Duration) -> ReleaseDecision {
    // 新意图优先于释放和超时，旧任务不能因为仍按着新快捷键而报错。
    if !current {
        ReleaseDecision::Superseded
    } else {
        release_decision(logical, elapsed)
    }
}
fn wait_with(
    intent: &PasteIntent<'_>,
    mut check: impl FnMut() -> Result<()>,
    mut sample: impl FnMut() -> (u16, Duration),
    mut pause: impl FnMut(),
) -> Result<PasteStatus> {
    loop {
        if !intent.is_current() {
            return Ok(PasteStatus::Superseded);
        }
        check()?;
        let (logical, elapsed) = sample();
        match intent_release_decision(intent.is_current(), logical, elapsed) {
            ReleaseDecision::Ready => return Ok(PasteStatus::Complete),
            ReleaseDecision::Superseded => return Ok(PasteStatus::Superseded),
            ReleaseDecision::Timeout => bail!("请松开修饰键后重试"),
            ReleaseDecision::Wait => pause(),
        }
    }
}
pub fn wait_for_release(intent: &PasteIntent<'_>) -> Result<PasteStatus> {
    wait_for_release_checked(intent, || Ok(()))
}
pub(crate) fn wait_for_release_checked(
    intent: &PasteIntent<'_>,
    check: impl FnMut() -> Result<()>,
) -> Result<PasteStatus> {
    let start = Instant::now();
    wait_with(
        intent,
        check,
        || (logical(), start.elapsed()),
        || thread::sleep(Duration::from_millis(5)),
    )
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteStage {
    Load,
    Write,
    Wait,
    Prepare,
    Inject,
    Touch,
}
impl PasteStage {
    fn name(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Write => "write",
            Self::Wait => "wait",
            Self::Prepare => "prepare",
            Self::Inject => "inject",
            Self::Touch => "touch",
        }
    }
}
// 两个入口共用这条顺序；闭包隔离系统副作用，测试无需操作键鼠或剪贴板。
pub fn run_paste(
    intent: &PasteIntent<'_>,
    step: impl FnMut(PasteStage) -> Result<PasteStatus>,
) -> Result<PasteStatus> {
    run_paste_stages(
        intent,
        [
            PasteStage::Load,
            PasteStage::Write,
            PasteStage::Wait,
            PasteStage::Prepare,
            PasteStage::Inject,
            PasteStage::Touch,
        ],
        step,
    )
}
pub(crate) fn run_paste_stages(
    intent: &PasteIntent<'_>,
    stages: impl IntoIterator<Item = PasteStage>,
    mut step: impl FnMut(PasteStage) -> Result<PasteStatus>,
) -> Result<PasteStatus> {
    for stage in stages {
        // 已发送成功的任务仍应置顶；此前任一阶段过期都立即停止。
        if stage != PasteStage::Touch && !intent.is_current() {
            return Ok(PasteStatus::Superseded);
        }
        let start = Instant::now();
        let mut result = step(stage);
        if stage != PasteStage::Touch && !intent.is_current() && result.is_err() {
            result = Ok(PasteStatus::Superseded);
        }
        let status = match &result {
            Ok(PasteStatus::Complete) => "ok",
            Ok(PasteStatus::Superseded) => "superseded",
            Err(_) => "error",
        };
        diag::write(format!(
            "paste generation={} phase={} elapsed_ms={:.3} status={status}",
            intent.expected,
            stage.name(),
            start.elapsed().as_secs_f64() * 1000.0
        ));
        if result? == PasteStatus::Superseded {
            return Ok(PasteStatus::Superseded);
        }
    }
    Ok(PasteStatus::Complete)
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
pub(crate) fn inject(
    target: isize,
    intent: &PasteIntent<'_>,
    restored: Option<focus_restore::Target>,
) -> Result<PasteStatus> {
    inject_checked(target, intent, restored, || Ok(()))
}
pub(crate) fn inject_checked(
    target: isize,
    intent: &PasteIntent<'_>,
    restored: Option<focus_restore::Target>,
    mut check: impl FnMut() -> Result<()>,
) -> Result<PasteStatus> {
    // 准备焦点期间若又按下修饰键，仍按代号等待；只有真实超时才报错。
    if wait_for_release_checked(intent, &mut check)? == PasteStatus::Superseded {
        return Ok(PasteStatus::Superseded);
    }
    if platform::foreground() != target {
        bail!("前台窗口已变化，已取消按键注入");
    }
    diag::snapshot("before_send");
    diag::focus::snapshot("before_send", target, intent.expected);
    if let Some(restored) = restored {
        focus_restore::verify(restored)?;
    }
    check()?;
    // 紧邻系统调用再次检查，避免诊断和焦点采样期间的新 Open 被旧任务越过。
    if !intent.is_current() {
        return Ok(PasteStatus::Superseded);
    }
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
    diag::focus::snapshot("after_send", target, intent.expected);
    if sent != 4 {
        bail!("SendInput 未完整发送；可能受到目标权限或桌面限制");
    }
    Ok(PasteStatus::Complete)
}
#[cfg(test)]
mod tests {
    #[test]
    fn target_poll_preserves_probe_and_pause_order() {
        use std::cell::RefCell;
        for success_at in [Some(0), Some(4), Some(9), None] {
            let events = RefCell::new(Vec::new());
            let mut probes = 0;
            let result = super::wait_for_target(
                7,
                || {
                    events.borrow_mut().push("probe");
                    let found = success_at == Some(probes);
                    probes += 1;
                    if found {
                        7
                    } else {
                        3
                    }
                },
                |delay| {
                    assert_eq!(delay, Duration::from_millis(10));
                    events.borrow_mut().push("pause");
                },
            );
            let failures = success_at.unwrap_or(10);
            let mut expected = ["probe", "pause"].repeat(failures);
            if success_at.is_some() {
                expected.push("probe");
            }
            assert_eq!(*events.borrow(), expected);
            assert_eq!(result.is_ok(), success_at.is_some());
            if let Err(error) = result {
                assert_eq!(error.to_string(), "前台窗口已变化，已取消粘贴");
            }
        }
    }

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

#[cfg(test)]
mod intent_tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn superseded_intent_wins_over_release_and_timeout() {
        for logical in [0, 1, 0xff] {
            for elapsed in [
                Duration::ZERO,
                Duration::from_secs(1),
                Duration::from_secs(2),
            ] {
                assert_eq!(
                    intent_release_decision(false, logical, elapsed),
                    ReleaseDecision::Superseded
                );
            }
        }
    }
    #[test]
    fn generation_change_during_wait_stops_without_error_or_extra_poll() {
        let generation = AtomicU64::new(7);
        let intent = PasteIntent::new(&generation, 7);
        let samples = Cell::new(0);
        let result = wait_with(
            &intent,
            || Ok(()),
            || {
                samples.set(samples.get() + 1);
                (1, Duration::ZERO)
            },
            || {
                generation.fetch_add(1, Ordering::AcqRel);
            },
        )
        .unwrap();
        assert_eq!(result, PasteStatus::Superseded);
        assert_eq!(samples.get(), 1);
    }
    #[test]
    fn unchanged_generation_reports_error_only_at_timeout() {
        let generation = AtomicU64::new(7);
        let elapsed = Cell::new(999);
        let pauses = Cell::new(0);
        let error = wait_with(
            &PasteIntent::new(&generation, 7),
            || Ok(()),
            || (1, Duration::from_millis(elapsed.get())),
            || {
                pauses.set(pauses.get() + 1);
                elapsed.set(1000);
            },
        )
        .unwrap_err();
        assert_eq!(pauses.get(), 1);
        assert_eq!(error.to_string(), "请松开修饰键后重试");
    }
    #[test]
    fn generation_change_in_sample_suppresses_timeout() {
        let generation = AtomicU64::new(7);
        let result = wait_with(
            &PasteIntent::new(&generation, 7),
            || Ok(()),
            || {
                generation.store(8, Ordering::Release);
                (1, Duration::from_secs(1))
            },
            || panic!("过期任务不应继续等待"),
        )
        .unwrap();
        assert_eq!(result, PasteStatus::Superseded);
    }
    #[test]
    fn releasing_modifiers_completes_before_timeout() {
        let generation = AtomicU64::new(7);
        let held = Cell::new(true);
        assert_eq!(
            wait_with(
                &PasteIntent::new(&generation, 7),
                || Ok(()),
                || (u16::from(held.get()), Duration::from_millis(20)),
                || held.set(false)
            )
            .unwrap(),
            PasteStatus::Complete
        );
    }
    #[test]
    fn flow_loads_and_writes_before_waiting_and_touches_last() {
        let generation = AtomicU64::new(1);
        let mut stages = Vec::new();
        let result = run_paste(&PasteIntent::new(&generation, 1), |stage| {
            stages.push(stage);
            Ok(PasteStatus::Complete)
        })
        .unwrap();
        assert_eq!(result, PasteStatus::Complete);
        assert_eq!(
            stages,
            [
                PasteStage::Load,
                PasteStage::Write,
                PasteStage::Wait,
                PasteStage::Prepare,
                PasteStage::Inject,
                PasteStage::Touch
            ]
        );
    }
    #[test]
    fn superseded_queued_job_does_not_load_or_write() {
        let generation = AtomicU64::new(2);
        assert_eq!(
            run_paste(&PasteIntent::new(&generation, 1), |_| panic!(
                "旧任务不得开始加载"
            ))
            .unwrap(),
            PasteStatus::Superseded
        );
    }
    #[test]
    fn new_intent_after_each_pre_inject_stage_stops_remaining_work() {
        for last in [
            PasteStage::Load,
            PasteStage::Write,
            PasteStage::Wait,
            PasteStage::Prepare,
        ] {
            let generation = AtomicU64::new(1);
            let mut stages = Vec::new();
            let result = run_paste(&PasteIntent::new(&generation, 1), |stage| {
                stages.push(stage);
                if stage == last {
                    generation.store(2, Ordering::Release);
                }
                Ok(PasteStatus::Complete)
            })
            .unwrap();
            assert_eq!(result, PasteStatus::Superseded);
            assert_eq!(stages.last(), Some(&last));
            assert!(!stages.contains(&PasteStage::Inject));
            assert!(!stages.contains(&PasteStage::Touch));
        }
    }
    #[test]
    fn superseded_stage_error_is_silent_but_current_error_propagates() {
        for change in [false, true] {
            let generation = AtomicU64::new(1);
            let result = run_paste(&PasteIntent::new(&generation, 1), |stage| {
                if stage == PasteStage::Write {
                    if change {
                        generation.store(2, Ordering::Release);
                    }
                    bail!("合成写回失败");
                }
                Ok(PasteStatus::Complete)
            });
            if change {
                assert_eq!(result.unwrap(), PasteStatus::Superseded);
            } else {
                assert_eq!(result.unwrap_err().to_string(), "合成写回失败");
            }
        }
    }
    #[test]
    fn cancelled_or_failed_injection_never_touches() {
        for failed in [false, true] {
            let generation = AtomicU64::new(1);
            let result = run_paste(&PasteIntent::new(&generation, 1), |stage| {
                assert_ne!(stage, PasteStage::Touch);
                if stage == PasteStage::Inject {
                    if failed {
                        bail!("合成注入失败");
                    }
                    return Ok(PasteStatus::Superseded);
                }
                Ok(PasteStatus::Complete)
            });
            assert_eq!(result.is_err(), failed);
        }
    }
    #[test]
    fn already_sent_paste_still_touches_when_next_intent_arrives() {
        let generation = AtomicU64::new(1);
        let mut touched = false;
        assert_eq!(
            run_paste(&PasteIntent::new(&generation, 1), |stage| {
                if stage == PasteStage::Inject {
                    generation.store(2, Ordering::Release);
                }
                if stage == PasteStage::Touch {
                    touched = true;
                }
                Ok(PasteStatus::Complete)
            })
            .unwrap(),
            PasteStatus::Complete
        );
        assert!(touched);
    }
}
