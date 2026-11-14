//! 仅在后台查询存储；读取快照只短暂持有独立内存锁。
use crate::storage::Storage;
use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex};

pub(super) struct Snapshot {
    pub version: u64,
    pub ids: Arc<[String]>,
}

#[derive(Default)]
enum Status {
    #[default]
    Pending,
    Ready,
    Failed(String),
}

#[derive(Default)]
struct Index {
    complete: Option<Arc<Snapshot>>,
    status: Status,
}

#[derive(Default)]
pub(super) struct Cache(Mutex<Index>);

impl Cache {
    pub(super) fn fail(&self, message: String) -> Result<()> {
        self.0.lock().map_err(|_| anyhow!("历史索引不可用"))?.status = Status::Failed(message);
        Ok(())
    }
    pub(super) fn snapshot(&self) -> Result<Arc<Snapshot>> {
        let index = self.0.lock().map_err(|_| anyhow!("历史索引不可用"))?;
        match &index.status {
            Status::Pending => Err(anyhow!("历史索引正在准备，请稍后重试")),
            Status::Failed(message) => Err(anyhow!("历史索引刷新失败：{message}")),
            Status::Ready => index
                .complete
                .clone()
                .ok_or_else(|| anyhow!("历史索引缺少完整快照")),
        }
    }

    /// 持有存储锁期间完成查询和发布，避免并发刷新逆序覆盖。
    pub(super) fn refresh(&self, storage: &Storage) -> Result<()> {
        self.replace(storage.panel_id_order())
    }

    fn replace(&self, result: Result<Vec<String>>) -> Result<()> {
        let ids = match result {
            Ok(ids) => ids.into(),
            Err(error) => {
                let mut index = self.0.lock().map_err(|_| anyhow!("历史索引不可用"))?;
                index.status = Status::Failed(error.to_string());
                return Err(error);
            }
        };
        self.publish(ids)
    }

    fn publish(&self, ids: Arc<[String]>) -> Result<()> {
        let mut index = self.0.lock().map_err(|_| anyhow!("历史索引不可用"))?;
        let version = index.complete.as_ref().map_or(1, |old| old.version + 1);
        let previous = index.complete.replace(Arc::new(Snapshot { version, ids }));
        index.status = Status::Ready;
        drop(index);
        drop(previous);
        Ok(())
    }
}

pub(super) fn initialize(state: &super::State) -> Result<()> {
    let storage = match state.storage.lock() {
        Ok(storage) => storage,
        Err(_) => return state.history_index.replace(Err(anyhow!("存储不可用"))),
    };
    state.history_index.refresh(&storage)
}

#[cfg(test)]
mod tests;
