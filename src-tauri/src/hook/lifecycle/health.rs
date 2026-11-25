//! 低频检测键盘/鼠标钩子活性并在钩子线程内恢复注册。
use super::*;
use anyhow::Context;
use std::sync::atomic::Ordering;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

const HEALTH_TIMER_ID: usize = 0x4B32;
const HEALTH_CHECK_INTERVAL_MS: u32 = 5_000;
const HEALTH_EVENT_GAP_MS: u32 = 15_000;
const HEALTH_CONFIRMATIONS: u8 = 2;
const HEALTH_RETRY_DELAY_MS: u32 = 60_000;

pub(super) struct HealthTimer;

impl HealthTimer {
    pub(super) fn install() -> Result<Self> {
        if unsafe { SetTimer(None, HEALTH_TIMER_ID, HEALTH_CHECK_INTERVAL_MS, None) } == 0 {
            return Err(windows::core::Error::from_win32()).context("钩子健康检查定时器创建失败");
        }
        Ok(Self)
    }
}

impl Drop for HealthTimer {
    fn drop(&mut self) {
        crate::app_events::record("hook health timer removal failed", unsafe {
            KillTimer(None, HEALTH_TIMER_ID)
        });
    }
}

#[derive(Default)]
pub(super) struct HookHealth {
    suspicions: u8,
    retry_after: u32,
}

impl HookHealth {
    pub(super) fn should_recover(&mut self, system_tick: u32, hook_tick: u32, now: u32) -> bool {
        if self.retry_after != 0 && (now.wrapping_sub(self.retry_after) as i32) < 0 {
            return false;
        }
        let gap = system_tick.wrapping_sub(hook_tick) as i32;
        if gap <= HEALTH_EVENT_GAP_MS as i32 {
            self.suspicions = 0;
            return false;
        }
        self.suspicions = self.suspicions.saturating_add(1);
        if self.suspicions < HEALTH_CONFIRMATIONS {
            return false;
        }
        self.suspicions = 0;
        true
    }

    pub(super) fn recovery_result(&mut self, now: u32, success: bool) {
        self.retry_after = if success {
            0
        } else {
            now.wrapping_add(HEALTH_RETRY_DELAY_MS)
        };
    }
}

pub(super) fn last_input_tick() -> Result<u32> {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return Err(windows::core::Error::from_win32()).context("读取系统最近输入时间失败");
    }
    Ok(info.dwTime)
}

pub(super) fn dispatch_health(
    message: &MSG,
    registration: &mut Option<Registration>,
    health: &mut HookHealth,
) {
    if message.message != WM_TIMER || message.wParam.0 != HEALTH_TIMER_ID {
        dispatch(message);
        return;
    }
    check_hook_health(registration, health);
}

fn check_hook_health(registration: &mut Option<Registration>, health: &mut HookHealth) {
    let system_tick = match last_input_tick() {
        Ok(tick) => tick,
        Err(error) => {
            crate::diag::write(format!("hook health input query failed: {error:#}"));
            return;
        }
    };
    let now = unsafe { GetTickCount() };
    if !health.should_recover(
        system_tick,
        LAST_HOOK_EVENT_TICK.load(Ordering::Acquire),
        now,
    ) {
        return;
    }
    cancel_panel();
    match replace_registration(registration, Registration::install) {
        Ok(()) => {
            seed_state();
            LAST_HOOK_EVENT_TICK.store(system_tick, Ordering::Release);
            health.recovery_result(now, true);
            crate::diag::write("hook recovery result=success".into());
            notify_recovery(true);
        }
        Err(error) => {
            health.recovery_result(now, false);
            crate::diag::write(format!("hook recovery result=failure error={error:#}"));
            notify_recovery(false);
        }
    }
}

pub(super) fn replace_registration<T>(
    registration: &mut Option<T>,
    install: impl FnOnce() -> Result<T>,
) -> Result<()> {
    drop(registration.take());
    *registration = Some(install()?);
    Ok(())
}

fn notify_recovery(recovered: bool) {
    let receiver = RECEIVER.load(Ordering::Acquire);
    if receiver == 0 {
        return;
    }
    crate::app_events::record("hook health notification post failed", unsafe {
        PostThreadMessageW(
            receiver,
            HEALTH_MESSAGE,
            WPARAM(usize::from(recovered)),
            LPARAM(0),
        )
    });
}
