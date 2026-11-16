//! 单任务后台调度；任务位直到工作与清理结束才释放。
use super::control::{Control, Phase, Progress};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Export,
    Restore,
}
#[derive(Clone, Debug, Serialize)]
pub struct Success {
    pub restored_root: Option<PathBuf>,
    pub replace_current: bool,
    pub cleanup_warning: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    Succeeded(Success),
    Failed { message: String },
    Cancelled { message: String },
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub id: u64,
    pub kind: Kind,
    pub running: bool,
    pub progress: Progress,
    pub outcome: Option<Outcome>,
}
struct Task {
    id: u64,
    kind: Kind,
    control: Arc<Control>,
    running: bool,
    outcome: Option<Outcome>,
}
#[derive(Default)]
struct Slot {
    next_id: u64,
    restarting: bool,
    task: Option<Task>,
}
#[derive(Default)]
pub struct Manager {
    slot: Mutex<Slot>,
}
impl Manager {
    pub fn start(
        self: &Arc<Self>,
        kind: Kind,
        work: impl FnOnce(&Control) -> Result<Success> + Send + 'static,
    ) -> Result<u64> {
        let (id, control) = self.reserve(kind)?;
        let manager = Arc::clone(self);
        let worker = thread::Builder::new()
            .name("clipraft-archive-task".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&control)))
                        .unwrap_or_else(|_| {
                            Err(anyhow::anyhow!(
                                "归档后台任务异常终止，暂存内容已保留供检查"
                            ))
                        });
                if let Err(error) = manager.finish(id, result) {
                    crate::diag::write(format!("archive task status update failed: {error:#}"));
                }
            });
        if let Err(error) = worker {
            self.finish(id, Err(error.into()))?;
            bail!("无法启动归档后台任务");
        }
        Ok(id)
    }
    fn reserve(&self, kind: Kind) -> Result<(u64, Arc<Control>)> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态不可用"))?;
        if slot.restarting {
            bail!("正在准备迁移重启，不能启动归档任务");
        }
        if slot.task.as_ref().is_some_and(|task| task.running) {
            bail!("已有归档任务正在执行或清理");
        }
        let id = slot.next_id.checked_add(1).context("归档任务编号已耗尽")?;
        slot.next_id = id;
        let control = Arc::new(Control::default());
        slot.task = Some(Task {
            id,
            kind,
            control: control.clone(),
            running: true,
            outcome: None,
        });
        Ok((id, control))
    }
    pub(crate) fn begin_restart(self: &Arc<Self>) -> Result<RestartGuard> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态不可用"))?;
        if slot.restarting || slot.task.as_ref().is_some_and(|task| task.running) {
            bail!("归档任务或迁移重启正在执行，请等待结束");
        }
        slot.restarting = true;
        Ok(RestartGuard {
            manager: self.clone(),
            committed: false,
        })
    }
    pub(crate) fn restored_for_restart(
        self: &Arc<Self>,
        id: u64,
    ) -> Result<(Success, RestartGuard)> {
        let restart = self.begin_restart()?;
        let slot = self
            .slot
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态不可用"))?;
        let task = slot
            .task
            .as_ref()
            .filter(|task| task.id == id && task.kind == Kind::Restore)
            .context("恢复任务已失效，请重新恢复归档")?;
        let success = match &task.outcome {
            Some(Outcome::Succeeded(success)) if success.restored_root.is_some() => success.clone(),
            _ => bail!("恢复任务尚未成功，不能替换当前数据"),
        };
        Ok((success, restart))
    }

    pub fn status(&self) -> Result<Option<Report>> {
        let slot = self
            .slot
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态不可用"))?;
        slot.task
            .as_ref()
            .map(|task| {
                Ok(Report {
                    id: task.id,
                    kind: task.kind,
                    running: task.running,
                    progress: task.control.progress()?,
                    outcome: task.outcome.clone(),
                })
            })
            .transpose()
    }
    pub fn cancel(&self, id: u64) -> Result<bool> {
        let slot = self
            .slot
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态不可用"))?;
        let task = slot.task.as_ref().context("没有可取消的归档任务")?;
        if task.id != id {
            bail!("归档任务编号已失效");
        }
        if !task.running {
            return Ok(false);
        }
        task.control.cancel()
    }
    fn finish(&self, id: u64, result: Result<Success>) -> Result<()> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态不可用"))?;
        let task = slot
            .task
            .as_mut()
            .filter(|task| task.id == id && task.running)
            .context("归档任务已失效")?;
        task.outcome = Some(outcome(&task.control, result)?);
        task.running = false;
        Ok(())
    }
}
pub(crate) struct RestartGuard {
    manager: Arc<Manager>,
    committed: bool,
}
impl RestartGuard {
    pub(crate) fn commit(mut self) {
        self.committed = true;
    }
}
impl Drop for RestartGuard {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        match self.manager.slot.lock() {
            Ok(mut slot) => slot.restarting = false,
            Err(_) => crate::diag::write("archive restart guard lock unavailable".into()),
        }
    }
}

fn outcome(control: &Control, result: Result<Success>) -> Result<Outcome> {
    let phase = control.progress()?.phase;
    match (phase, result) {
        (Phase::Complete, Ok(success)) => Ok(Outcome::Succeeded(success)),
        (Phase::Cancelled, Err(error)) => Ok(Outcome::Cancelled {
            message: format!("{error:#}"),
        }),
        (_, result) => {
            let message = result.err().map_or_else(
                || "归档任务未完成就返回成功".into(),
                |error| format!("{error:#}"),
            );
            control.fail()?;
            Ok(Outcome::Failed { message })
        }
    }
}
#[cfg(test)]
mod tests;
