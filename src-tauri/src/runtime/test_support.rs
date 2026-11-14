use super::*;

pub(super) struct Fixture(std::path::PathBuf);

impl Fixture {
    pub(super) fn new(label: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../.test-data")
                .join(format!("runtime-{label}-{}-{stamp}", std::process::id())),
        )
    }
    pub(super) fn state(&self) -> State {
        state(&self.0)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("隔离运行时测试目录清理失败 {}：{error}", self.0.display());
        }
    }
}

pub(super) fn captured() -> clipboard::Captured {
    clipboard::Captured {
        source: Default::default(),
        clip: crate::model::Clip {
            formats: vec![crate::model::FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: "synthetic capture"
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            }],
        },
        sequence: 1,
        context: crate::owner_flush::CaptureContext {
            formats: Some([13].into_iter().collect()),
            owner: crate::clipboard_owner::ClipboardOwner::absent(),
            fingerprint: None,
            bitmap_offered: false,
            record_id: None,
        },
    }
}

pub(super) fn state(root: &std::path::Path) -> State {
    let old = Settings::default();
    let mut storage = Storage::open(root, &[72; 32]).unwrap();
    storage.save_settings(&old).unwrap();
    State {
        files: crate::file_status::Cache::new(|| {}).unwrap(),
        settings_requested: AtomicBool::new(false),
        items: Mutex::new(Vec::new()),
        storage: Mutex::new(storage),
        capture_context: Mutex::new(None),
        retention_policy: Mutex::new(old.retention_policy()),
        hide_on_blur: AtomicBool::new(old.hide_on_blur),
        settings: Mutex::new(old),
        startup_error: Mutex::new(None),
        jobs: mpsc::sync_channel(1).0,
        control: mpsc::channel().0,
        target: AtomicIsize::new(0),
        main: AtomicIsize::new(0),
        quick: AtomicIsize::new(0),
        ready: AtomicBool::new(false),
        panel: Mutex::new(Panel::default()),
        error: Mutex::new(None),
        smoke: true,
        main_ready: AtomicBool::new(false),
        quick_ready: AtomicBool::new(false),
        thumbnail_ready: AtomicBool::new(false),
        preview_ready: AtomicBool::new(false),
    }
}

pub(super) fn poison<T>(mutex: &Mutex<T>) {
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = mutex.lock().unwrap();
        panic!("模拟运行时锁中毒");
    }))
    .is_err());
}
