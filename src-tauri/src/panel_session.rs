//! U3 纯会话：只保存 ID 和请求版本，不访问存储、窗口或剪贴板。

use crate::hook::{initial_selection, move_selection};
use std::{collections::HashSet, sync::Arc};

pub const MAX_NEIGHBORHOOD_ITEMS: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Next,
    Previous,
}

/// session 必须由调用方为每次打开分配唯一编号，不能复用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestVersion {
    session: u64,
    revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborhoodRequest {
    pub version: RequestVersion,
    pub ids: Vec<String>,
    pub selected: usize,
    pub position: usize,
    pub total: usize,
}

pub struct Session {
    order: Arc<[String]>,
    selected: Option<usize>,
    version: RequestVersion,
}

impl Session {
    /// order 来自已完成的唯一 ID 索引；空历史不建立活动会话。
    pub fn open(session: u64, order: Arc<[String]>) -> Option<Self> {
        if order.is_empty() {
            return None;
        }
        Some(Self {
            selected: Some(initial_selection(order.len())),
            order,
            version: RequestVersion {
                session,
                revision: 0,
            },
        })
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected.map(|index| self.order[index].as_str())
    }

    /// 位置从零开始；邻域是否已加载不参与选择和提交。
    pub fn position(&self) -> Option<usize> {
        self.selected
    }

    pub fn total(&self) -> usize {
        self.order.len()
    }

    pub fn id_snapshot(&self) -> Arc<[String]> {
        self.order.clone()
    }

    pub fn navigate(&mut self, direction: Direction) {
        let Some(selected) = self.selected else {
            return;
        };
        let forward = match direction {
            Direction::Next => true,
            Direction::Previous => false,
        };
        self.selected = Some(move_selection(selected, self.order.len(), forward));
        self.invalidate();
    }

    /// 敏感状态变化先使旧响应失效，调用方随后清空旧安全视图并重新取数。
    pub fn invalidate(&mut self) {
        self.version.revision += 1;
    }

    pub fn accepts(&self, version: RequestVersion) -> bool {
        self.selected.is_some() && self.version == version
    }

    pub fn request(&mut self, visible_count: usize) -> Option<NeighborhoodRequest> {
        let position = self.selected?;
        self.invalidate();
        let count = visible_count
            .clamp(1, MAX_NEIGHBORHOOD_ITEMS)
            .min(self.total());
        let selected = count / 2;
        let start = (position + self.total() - selected) % self.total();
        let ids = (0..count)
            .map(|offset| self.order[(start + offset) % self.total()].clone())
            .collect();
        Some(NeighborhoodRequest {
            version: self.version,
            ids,
            selected,
            position,
            total: self.total(),
        })
    }

    /// existing 必须是整个会话的完整有效性结果，不能传入首页或某个邻域。
    pub fn reconcile(&mut self, existing: &HashSet<String>) {
        let Some(selected_id) = self.selected_id().map(str::to_owned) else {
            return;
        };
        self.invalidate();
        if !existing.contains(&selected_id) {
            self.cancel();
            return;
        }
        self.order = self
            .order
            .iter()
            .filter(|id| existing.contains(*id))
            .cloned()
            .collect();
        self.selected = self.order.iter().position(|id| *id == selected_id);
    }

    pub fn cancel(&mut self) {
        self.selected = None;
        self.invalidate();
    }

    /// 提交只消费当前选中 ID；关闭后重复提交不会再次产生粘贴任务。
    pub fn commit(&mut self) -> Option<String> {
        let id = self.selected_id()?.to_owned();
        self.cancel();
        Some(id)
    }
}
