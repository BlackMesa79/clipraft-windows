//! 运行时快照维护；只暂停采集，不改写用户暂停偏好。
use super::{State, CAPTURE_GATE, LAST_SEQUENCE};
use crate::{
    archive::control::Control,
    storage::{SnapshotFiles, Storage},
};
use anyhow::{anyhow, bail, Context, Result};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, MutexGuard, TryLockError,
    },
    thread,
    time::Duration,
};

#[cfg(test)]
pub(super) static TEST_SERIAL: Mutex<()> = Mutex::new(());

static ACTIVE: AtomicBool = AtomicBool::new(false);
const LOCK_RETRY: Duration = Duration::from_millis(20);

pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// 目标必须尚不存在。失败副本保留供上层任务按所有权清理，不作为有效快照。
/// 调用方随后应释放本阶段任务，再用独立导出 Control 处理密码归档。
pub fn create(
    app: &tauri::AppHandle,
    target: &Path,
    master: &[u8; 32],
    control: &Control,
) -> Result<()> {
    use tauri::Manager;
    let state = app
        .try_state::<std::sync::Arc<State>>()
        .context("运行时尚未初始化")?;
    create_with_state(&state, target, master, control)
}
fn create_with_state(
    state: &State,
    target: &Path,
    master: &[u8; 32],
    control: &Control,
) -> Result<()> {
    control.start()?;
    let result = create_owned(state, &mut SnapshotFiles::new(target), master, control)
        .and_then(|()| control.snapshot_finished());
    if result.is_err() {
        control.fail()?;
    }
    result
}
pub(super) fn create_owned(
    state: &State,
    files: &mut SnapshotFiles,
    master: &[u8; 32],
    control: &Control,
) -> Result<()> {
    let target = files.root().to_path_buf();
    let parent = target.parent().context("快照目标缺少父目录")?;
    if !target.is_absolute() || target.try_exists()? {
        bail!("快照目标必须是不存在的绝对目录");
    }
    let sequence: fn() -> u32 = if state.smoke { || 0 } else { current_sequence };
    let _pause = CapturePause::begin(control, sequence)?;
    let _context = lock_checked(&state.capture_context, control)?;
    let mut storage = lock_checked(&state.storage, control)?;
    let bytes = storage.snapshot_bytes(|| control.check())?;
    crate::data_directory::capacity(parent, bytes)?;
    control.snapshotting()?;
    storage.create_snapshot_owned(files, master, || control.check())?;
    drop(storage);
    drop(_context);
    // 白名单只修改独立副本，释放源存储锁后执行。
    control.check()?;
    Storage::prepare_portable_snapshot(&target, master)?;
    control.check()?;
    Ok(())
}
fn lock_checked<'a, T>(mutex: &'a Mutex<T>, control: &Control) -> Result<MutexGuard<'a, T>> {
    loop {
        control.check()?;
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => bail!("快照维护锁不可用"),
            Err(TryLockError::WouldBlock) => thread::sleep(LOCK_RETRY),
        }
    }
}
fn current_sequence() -> u32 {
    unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}
struct CapturePause {
    sequence: fn() -> u32,
}
impl CapturePause {
    fn begin(control: &Control, sequence: fn() -> u32) -> Result<Self> {
        let _gate = lock_checked(&CAPTURE_GATE, control)?;
        if ACTIVE.swap(true, Ordering::AcqRel) {
            bail!("已有快照维护任务正在运行");
        }
        Ok(Self { sequence })
    }
}
impl Drop for CapturePause {
    fn drop(&mut self) {
        // 先丢弃维护期间序列，再放行采集，用户暂停位完全由用户操作管理。
        let gate = CAPTURE_GATE
            .lock()
            .map_err(|_| anyhow!("快照结束时采集门不可用"));
        match gate {
            Ok(_gate) => {
                LAST_SEQUENCE.store((self.sequence)(), Ordering::Release);
                ACTIVE.store(false, Ordering::Release);
            }
            Err(error) => {
                ACTIVE.store(false, Ordering::Release);
                crate::diag::write(error.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests;
