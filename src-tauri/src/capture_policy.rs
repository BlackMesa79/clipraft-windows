//! 单轮采集策略；序列采样、读取、等待均可注入，不依赖系统剪贴板。
use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    Busy,
    Limit,
    Invalid,
}
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "剪贴板被其他程序长时间占用，本次复制未记录",
            Self::Limit => "剪贴板数据超过 128 MiB 限制，本次复制未记录",
            Self::Invalid => "剪贴板格式无效或数据不可读，本次复制未记录",
        })
    }
}
impl std::error::Error for CaptureError {}

#[derive(Clone, Copy)]
pub struct Snapshot {
    pub sequence: u32,
    pub self_write: bool,
}
const BACKOFF_MS: [u64; 5] = [50, 100, 200, 400, 800];

pub fn run<T>(
    last: &AtomicU32,
    mut observe: impl FnMut() -> Snapshot,
    mut read: impl FnMut(u32) -> Result<Option<T>, CaptureError>,
    mut wait: impl FnMut(Duration),
    mut log: impl FnMut(String),
) -> Result<Option<T>, CaptureError> {
    let first = observe();
    let sequence = first.sequence;
    let mut current = first;
    let mut retries = 0;
    let mut busy = false;
    let mut finished_sequence = sequence;
    let (result, reason) = loop {
        // 自写回优先于序列变化判断，无需打开被目标应用占用的剪贴板。
        if current.self_write {
            finished_sequence = current.sequence;
            break (Ok(None), "self");
        }
        if current.sequence != sequence {
            break (Ok(None), "superseded");
        }
        if sequence == last.load(Ordering::Acquire) {
            break (Ok(None), "duplicate");
        }
        let attempt = read(sequence);
        busy |= matches!(attempt, Err(CaptureError::Busy));
        // 打开/读取期间也可能换序列；不向用户报告过期结果。
        current = observe();
        if current.self_write || current.sequence != sequence {
            continue;
        }
        if !matches!(attempt, Err(CaptureError::Busy)) {
            break (attempt, "read");
        }
        // 等待前后均检查，耗尽时也须确认不是新序列或自写回。
        let Some(delay) = BACKOFF_MS.get(retries) else {
            break (Err(CaptureError::Busy), "exhausted");
        };
        wait(Duration::from_millis(*delay));
        retries += 1;
        current = observe();
    };
    // 失败也结束本轮；外部序列已变化时只确认旧序列，让下一轮读取新内容。
    last.store(finished_sequence, Ordering::Release);
    let status = match &result {
        Ok(Some(_)) => "ok",
        Ok(None) => "skipped",
        Err(CaptureError::Busy) => "busy",
        Err(CaptureError::Limit) => "limit",
        Err(CaptureError::Invalid) => "invalid",
    };
    log(format!("capture sequence={sequence} current={} self_write={} busy={busy} retries={retries} result={status} reason={reason}", current.sequence, current.self_write));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn external(sequence: u32) -> Snapshot {
        Snapshot {
            sequence,
            self_write: false,
        }
    }
    fn no_wait(_: Duration) {
        panic!("不应退避");
    }

    #[test]
    fn self_write_skips_before_open_and_acknowledges_sequence() {
        let last = AtomicU32::new(0);
        let mut logs = Vec::new();
        let result = run::<()>(
            &last,
            || Snapshot {
                sequence: 42,
                self_write: true,
            },
            |_| panic!("自写回不应打开剪贴板"),
            no_wait,
            |line| logs.push(line),
        );
        assert_eq!(result, Ok(None));
        assert_eq!(last.load(Ordering::Acquire), 42);
        assert!(logs[0].contains("self_write=true busy=false retries=0 result=skipped"));
    }
    #[test]
    fn busy_then_success_uses_backoff() {
        let last = AtomicU32::new(0);
        let mut attempts = 0;
        let mut waits = Vec::new();
        let mut logs = Vec::new();
        let result = run(
            &last,
            || external(42),
            |_| {
                attempts += 1;
                if attempts < 3 {
                    Err(CaptureError::Busy)
                } else {
                    Ok(Some(7))
                }
            },
            |duration| waits.push(duration.as_millis()),
            |line| logs.push(line),
        );
        assert_eq!(result, Ok(Some(7)));
        assert_eq!(waits, [50, 100]);
        assert_eq!(last.load(Ordering::Acquire), 42);
        assert!(logs[0].contains("busy=true retries=2 result=ok"));
    }
    #[test]
    fn sequence_change_during_wait_leaves_new_sequence_for_next_round() {
        let last = AtomicU32::new(0);
        let sequence = Cell::new(42);
        let mut attempts = 0;
        assert_eq!(
            run::<()>(
                &last,
                || external(sequence.get()),
                |_| {
                    attempts += 1;
                    Err(CaptureError::Busy)
                },
                |_| sequence.set(43),
                |_| {}
            ),
            Ok(None)
        );
        assert_eq!(attempts, 1);
        assert_eq!(last.load(Ordering::Acquire), 42);
        assert_eq!(
            run(&last, || external(43), |_| Ok(Some(7)), no_wait, |_| {}),
            Ok(Some(7))
        );
        assert_eq!(last.load(Ordering::Acquire), 43);
    }
    #[test]
    fn sequence_change_before_wait_does_not_sleep() {
        let last = AtomicU32::new(0);
        let sequence = Cell::new(42);
        assert_eq!(
            run::<()>(
                &last,
                || external(sequence.get()),
                |_| {
                    sequence.set(43);
                    Err(CaptureError::Busy)
                },
                no_wait,
                |_| {}
            ),
            Ok(None)
        );
        assert_eq!(last.load(Ordering::Acquire), 42);
    }
    #[test]
    fn self_write_during_wait_is_acknowledged_without_reopening() {
        let last = AtomicU32::new(0);
        let changed = Cell::new(false);
        let mut attempts = 0;
        assert_eq!(
            run::<()>(
                &last,
                || Snapshot {
                    sequence: if changed.get() { 43 } else { 42 },
                    self_write: changed.get(),
                },
                |_| {
                    attempts += 1;
                    Err(CaptureError::Busy)
                },
                |_| changed.set(true),
                |_| {}
            ),
            Ok(None)
        );
        assert_eq!(attempts, 1);
        assert_eq!(last.load(Ordering::Acquire), 43);
    }
    #[test]
    fn exhausted_busy_reports_once_and_same_sequence_is_not_reopened() {
        let last = AtomicU32::new(0);
        let mut attempts = 0;
        let mut reports = 0;
        let mut waits = Vec::new();
        let mut logs = Vec::new();
        for _ in 0..3 {
            if run::<()>(
                &last,
                || external(42),
                |_| {
                    attempts += 1;
                    Err(CaptureError::Busy)
                },
                |duration| waits.push(duration.as_millis()),
                |line| logs.push(line),
            )
            .is_err()
            {
                reports += 1;
            }
        }
        assert_eq!(reports, 1);
        assert_eq!(attempts, 6);
        assert_eq!(waits, [50, 100, 200, 400, 800]);
        assert_eq!(last.load(Ordering::Acquire), 42);
        assert!(logs[0].contains("busy=true retries=5 result=busy"));
        assert!(logs[1].contains("result=skipped reason=duplicate"));
    }
    #[test]
    fn last_busy_attempt_checks_for_new_self_sequence_before_reporting() {
        let last = AtomicU32::new(0);
        let changed = Cell::new(false);
        let mut attempts = 0;
        assert_eq!(
            run::<()>(
                &last,
                || Snapshot {
                    sequence: if changed.get() { 43 } else { 42 },
                    self_write: changed.get(),
                },
                |_| {
                    attempts += 1;
                    if attempts == 6 {
                        changed.set(true);
                    }
                    Err(CaptureError::Busy)
                },
                |_| {},
                |_| {}
            ),
            Ok(None)
        );
        assert_eq!(attempts, 6);
        assert_eq!(last.load(Ordering::Acquire), 43);
    }
    #[test]
    fn success_and_skip_both_deduplicate() {
        for value in [Some(7), None] {
            let last = AtomicU32::new(0);
            assert_eq!(
                run(&last, || external(42), |_| Ok(value), no_wait, |_| {}),
                Ok(value)
            );
            assert_eq!(
                run::<i32>(
                    &last,
                    || external(42),
                    |_| panic!("同序列不应再读取"),
                    no_wait,
                    |_| {}
                ),
                Ok(None)
            );
        }
    }
    #[test]
    fn invalid_and_limit_do_not_retry_and_each_reports_once() {
        for (error, status) in [
            (CaptureError::Invalid, "invalid"),
            (CaptureError::Limit, "limit"),
        ] {
            let last = AtomicU32::new(0);
            let mut logs = Vec::new();
            assert_eq!(
                run::<()>(
                    &last,
                    || external(42),
                    |_| Err(error),
                    no_wait,
                    |line| logs.push(line)
                ),
                Err(error)
            );
            assert!(logs[0].contains(&format!("busy=false retries=0 result={status}")));
            assert_eq!(
                run::<()>(
                    &last,
                    || external(42),
                    |_| panic!("失败序列不应再读取"),
                    no_wait,
                    |_| {}
                ),
                Ok(None)
            );
        }
        assert_ne!(
            CaptureError::Busy.to_string(),
            CaptureError::Invalid.to_string()
        );
        assert_ne!(
            CaptureError::Limit.to_string(),
            CaptureError::Invalid.to_string()
        );
    }
    #[test]
    fn wrapped_sequence_is_a_new_round() {
        let last = AtomicU32::new(u32::MAX);
        assert_eq!(
            run(&last, || external(1), |_| Ok(Some(7)), no_wait, |_| {}),
            Ok(Some(7))
        );
        assert_eq!(last.load(Ordering::Acquire), 1);
    }
    #[test]
    fn read_race_discards_stale_error_and_keeps_new_sequence_pending() {
        let last = AtomicU32::new(0);
        let sequence = Cell::new(42);
        assert_eq!(
            run::<()>(
                &last,
                || external(sequence.get()),
                |_| {
                    sequence.set(43);
                    Err(CaptureError::Invalid)
                },
                no_wait,
                |_| {}
            ),
            Ok(None)
        );
        assert_eq!(last.load(Ordering::Acquire), 42);
    }
    #[test]
    fn owner_marker_found_while_reading_is_logged_as_self_write() {
        let last = AtomicU32::new(0);
        let owned = Cell::new(false);
        let mut logs = Vec::new();
        assert_eq!(
            run::<()>(
                &last,
                || Snapshot {
                    sequence: 42,
                    self_write: owned.get()
                },
                |_| {
                    owned.set(true);
                    Ok(None)
                },
                no_wait,
                |line| logs.push(line)
            ),
            Ok(None)
        );
        assert_eq!(last.load(Ordering::Acquire), 42);
        assert!(logs[0].contains("self_write=true busy=false retries=0 result=skipped reason=self"));
    }
    #[test]
    fn capture_log_contains_metadata_without_clipboard_content() {
        let last = AtomicU32::new(0);
        let mut logs = Vec::new();
        let content = "合成内容，不得写入诊断日志";
        assert_eq!(
            run(
                &last,
                || external(42),
                |_| Ok(Some(content)),
                no_wait,
                |line| logs.push(line)
            ),
            Ok(Some(content))
        );
        assert_eq!(logs.len(), 1);
        assert!(!logs[0].contains(content));
        assert!(logs[0]
            .contains("sequence=42 current=42 self_write=false busy=false retries=0 result=ok"));
    }
}
