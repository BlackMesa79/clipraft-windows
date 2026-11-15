use super::{refresh, Control, State, LISTENER, PASTE_GENERATION};
use crate::{clipboard, diag, model::Clip, paster, platform};
use anyhow::{anyhow, Result};
use std::sync::atomic::Ordering;
use tauri::AppHandle;

pub(super) struct PasteRequest {
    pub id: String,
    pub target: isize,
    pub restore: bool,
    pub plain: bool,
    pub generation: u64,
}

struct PasteTask<'a> {
    state: &'a State,
    request: PasteRequest,
    intent: paster::PasteIntent<'a>,
    clip: Option<Clip>,
    expected_focus: Option<paster::focus_restore::Target>,
}

impl PasteTask<'_> {
    fn report_failure(&self, error: anyhow::Error) {
        if self.intent.is_current() {
            crate::app_events::record(
                "paste report send failed",
                crate::app_events::send_control(
                    &self.state.control,
                    Control::PasteReport(error.to_string(), self.request.generation),
                ),
            );
        }
    }

    fn load(&mut self) -> Result<()> {
        let loaded = self
            .state
            .storage
            .lock()
            .map_err(|_| anyhow!("存储不可用"))?
            .load_for_paste(&self.request.id)?;
        for format in &loaded.formats {
            // 仅输出固定支持格式的名称和大小，不记录内容或记录标识。
            if clipboard::SUPPORTED
                .iter()
                .any(|(name, _)| *name == format.name)
            {
                diag::write(format!(
                    "paste generation={} format={} bytes={}",
                    self.request.generation,
                    format.name,
                    format.bytes.len()
                ));
            }
        }
        self.clip = Some(loaded);
        Ok(())
    }

    fn write(&self) -> Result<()> {
        let clip = self
            .clip
            .as_ref()
            .ok_or_else(|| anyhow!("粘贴记录尚未加载"))?;
        super::validate_sources(clip, |p| std::path::Path::new(p).exists())?;
        clipboard::write(
            platform::hwnd(LISTENER.load(Ordering::Acquire)),
            clip,
            self.request.plain,
        )
    }

    fn prepare(&self) -> Result<paster::PasteStatus> {
        if !self.intent.is_current() {
            return Ok(paster::PasteStatus::Superseded);
        }
        let main = self.state.main.load(Ordering::Acquire);
        if !self.request.restore {
            paster::prepare_current_target(self.request.target, main)?;
            return Ok(paster::PasteStatus::Complete);
        }
        paster::prepare_restored_target(self.request.target, main)?;
        match self.expected_focus {
            Some(target) => paster::focus_restore::wait_ready(target, &self.intent),
            None => Ok(paster::PasteStatus::Complete),
        }
    }

    fn step(&mut self, stage: paster::PasteStage) -> Result<paster::PasteStatus> {
        match stage {
            paster::PasteStage::Load => self.load()?,
            paster::PasteStage::Write => self.write()?,
            paster::PasteStage::Wait => {
                diag::snapshot("before_wait");
                return paster::wait_for_release(&self.intent);
            }
            paster::PasteStage::Prepare => {
                diag::focus::snapshot(
                    "before_prepare",
                    self.request.target,
                    self.request.generation,
                );
                let prepared = self.prepare();
                diag::focus::snapshot(
                    "after_prepare",
                    self.request.target,
                    self.request.generation,
                );
                return prepared;
            }
            paster::PasteStage::Inject => {
                return paster::inject(self.request.target, &self.intent, self.expected_focus)
            }
            paster::PasteStage::Touch => self
                .state
                .storage
                .lock()
                .map_err(|_| anyhow!("存储不可用"))?
                .touch(&self.request.id)?,
        }
        Ok(paster::PasteStatus::Complete)
    }
}

pub(super) fn run(app: &AppHandle, state: &State, request: PasteRequest) {
    if diag::enabled() {
        diag::write(format!(
            "paste_request generation={} restore={} plain={} target=0x{:X}",
            request.generation, request.restore, request.plain, request.target
        ));
    }
    let expected_focus = match request.restore {
        true => match state.target_focus.lock() {
            Ok(focus) => focus.filter(|focus| focus.window() == request.target),
            Err(error) => {
                super::report(app, state, &format!("输入焦点状态不可用：{error}"));
                return;
            }
        },
        false => None,
    };
    if diag::enabled() {
        diag::write(format!(
            "paste_focus generation={} captured={}",
            request.generation,
            expected_focus.is_some()
        ));
    }
    let mut task = PasteTask {
        state,
        intent: paster::PasteIntent::new(&PASTE_GENERATION, request.generation),
        request,
        clip: None,
        expected_focus,
    };
    let intent = paster::PasteIntent::new(&PASTE_GENERATION, task.request.generation);
    let result = paster::run_paste(&intent, |stage| task.step(stage));
    diag::snapshot(match &result {
        Ok(paster::PasteStatus::Complete) => "paste_complete",
        Ok(paster::PasteStatus::Superseded) => "paste_superseded",
        Err(_) => "paste_cancelled",
    });
    let result = result.and_then(|status| {
        if status == paster::PasteStatus::Complete {
            refresh(app, state)?;
        }
        Ok(())
    });
    if let Err(error) = result {
        task.report_failure(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::test_support::{poison, Fixture};
    use std::sync::{atomic::AtomicU64, mpsc};

    fn request() -> PasteRequest {
        PasteRequest {
            id: "synthetic".into(),
            target: 0,
            restore: false,
            plain: false,
            generation: 7,
        }
    }

    #[test]
    fn poisoned_load_stops_before_clipboard_write_and_touch_reports_error() {
        let fixture = Fixture::new("paste-poison");
        let state = fixture.state();
        poison(&state.storage);
        let generation = AtomicU64::new(7);
        let mut task = PasteTask {
            state: &state,
            request: request(),
            intent: paster::PasteIntent::new(&generation, 7),
            clip: None,
            expected_focus: None,
        };
        let mut visited = Vec::new();
        let intent = paster::PasteIntent::new(&generation, 7);
        let result = paster::run_paste(&intent, |stage| {
            visited.push(stage);
            task.step(stage)
        });
        assert_eq!(result.unwrap_err().to_string(), "存储不可用");
        assert_eq!(visited, [paster::PasteStage::Load]);
        assert!(task.clip.is_none());
        assert_eq!(
            task.step(paster::PasteStage::Touch)
                .unwrap_err()
                .to_string(),
            "存储不可用"
        );
    }

    #[test]
    fn superseded_task_never_loads_and_old_errors_are_not_reported() {
        let fixture = Fixture::new("paste-generation");
        let mut state = fixture.state();
        let (control, messages) = mpsc::channel();
        state.control = control;
        let generation = AtomicU64::new(7);
        let mut task = PasteTask {
            state: &state,
            request: request(),
            intent: paster::PasteIntent::new(&generation, 7),
            clip: None,
            expected_focus: None,
        };
        task.report_failure(anyhow!("synthetic failure"));
        assert!(
            matches!(messages.try_recv().unwrap(), Control::PasteReport(message, 7) if message == "synthetic failure")
        );
        generation.store(8, Ordering::Release);
        task.report_failure(anyhow!("stale failure"));
        assert!(messages.try_recv().is_err());
        let intent = paster::PasteIntent::new(&generation, 7);
        assert_eq!(
            paster::run_paste(&intent, |stage| task.step(stage)).unwrap(),
            paster::PasteStatus::Superseded
        );
        assert!(task.clip.is_none());
    }
}
