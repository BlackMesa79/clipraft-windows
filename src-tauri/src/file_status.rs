//! 仅在后台访问文件系统；列表读取只查询有界缓存并尝试排队。
use crate::model::ItemView;
use std::{
    collections::HashMap,
    sync::{
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

struct Entry {
    checked: Instant,
    status: &'static str,
}

pub struct Cache {
    entries: Arc<Mutex<HashMap<Vec<String>, Entry>>>,
    pending: Arc<Mutex<std::collections::HashSet<Vec<String>>>>,
    queue: SyncSender<Vec<String>>,
}
pub fn check(paths: &[String], mut exists: impl FnMut(&str) -> Option<bool>) -> &'static str {
    let mut unknown = paths.is_empty();
    for path in paths {
        match exists(path) {
            Some(false) => return "missing",
            None => unknown = true,
            _ => {}
        }
    }
    if unknown {
        "unknown"
    } else {
        "available"
    }
}
impl Cache {
    pub fn new(changed: impl Fn() + Send + 'static) -> Self {
        let entries = Arc::new(Mutex::new(HashMap::<Vec<String>, Entry>::new()));
        let pending = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let (queue, receiver) = mpsc::sync_channel::<Vec<String>>(128);
        let cache = entries.clone();
        let waiting = pending.clone();
        std::thread::Builder::new()
            .name("file-status".into())
            .spawn(move || {
                while let Ok(paths) = receiver.recv() {
                    let status = check(&paths, |p| std::path::Path::new(p).try_exists().ok());
                    let mut map = cache.lock().unwrap();
                    if map.len() >= 4096 {
                        map.clear();
                    }
                    map.insert(
                        paths.clone(),
                        Entry {
                            checked: Instant::now(),
                            status,
                        },
                    );
                    drop(map);
                    waiting.lock().unwrap().remove(&paths);
                    changed();
                }
            })
            .expect("文件检查线程启动失败");
        Self {
            entries,
            pending,
            queue,
        }
    }
    pub fn annotate(&self, items: &mut [ItemView]) {
        let cache = self.entries.lock().unwrap();
        for item in items
            .iter_mut()
            .filter(|i| i.kind == "files" && !i.sensitive)
        {
            let entry = cache.get(&item.files);
            item.file_status = entry.map(|e| e.status.to_string());
            if entry.is_none_or(|e| e.checked.elapsed() >= Duration::from_secs(30)) {
                let mut pending = self.pending.lock().unwrap();
                if pending.insert(item.files.clone())
                    && self.queue.try_send(item.files.clone()).is_err()
                {
                    pending.remove(&item.files);
                }
            }
        }
    }
}

#[cfg(test)]
mod t8_tests {
    use super::*;

    fn item(paths: &[&str]) -> ItemView {
        ItemView {
            kind: "files".into(),
            files: paths.iter().map(|p| p.to_string()).collect(),
            ..Default::default()
        }
    }
    // AC-T8-3：无后台线程的受控队列，使过期、去重和满队列测试不依赖调度。
    fn controlled(capacity: usize) -> (Cache, mpsc::Receiver<Vec<String>>) {
        let (queue, receiver) = mpsc::sync_channel(capacity);
        (
            Cache {
                entries: Arc::new(Mutex::new(HashMap::new())),
                pending: Arc::new(Mutex::new(Default::default())),
                queue,
            },
            receiver,
        )
    }
    #[test]
    fn t8_status_missing_overrides_unknown_and_short_circuits() {
        let mut visited = Vec::new();
        assert_eq!(
            check(&["denied".into(), "gone".into(), "unused".into()], |p| {
                visited.push(p.to_owned());
                if p == "gone" {
                    Some(false)
                } else {
                    None
                }
            }),
            "missing"
        );
        assert_eq!(visited, ["denied", "gone"]);
    }
    #[test]
    fn t8_status_empty_unknown_and_all_available() {
        assert_eq!(check(&[], |_| panic!("空集合不应访问文件系统")), "unknown");
        assert_eq!(
            check(&["a".into(), "b".into()], |p| if p == "a" {
                Some(true)
            } else {
                None
            }),
            "unknown"
        );
        assert_eq!(
            check(&["a".into(), "b".into()], |_| Some(true)),
            "available"
        );
    }
    #[test]
    fn t8_cold_cache_deduplicates_same_paths_and_preserves_unicode() {
        let (cache, receiver) = controlled(4);
        let mut items = [
            item(&[r"C:\测试 文件\a.txt"]),
            item(&[r"C:\测试 文件\a.txt"]),
        ];
        cache.annotate(&mut items);
        assert!(items.iter().all(|i| i.file_status.is_none()));
        assert_eq!(receiver.try_recv().unwrap(), items[0].files);
        assert!(receiver.try_recv().is_err());
        assert_eq!(cache.pending.lock().unwrap().len(), 1);
    }
    #[test]
    fn t8_fresh_cache_is_returned_without_requeue() {
        let (cache, receiver) = controlled(4);
        let mut items = [item(&["a"])];
        cache.entries.lock().unwrap().insert(
            items[0].files.clone(),
            Entry {
                checked: Instant::now(),
                status: "available",
            },
        );
        cache.annotate(&mut items);
        assert_eq!(items[0].file_status.as_deref(), Some("available"));
        assert!(receiver.try_recv().is_err());
    }
    #[test]
    fn t8_expired_cache_requeues_once_and_keeps_last_status() {
        let (cache, receiver) = controlled(4);
        let mut items = [item(&["a"])];
        cache.entries.lock().unwrap().insert(
            items[0].files.clone(),
            Entry {
                checked: Instant::now() - Duration::from_secs(31),
                status: "missing",
            },
        );
        cache.annotate(&mut items);
        cache.annotate(&mut items);
        assert_eq!(items[0].file_status.as_deref(), Some("missing"));
        assert_eq!(receiver.try_recv().unwrap(), items[0].files);
        assert!(receiver.try_recv().is_err());
    }
    #[test]
    fn t8_full_queue_does_not_leave_unretryable_pending_entry() {
        let (cache, receiver) = controlled(1);
        let mut items = [item(&["a"]), item(&["b"])];
        cache.annotate(&mut items);
        assert!(!cache.pending.lock().unwrap().contains(&items[1].files));
        assert_eq!(receiver.try_recv().unwrap(), items[0].files);
        cache.annotate(&mut items[1..]);
        assert_eq!(receiver.try_recv().unwrap(), items[1].files);
    }
    #[test]
    fn t8_sensitive_and_nonfile_items_never_schedule_checks() {
        let (cache, receiver) = controlled(4);
        let mut secret = item(&["secret"]);
        secret.sensitive = true;
        let mut text = item(&["not-a-file"]);
        text.kind = "text".into();
        cache.annotate(&mut [secret, text]);
        assert!(receiver.try_recv().is_err());
        assert!(cache.pending.lock().unwrap().is_empty());
    }
    #[test]
    fn t8_worker_publishes_result_and_invalidates_at_capacity() {
        let (tx, rx) = mpsc::channel();
        let cache = Cache::new(move || {
            let _ = tx.send(());
        });
        {
            let mut entries = cache.entries.lock().unwrap();
            for i in 0..4096 {
                entries.insert(
                    vec![format!("synthetic-{i}")],
                    Entry {
                        checked: Instant::now(),
                        status: "unknown",
                    },
                );
            }
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("Cargo.toml")
            .to_string_lossy()
            .into_owned();
        let mut items = [item(&[&path])];
        cache.annotate(&mut items);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("后台完成后必须通知前端刷新");
        cache.annotate(&mut items);
        assert_eq!(items[0].file_status.as_deref(), Some("available"));
        assert_eq!(cache.entries.lock().unwrap().len(), 1);
        assert!(cache.pending.lock().unwrap().is_empty());
    }
}
