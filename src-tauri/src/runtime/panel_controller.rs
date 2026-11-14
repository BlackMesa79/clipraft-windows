mod session;

use super::{
    apply_settings, hide_panel, main_window, on_main, paste_generation, paused, platform, publish,
    update_error, Control, Job, NoticeDeadline, Panel, State, CAPTURE_GATE, LAST_SEQUENCE, PAUSED,
};
use crate::{
    diag,
    hook::{self, Action},
};
use anyhow::{anyhow, Result};
use std::{
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver},
        Arc,
    },
    thread,
    time::Instant,
};
use tauri::{AppHandle, Emitter};
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;

struct Controller {
    app: AppHandle,
    state: Arc<State>,
    panel: Panel,
    target: isize,
    notice: NoticeDeadline,
    coordinator: Option<crate::panel_neighborhood::Coordinator>,
    loader: crate::panel_neighborhood::Loader,
    next_session: u64,
}

impl Controller {
    fn handle(&mut self, event: Control) -> Result<()> {
        match event {
            Control::RecordedHotkey(packed, session) => {
                main_window::recorded_hotkey(&self.app, packed, session)
            }
            Control::UpdateSettings(settings, reply) => {
                let result = apply_settings(&self.app, &self.state, settings, self.panel.visible)
                    .map_err(|e| e.to_string());
                let _ = reply.send(result);
                Ok(())
            }
            Control::Report(message) => self.show_report(message, None),
            Control::PasteReport(message, generation) => {
                self.show_report(message, Some(generation))
            }
            Control::TogglePause => self.toggle_pause(),
            Control::OpenSettings => self.open_settings(),
            Control::Toggle => self.toggle_main(),
            Control::HistoryChanged => self.history_changed(),
            Control::NeighborhoodReady => self.neighborhood_ready(),
            Control::Hide => main_window::hide_main(&self.app, &self.state),
            Control::Hook(action, generation) => self.action(action, generation),
        }
    }

    fn show_report(&mut self, message: String, generation: Option<u64>) -> Result<()> {
        if generation.is_some_and(|g| g != paste_generation()) {
            return Ok(());
        }
        update_error(&self.state, Some(message.clone()))?;
        if !self.panel.visible {
            self.panel.items.clear();
            self.panel.selected = 0;
        }
        self.panel.error = Some(message.clone());
        publish(&self.app, &self.state, &self.panel)?;
        self.app.emit("operation-error", &message)?;
        self.notice.reset(Instant::now());
        main_window::show_notice(&self.app, &self.state, message, generation)
    }

    fn toggle_pause(&self) -> Result<()> {
        let gate = CAPTURE_GATE.lock().map_err(|_| anyhow!("采集门不可用"))?;
        let mut context = self
            .state
            .capture_context
            .lock()
            .map_err(|_| anyhow!("采集上下文不可用"))?;
        let next = !paused();
        LAST_SEQUENCE.store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
        PAUSED.store(next, Ordering::Release);
        *context = None;
        drop(context);
        drop(gate);
        crate::tray_status::update(&self.app, next);
        Ok(())
    }

    fn open_settings(&mut self) -> Result<()> {
        hook::cancel_panel();
        self.notice.cancel();
        self.close()?;
        main_window::open_settings(&self.app, &self.state)
    }

    fn toggle_main(&mut self) -> Result<()> {
        update_error(&self.state, None)?;
        self.notice.cancel();
        self.panel.error = None;
        self.app.emit("operation-error", "")?;
        hook::cancel_panel();
        self.clear_session()?;
        self.panel.visible = false;
        diag::panel(true, false);
        publish(&self.app, &self.state, &self.panel)?;
        main_window::toggle(&self.app, &self.state)
    }

    fn action(&mut self, action: Action, generation: u64) -> Result<()> {
        if generation != paste_generation() {
            return Ok(());
        }
        match action {
            Action::Open => self.open(generation),
            Action::Next | Action::Previous => self.navigate(action),
            Action::Commit => self.commit(generation),
            Action::Cancel => self.close(),
        }
    }

    fn open(&mut self, generation: u64) -> Result<()> {
        update_error(&self.state, None)?;
        self.target = platform::foreground();
        if !platform::valid_external(self.target) {
            return self.close();
        }
        if let Err(error) = self.start_session() {
            self.close()?;
            return self.show_report(error.to_string(), Some(generation));
        }
        if !self.panel.visible {
            return self.close();
        }
        self.notice.cancel();
        self.app.emit("operation-error", "")?;
        publish(&self.app, &self.state, &self.panel)?;
        main_window::show_quick(&self.app, &self.state, self.target, generation)
    }

    fn commit(&mut self, generation: u64) -> Result<()> {
        let id = self
            .coordinator
            .as_mut()
            .and_then(|session| session.commit());
        self.close()?;
        let Some(id) = id else {
            return Ok(());
        };
        if self
            .state
            .jobs
            .try_send(Job::Paste {
                id,
                target: self.target,
                restore: false,
                plain: false,
                generation,
            })
            .is_err()
        {
            super::report(&self.app, &self.state, "任务队列繁忙，请重试");
        }
        Ok(())
    }

    fn expire_notice(&mut self) -> Result<()> {
        if !self.notice.expire(Instant::now()) {
            return Ok(());
        }
        self.panel.error = None;
        publish(&self.app, &self.state, &self.panel)?;
        self.app.emit("operation-error", "")?;
        if !self.panel.visible {
            let quick = self.state.quick.load(Ordering::Acquire);
            on_main(&self.app, move || platform::hide(quick))?;
        }
        Ok(())
    }

    fn abort(&mut self, error: anyhow::Error) {
        diag::write(format!("panel control failed: {error}"));
        let generation = self.cancel_session();
        if let Err(error) = publish(&self.app, &self.state, &self.panel) {
            diag::write(format!("panel cancellation publish failed: {error}"));
        }
        let quick = self.state.quick.load(Ordering::Acquire);
        if let Err(error) = on_main(&self.app, move || {
            if generation == paste_generation() {
                platform::hide(quick);
            }
        }) {
            diag::write(format!("panel cancellation dispatch failed: {error}"));
        }
    }

    fn cancel_session(&mut self) -> u64 {
        hook::cancel_panel();
        // 失败后连同已排队的显示回调一起作废，旧 Commit 不能复用失效快照。
        let generation = super::new_paste_intent();
        if let Err(error) = self.clear_session() {
            diag::write(format!("panel session cleanup failed: {error}"));
        }
        self.panel = Panel::default();
        self.notice.cancel();
        generation
    }
}

pub(super) fn start(app: AppHandle, state: Arc<State>, controls: Receiver<Control>) -> Result<()> {
    thread::Builder::new()
        .name("panel-controller".into())
        .spawn(move || {
            let mut controller = Controller::new(app, state);
            loop {
                if let Err(error) = controller.expire_notice() {
                    controller.abort(error);
                }
                let event = match controls.recv_timeout(controller.notice.remaining(Instant::now()))
                {
                    Ok(event) => event,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                if let Err(error) = controller.handle(event) {
                    controller.abort(error);
                }
            }
        })?;
    Ok(())
}

#[cfg(test)]
pub(super) mod integration_tests;
#[cfg(test)]
pub(super) mod tests;
