use super::*;
use crate::{
    panel_neighborhood::{self, Coordinator, Loader, Request, Response, ViewState},
    panel_session::Direction,
};

pub(super) fn read(state: &State, request: Request) -> Response {
    let storage = match state.storage.lock() {
        Ok(storage) => storage,
        Err(_) => return request.failed("存储不可用".into()),
    };
    let mut response = panel_neighborhood::read(&storage, request);
    if let Response::Views { result, .. } = &mut response {
        *result = result
            .as_mut()
            .map_err(|error| error.clone())
            .and_then(|items| {
                state
                    .files
                    .annotate(items)
                    .map_err(|error| error.to_string())?;
                Ok(std::mem::take(items))
            });
    }
    response
}

impl Controller {
    pub(super) fn new(app: AppHandle, state: Arc<State>) -> Self {
        let reader_state = state.clone();
        let control = state.control.clone();
        let loader = Loader::new(
            move |request| read(&reader_state, request),
            move || {
                crate::app_events::record(
                    "panel neighborhood notification failed",
                    crate::app_events::send_control(&control, Control::NeighborhoodReady),
                )
            },
        );
        Self {
            app,
            state,
            loader,
            panel: Panel::default(),
            target: 0,
            notice: NoticeDeadline::default(),
            coordinator: None,
            next_session: 0,
        }
    }

    pub(super) fn start_session(&mut self) -> Result<()> {
        self.clear_session()?;
        self.panel = Panel::default();
        let stamp = self.state.history_gate.stamp()?;
        if stamp.active != 0 {
            return Err(anyhow!("历史正在更新，请稍后重试"));
        }
        let index = self.state.history_index.snapshot()?;
        let count = self
            .state
            .settings
            .lock()
            .map_err(|_| anyhow!("设置状态不可用"))?
            .quick_visible_count;
        self.next_session += 1;
        let Some((coordinator, request)) =
            Coordinator::open(self.next_session, index.ids.clone(), count as usize)
        else {
            return Ok(());
        };
        diag::write(format!(
            "panel index version={} total={}",
            index.version,
            index.ids.len()
        ));
        self.panel = Panel {
            session: self.next_session,
            visible_count: count,
            visible: true,
            history_revision: stamp.revision,
            ..Default::default()
        };
        self.coordinator = Some(coordinator);
        self.loader.submit(request)?;
        self.sync_view();
        Ok(())
    }

    pub(super) fn clear_session(&mut self) -> Result<()> {
        if let Some(mut coordinator) = self.coordinator.take() {
            coordinator.cancel();
        }
        self.panel.items.clear();
        self.panel.selected = 0;
        self.panel.loading = false;
        self.panel.total = 0;
        self.panel.position = 0;
        self.loader.clear()
    }

    pub(super) fn close(&mut self) -> Result<()> {
        self.clear_session()?;
        self.panel.error = None;
        hide_panel(&self.app, &self.state, &mut self.panel)
    }

    fn sync_view(&mut self) {
        let Some(coordinator) = &self.coordinator else {
            return;
        };
        self.panel.position = coordinator.position().unwrap_or(0);
        self.panel.total = coordinator.total();
        self.panel.items.clear();
        self.panel.selected = 0;
        self.panel.loading = false;
        self.panel.error = None;
        match coordinator.view() {
            ViewState::Empty => {}
            ViewState::Loading => self.panel.loading = true,
            ViewState::Failed(message) => self.panel.error = Some(message.clone()),
            ViewState::Ready { items, selected } => {
                self.panel.items = items.clone();
                self.panel.selected = *selected;
            }
        }
    }

    fn submit(&mut self, request: Option<Request>) -> Result<()> {
        if let Some(request) = request {
            self.loader.submit(request)?;
        }
        self.sync_view();
        if self
            .coordinator
            .as_ref()
            .and_then(Coordinator::selected_id)
            .is_none()
        {
            return self.close();
        }
        publish(&self.app, &self.state, &self.panel)
    }

    pub(super) fn history_changed(&mut self) -> Result<()> {
        if !self.panel.visible {
            return Ok(());
        }
        let stamp = self.state.history_gate.stamp()?;
        if self.panel.history_revision == stamp.revision {
            return Ok(());
        }
        self.loader.clear()?;
        self.panel.history_revision = stamp.revision;
        let request = self
            .coordinator
            .as_mut()
            .and_then(Coordinator::history_changed);
        // 修改期间只失效缓存，等最后一个修改完成再发起完整核对。
        if stamp.active != 0 {
            self.sync_view();
            return publish(&self.app, &self.state, &self.panel);
        }
        self.submit(request)
    }

    pub(super) fn neighborhood_ready(&mut self) -> Result<()> {
        self.history_changed()?;
        let Some(response) = self.loader.take_response()? else {
            return Ok(());
        };
        let request = self
            .coordinator
            .as_mut()
            .and_then(|session| session.apply(response));
        self.submit(request)
    }

    pub(super) fn navigate(&mut self, action: Action) -> Result<()> {
        if !self.panel.visible {
            return Ok(());
        }
        self.history_changed()?;
        let direction = match action {
            Action::Next => Direction::Next,
            Action::Previous => Direction::Previous,
            Action::Open | Action::Commit | Action::Cancel => return Ok(()),
        };
        let request = self
            .coordinator
            .as_mut()
            .and_then(|session| session.navigate(direction));
        self.submit(request)
    }
}
