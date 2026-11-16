//! 单个归档任务的状态；发布开始后不再接受取消。
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Pending,
    Preparing,
    Snapshotting,
    Writing,
    Verifying,
    Publishing,
    Complete,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Progress {
    pub phase: Phase,
    pub completed_bytes: u64,
    pub total_bytes: u64,
}

pub struct Control(Mutex<Progress>);
impl Default for Control {
    fn default() -> Self {
        Self(Mutex::new(Progress {
            phase: Phase::Pending,
            completed_bytes: 0,
            total_bytes: 0,
        }))
    }
}
impl Control {
    pub fn progress(&self) -> Result<Progress> {
        Ok(*self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态锁损坏"))?)
    }
    pub fn cancel(&self) -> Result<bool> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态锁损坏"))?;
        match state.phase {
            Phase::Pending
            | Phase::Preparing
            | Phase::Snapshotting
            | Phase::Writing
            | Phase::Verifying => {
                state.phase = Phase::Cancelled;
                Ok(true)
            }
            Phase::Publishing | Phase::Complete | Phase::Cancelled | Phase::Failed => Ok(false),
        }
    }
    pub(crate) fn check(&self) -> Result<()> {
        if self.progress()?.phase == Phase::Cancelled {
            bail!("归档任务已取消");
        }
        Ok(())
    }
    pub(crate) fn start(&self) -> Result<()> {
        self.transition(Phase::Pending, Phase::Preparing)
    }
    pub(crate) fn snapshotting(&self) -> Result<()> {
        self.transition(Phase::Preparing, Phase::Snapshotting)
    }
    pub(crate) fn snapshot_prepared(&self) -> Result<()> {
        self.transition(Phase::Snapshotting, Phase::Preparing)
    }
    pub(crate) fn snapshot_finished(&self) -> Result<()> {
        self.transition(Phase::Snapshotting, Phase::Complete)
    }
    pub(super) fn writing(&self, total: u64) -> Result<()> {
        self.transition(Phase::Preparing, Phase::Writing)?;
        self.0
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态锁损坏"))?
            .total_bytes = total;
        self.check()
    }
    pub(super) fn advance(&self, bytes: u64) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态锁损坏"))?;
        if state.phase != Phase::Writing {
            bail!("归档任务已取消或不在写入阶段");
        }
        state.completed_bytes = state
            .completed_bytes
            .checked_add(bytes)
            .context("归档进度溢出")?;
        Ok(())
    }
    pub(super) fn verifying(&self) -> Result<()> {
        self.transition(Phase::Writing, Phase::Verifying)
    }
    pub(super) fn publish(&self) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态锁损坏"))?;
        if !matches!(state.phase, Phase::Writing | Phase::Verifying) {
            bail!("归档任务已取消或状态不允许发布");
        }
        state.phase = Phase::Publishing;
        Ok(())
    }
    pub(super) fn finish(&self) -> Result<()> {
        self.transition(Phase::Publishing, Phase::Complete)
    }
    pub(crate) fn fail(&self) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态锁损坏"))?;
        if state.phase != Phase::Cancelled {
            state.phase = Phase::Failed;
        }
        Ok(())
    }
    fn transition(&self, expected: Phase, next: Phase) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("归档任务状态锁损坏"))?;
        if state.phase != expected {
            bail!("归档任务已取消或状态不允许此操作");
        }
        state.phase = next;
        Ok(())
    }
}
