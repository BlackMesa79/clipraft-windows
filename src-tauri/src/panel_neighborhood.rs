//! 异步邻域的会话协调；实际 SQL 与调度分别位于 read 和 loader。
mod loader;
mod read;
pub use loader::Loader;
pub use read::read;

use crate::{
    model::ItemView,
    panel_session::{Direction, NeighborhoodRequest, RequestVersion, Session},
};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SafetyVersion {
    session: u64,
    revision: u64,
}

#[derive(Clone, Debug)]
pub enum Request {
    Views(NeighborhoodRequest),
    Validate {
        version: SafetyVersion,
        ids: Arc<[String]>,
    },
}

#[derive(Debug)]
pub enum Response {
    Views {
        request: NeighborhoodRequest,
        result: Result<Vec<ItemView>, String>,
    },
    Validated {
        version: SafetyVersion,
        result: Result<HashSet<String>, String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Views(RequestVersion),
    Validate(SafetyVersion),
}

impl Request {
    fn key(&self) -> Key {
        match self {
            Self::Views(request) => Key::Views(request.version),
            Self::Validate { version, .. } => Key::Validate(*version),
        }
    }

    pub fn failed(self, message: String) -> Response {
        match self {
            Self::Views(request) => Response::Views {
                request,
                result: Err(message),
            },
            Self::Validate { version, .. } => Response::Validated {
                version,
                result: Err(message),
            },
        }
    }
}

impl Response {
    fn key(&self) -> Key {
        match self {
            Self::Views { request, .. } => Key::Views(request.version),
            Self::Validated { version, .. } => Key::Validate(*version),
        }
    }
}

#[derive(Debug)]
pub enum ViewState {
    Empty,
    Loading,
    Ready {
        items: Vec<ItemView>,
        selected: usize,
    },
    Failed(String),
}

#[derive(PartialEq, Eq)]
enum Safety {
    Ready,
    Checking(SafetyVersion),
    Failed,
}

pub struct Coordinator {
    session: Option<Session>,
    safety_version: SafetyVersion,
    safety: Safety,
    visible_count: usize,
    view: ViewState,
}

impl Coordinator {
    pub fn open(session: u64, ids: Arc<[String]>, visible_count: usize) -> Option<(Self, Request)> {
        let mut coordinator = Self {
            session: Some(Session::open(session, ids)?),
            safety_version: SafetyVersion {
                session,
                revision: 0,
            },
            safety: Safety::Ready,
            visible_count,
            view: ViewState::Empty,
        };
        let request = coordinator.request_views()?;
        Some((coordinator, request))
    }

    pub fn view(&self) -> &ViewState {
        &self.view
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.session.as_ref()?.selected_id()
    }

    pub fn position(&self) -> Option<usize> {
        self.session.as_ref()?.position()
    }

    pub fn total(&self) -> usize {
        self.session.as_ref().map_or(0, Session::total)
    }

    fn request_views(&mut self) -> Option<Request> {
        let request = self.session.as_mut()?.request(self.visible_count)?;
        self.view = ViewState::Loading;
        Some(Request::Views(request))
    }

    pub fn navigate(&mut self, direction: Direction) -> Option<Request> {
        self.session.as_mut()?.navigate(direction);
        match self.safety {
            Safety::Ready => self.request_views(),
            Safety::Checking(_) => None,
            Safety::Failed => self.history_changed(),
        }
    }

    /// 先清除旧正文，再异步核对整个冻结顺序，不能把邻域缺席当作全局删除。
    pub fn history_changed(&mut self) -> Option<Request> {
        let session = self.session.as_mut()?;
        session.invalidate();
        self.safety_version.revision += 1;
        self.safety = Safety::Checking(self.safety_version);
        self.view = ViewState::Loading;
        Some(Request::Validate {
            version: self.safety_version,
            ids: session.id_snapshot(),
        })
    }

    pub fn retry(&mut self) -> Option<Request> {
        match self.safety {
            Safety::Ready => self.request_views(),
            Safety::Checking(_) => None,
            Safety::Failed => self.history_changed(),
        }
    }

    pub fn apply(&mut self, response: Response) -> Option<Request> {
        match response {
            Response::Views { request, result } => self.apply_views(request, result),
            Response::Validated { version, result } => self.apply_validation(version, result),
        }
    }

    fn apply_views(
        &mut self,
        request: NeighborhoodRequest,
        result: Result<Vec<ItemView>, String>,
    ) -> Option<Request> {
        if !self.session.as_ref()?.accepts(request.version) {
            return None;
        }
        let items = match result {
            Ok(items) => items,
            Err(message) => {
                self.view = ViewState::Failed(message);
                return None;
            }
        };
        let selected_id = self.selected_id()?;
        let Some(selected) = items.iter().position(|item| item.id == selected_id) else {
            self.cancel();
            return None;
        };
        if items.len() != request.ids.len() {
            return self.history_changed();
        }
        self.view = ViewState::Ready { items, selected };
        None
    }

    fn apply_validation(
        &mut self,
        version: SafetyVersion,
        result: Result<HashSet<String>, String>,
    ) -> Option<Request> {
        if self.safety != Safety::Checking(version) {
            return None;
        }
        let existing = match result {
            Ok(existing) => existing,
            Err(message) => {
                self.safety = Safety::Failed;
                self.view = ViewState::Failed(message);
                return None;
            }
        };
        self.session.as_mut()?.reconcile(&existing);
        if self.selected_id().is_none() {
            self.cancel();
            return None;
        }
        self.safety = Safety::Ready;
        self.request_views()
    }

    pub fn cancel(&mut self) {
        self.session = None;
        self.safety = Safety::Ready;
        self.view = ViewState::Empty;
    }

    pub fn commit(&mut self) -> Option<String> {
        let id = self.session.as_mut()?.commit();
        self.cancel();
        id
    }
}
