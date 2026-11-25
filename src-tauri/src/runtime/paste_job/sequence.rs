use super::{PasteRequest, PasteTask};
use crate::{
    app_events, diag,
    paster::{self, PasteIntent, PasteStage, PasteStatus},
    platform,
    runtime::{self, Control, PasteContent, State, PASTE_GENERATION},
};
use anyhow::{anyhow, bail, Result};
use std::{
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, EventTarget, Manager};

const PASTE_INTERVAL: Duration = Duration::from_millis(200);
const CHECK_INTERVAL: Duration = Duration::from_millis(10);

pub(in crate::runtime) struct Request {
    pub target: isize,
    pub generation: u64,
    pub escape_revision: u64,
}

impl Request {
    fn check_escape(&self) -> Result<()> {
        if runtime::paste_escape_revision() != self.escape_revision {
            bail!("已按 Esc 取消");
        }
        Ok(())
    }

    fn check_target(&self) -> Result<()> {
        self.check_escape()?;
        if platform::foreground() != self.target {
            bail!("前台窗口已变化");
        }
        Ok(())
    }
}

struct Sequence<'a> {
    state: &'a State,
    request: Request,
    focus: Option<paster::focus_restore::Target>,
    completed: usize,
}

impl Sequence<'_> {
    fn run(&mut self, ids: Vec<String>, intent: &PasteIntent<'_>) -> Result<PasteStatus> {
        for id in ids {
            let result = self.paste(id, intent)?;
            if result == PasteStatus::Superseded {
                return Ok(result);
            }
        }
        Ok(PasteStatus::Complete)
    }

    fn paste(&mut self, id: String, intent: &PasteIntent<'_>) -> Result<PasteStatus> {
        let first = self.completed == 0;
        if !first {
            let start = Instant::now();
            let status = wait_interval(
                intent,
                || self.request.check_target(),
                || start.elapsed(),
                thread::sleep,
            )?;
            if status == PasteStatus::Superseded {
                return Ok(status);
            }
        }
        let mut task = PasteTask {
            state: self.state,
            request: PasteRequest {
                content: PasteContent::Item(id),
                target: self.request.target,
                restore: first,
                plain: false,
                generation: self.request.generation,
            },
            intent: PasteIntent::new(&PASTE_GENERATION, self.request.generation),
            clip: None,
            expected_focus: self.focus,
        };
        let stages = [
            PasteStage::Load,
            PasteStage::Wait,
            PasteStage::Prepare,
            PasteStage::Write,
            PasteStage::Inject,
            PasteStage::Touch,
        ];
        paster::run_paste_stages(intent, stages, |stage| self.step(&mut task, stage))
    }

    fn step(&mut self, task: &mut PasteTask<'_>, stage: PasteStage) -> Result<PasteStatus> {
        match stage {
            PasteStage::Touch => self.completed += 1,
            PasteStage::Wait => {
                return paster::wait_for_release_checked(&task.intent, || {
                    if task.request.restore {
                        self.request.check_escape()
                    } else {
                        self.request.check_target()
                    }
                })
            }
            PasteStage::Prepare if !task.request.restore => {
                self.check_focus()?;
                return Ok(PasteStatus::Complete);
            }
            PasteStage::Write => self.check_focus()?,
            PasteStage::Inject => {
                return paster::inject_checked(
                    self.request.target,
                    &task.intent,
                    self.focus,
                    || self.request.check_target(),
                )
            }
            _ => self.request.check_escape()?,
        }
        let status = task.step(stage)?;
        if stage == PasteStage::Prepare && status == PasteStatus::Complete && self.focus.is_none() {
            self.focus = paster::focus_restore::capture(self.request.target);
        }
        Ok(status)
    }

    fn check_focus(&self) -> Result<()> {
        self.request.check_target()?;
        if let Some(focus) = self.focus {
            paster::focus_restore::verify(focus)?;
        }
        Ok(())
    }
}

fn wait_interval(
    intent: &PasteIntent<'_>,
    mut check: impl FnMut() -> Result<()>,
    mut elapsed: impl FnMut() -> Duration,
    mut pause: impl FnMut(Duration),
) -> Result<PasteStatus> {
    loop {
        if !intent.is_current() {
            return Ok(PasteStatus::Superseded);
        }
        check()?;
        let remaining = PASTE_INTERVAL.saturating_sub(elapsed());
        if remaining.is_zero() {
            return Ok(PasteStatus::Complete);
        }
        pause(remaining.min(CHECK_INTERVAL));
    }
}

pub(in crate::runtime) fn run(app: &AppHandle, state: &State, ids: Vec<String>, request: Request) {
    let total = ids.len();
    let generation = request.generation;
    let intent = PasteIntent::new(&PASTE_GENERATION, generation);
    let mut sequence = Sequence {
        state,
        request,
        focus: None,
        completed: 0,
    };
    let result = state
        .target_focus
        .lock()
        .map(|focus| {
            sequence.focus = focus.filter(|focus| focus.window() == sequence.request.target);
        })
        .map_err(|_| anyhow!("输入焦点状态不可用"))
        .and_then(|()| sequence.run(ids, &intent));
    let completed = sequence.completed;
    if completed != 0 {
        app_events::record(
            "batch paste history refresh failed",
            runtime::refresh(app, state),
        );
    }
    let message = match result {
        Ok(PasteStatus::Complete) => return,
        Ok(PasteStatus::Superseded) => format!("依次粘贴已停止，已粘贴 {completed}/{total} 条"),
        Err(error) if completed == total => {
            format!("已粘贴 {completed}/{total} 条，更新使用记录失败：{error}")
        }
        Err(error) => format!("依次粘贴已停止，已粘贴 {completed}/{total} 条：{error}"),
    };
    report(app, state, &intent, generation, message);
}

fn report(
    app: &AppHandle,
    state: &State,
    intent: &PasteIntent<'_>,
    generation: u64,
    message: String,
) {
    if intent.is_current() {
        app_events::record(
            "batch paste report failed",
            app_events::send_control(&state.control, Control::PasteReport(message, generation)),
        );
        return;
    }
    // 主面板已重新打开时，在面板内报告进度，不干扰后续快速面板操作。
    let handle = app.clone();
    if let Err(error) = runtime::on_main(app, move || {
        if runtime::paste_generation() == generation.wrapping_add(1)
            && handle
                .get_webview_window("main")
                .is_some_and(|window| window.is_focused().unwrap_or(false))
        {
            app_events::record(
                "batch paste progress failed",
                handle.emit_to(
                    EventTarget::webview_window("main"),
                    "operation-error",
                    message,
                ),
            );
        }
    }) {
        diag::write(format!("batch paste progress dispatch failed: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::atomic::{AtomicU64, Ordering},
    };

    #[test]
    fn interval_waits_exactly_two_hundred_milliseconds_with_regular_checks() {
        let generation = AtomicU64::new(1);
        let intent = PasteIntent::new(&generation, 1);
        let time = Cell::new(Duration::ZERO);
        let checks = Cell::new(0);
        let status = wait_interval(
            &intent,
            || {
                checks.set(checks.get() + 1);
                Ok(())
            },
            || time.get(),
            |delay| time.set(time.get() + delay),
        )
        .unwrap();
        assert_eq!(status, PasteStatus::Complete);
        assert_eq!(time.get(), Duration::from_millis(200));
        assert_eq!(checks.get(), 21);
    }

    #[test]
    fn interval_stops_on_focus_change_or_escape_without_waiting_again() {
        for reason in ["前台窗口已变化", "已按 Esc 取消"] {
            let generation = AtomicU64::new(1);
            let intent = PasteIntent::new(&generation, 1);
            let time = Cell::new(Duration::ZERO);
            let result = wait_interval(
                &intent,
                || {
                    if !time.get().is_zero() {
                        bail!("{reason}");
                    }
                    Ok(())
                },
                || time.get(),
                |delay| time.set(time.get() + delay),
            );
            assert_eq!(result.unwrap_err().to_string(), reason);
            assert_eq!(time.get(), CHECK_INTERVAL);
        }
    }

    #[test]
    fn new_generation_stops_before_checking_the_old_target_again() {
        let generation = AtomicU64::new(1);
        let intent = PasteIntent::new(&generation, 1);
        let checks = Cell::new(0);
        let status = wait_interval(
            &intent,
            || {
                checks.set(checks.get() + 1);
                Ok(())
            },
            || Duration::ZERO,
            |_| generation.store(2, Ordering::Release),
        )
        .unwrap();
        assert_eq!(status, PasteStatus::Superseded);
        assert_eq!(checks.get(), 1);
    }

    #[test]
    fn missing_record_stops_before_touching_following_records() {
        let fixture = runtime::test_support::Fixture::new("paste-sequence-missing");
        let state = fixture.state();
        let generation = AtomicU64::new(1);
        let intent = PasteIntent::new(&generation, 1);
        let mut sequence = Sequence {
            state: &state,
            request: Request {
                target: 0,
                generation: 1,
                escape_revision: runtime::paste_escape_revision(),
            },
            focus: None,
            completed: 0,
        };
        let error = sequence
            .run(vec!["missing".into(), "later".into()], &intent)
            .unwrap_err();
        assert_eq!(error.to_string(), "记录已删除或到期，请重新选择");
        assert_eq!(sequence.completed, 0);
    }
}
