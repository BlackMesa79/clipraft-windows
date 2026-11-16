use super::*;
use crate::{
    model::{Clip, FormatData},
    storage::Storage,
};
use config::Migration;
use std::panic::{catch_unwind, AssertUnwindSafe};

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    target: PathBuf,
    locations: Locations,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.test-data")
            .join(format!("s1-{}", config::nonce().unwrap()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let source = root.join("源数据 空格");
        let target = root.join("目标数据 空格");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        let locations = Locations::new(source.clone(), Some(root.join("profile"))).unwrap();
        fs::create_dir(root.join("profile")).unwrap();
        Self {
            root,
            source,
            target,
            locations,
        }
    }
    fn seed(&self) -> String {
        let key = crate::platform::master_key(&self.source).unwrap();
        let mut storage = Storage::open(&self.source, &key).unwrap();
        let clip = Clip {
            formats: vec![FormatData {
                name: "CF_UNICODETEXT".into(),
                bytes: "迁移测试文本\n"
                    .repeat(9000)
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            }],
        };
        storage.save(&clip).unwrap();
        let id = clip.hash();
        let group = storage.create_group("迁移分组").unwrap();
        storage
            .move_to_group(std::slice::from_ref(&id), Some(group.id))
            .unwrap();
        storage
            .set_favorite(std::slice::from_ref(&id), true)
            .unwrap();
        storage.set_pinned(std::slice::from_ref(&id), true).unwrap();
        let settings = crate::settings::Settings {
            compact: true,
            ..Default::default()
        };
        storage.save_settings(&settings).unwrap();
        drop(storage);
        fs::write(
            self.source.join("history.db.bak-v4"),
            b"encrypted backup fixture",
        )
        .unwrap();
        id
    }
    fn task(&self) -> Configuration {
        Configuration {
            version: 1,
            startup_capture: Default::default(),
            root: self.source.clone(),
            previous: None,
            cleanup: None,
            displaced: None,
            replacement: None,
            pending: Some(Migration {
                source: self.source.clone(),
                target: self.target.clone(),
                id: config::nonce().unwrap(),
                displaced: None,
            }),
        }
    }
    fn control(&self) -> DirectoryControl {
        DirectoryControl {
            locations: self.locations.clone(),
            root: self.source.clone(),
            plan: Mutex::new(None),
        }
    }
    fn migrate(&self, task: &Configuration) -> Result<(Configuration, File)> {
        migration::execute(&self.locations, task, &mut |_, _, _| {})
    }
    fn stage(&self, task: &Configuration) -> PathBuf {
        self.root.join(format!(
            ".clipraft-migration-{}",
            task.pending.as_ref().unwrap().id
        ))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn legacy_default_and_isolation_do_not_write_startup_config() {
    let f = Fixture::new();
    let isolated = Locations::new(f.source.clone(), None).unwrap();
    assert_eq!(isolated.read().unwrap().root, f.source);
    assert!(isolated
        .save(&Configuration::initial(f.source.clone()))
        .is_err());
    assert!(!f.locations.config.as_ref().unwrap().exists());
    let control = DirectoryControl {
        locations: isolated,
        root: f.source.clone(),
        plan: Mutex::new(None),
    };
    assert!(control.preview(&f.target).is_err());
}
#[test]
fn configuration_atomically_replaces_and_rejects_invalid_versions() {
    let f = Fixture::new();
    let first = Configuration::initial(f.source.clone());
    f.locations.save(&first).unwrap();
    let second = Configuration::initial(f.target.clone());
    f.locations.save(&second).unwrap();
    assert_eq!(f.locations.read().unwrap().root, f.target);
    fs::write(
        f.locations.config.as_ref().unwrap(),
        r#"{"version":99,"root":"D:/bad","pending":null}"#,
    )
    .unwrap();
    assert!(f.locations.read().is_err());
    assert_eq!(fs::read_dir(f.root.join("profile")).unwrap().count(), 2);
}
#[test]
fn configuration_rejects_corruption_and_malformed_migration() {
    let f = Fixture::new();
    fs::write(f.locations.config.as_ref().unwrap(), "{").unwrap();
    assert!(f.locations.read().is_err());
    let mut task = f.task();
    task.pending.as_mut().unwrap().id = "../outside".into();
    f.locations.save(&task).unwrap();
    assert!(f.locations.read().is_err());
}
#[test]
fn directory_lock_is_exclusive_and_released_on_drop() {
    let f = Fixture::new();
    let guard = files::lock(&f.source).unwrap();
    assert!(files::lock(&f.source).is_err());
    drop(guard);
    assert!(files::lock(&f.source).is_ok());
}
#[test]
fn same_parent_child_and_nonempty_targets_are_rejected() {
    let f = Fixture::new();
    let control = f.control();
    assert!(control.preview(&f.source).is_err());
    assert!(control.preview(&f.root).is_err());
    let child = f.source.join("child");
    fs::create_dir(&child).unwrap();
    assert!(control.preview(&child).is_err());
    fs::write(f.target.join("user-file"), "保留").unwrap();
    assert!(control.preview(&f.target).is_err());
    assert_eq!(
        fs::read_to_string(f.target.join("user-file")).unwrap(),
        "保留"
    );
}
#[test]
fn preflight_cancellation_has_no_persistent_migration_and_ticket_is_checked() {
    let f = Fixture::new();
    f.seed();
    let control = f.control();
    let plan = control.preview(&f.target).unwrap();
    assert!(control.schedule("wrong-ticket").is_err());
    assert!(f.locations.read().unwrap().pending.is_none());
    control.schedule(&plan.ticket).unwrap();
    assert!(f.locations.read().unwrap().pending.is_some());
    assert!(control.schedule(&plan.ticket).is_err());
}
#[test]
fn target_is_rechecked_at_confirmation() {
    let f = Fixture::new();
    f.seed();
    let control = f.control();
    let plan = control.preview(&f.target).unwrap();
    fs::write(f.target.join("new-file"), "其他程序写入").unwrap();
    assert!(control.schedule(&plan.ticket).is_err());
    assert!(f.locations.read().unwrap().pending.is_none());
}
#[test]
fn insufficient_space_is_rejected_without_large_allocations() {
    let f = Fixture::new();
    assert!(paths::capacity(&f.target, u64::MAX - 1).is_err());
}
#[test]
fn migration_preserves_ciphertext_key_settings_groups_and_ignores_runtime_files() {
    let f = Fixture::new();
    let id = f.seed();
    fs::create_dir(f.source.join("webview")).unwrap();
    fs::write(f.source.join("webview/cache"), "不迁移缓存").unwrap();
    fs::write(f.source.join("diag.log"), "不迁移日志").unwrap();
    let before = files::inventory(&f.source).unwrap();
    let (committed, guard) = f.migrate(&f.task()).unwrap();
    assert_eq!(committed.root, f.target);
    assert_eq!(committed.previous.as_ref(), Some(&f.source));
    assert!(committed.pending.is_none());
    assert_eq!(files::inventory(&f.target).unwrap(), before);
    assert_eq!(files::inventory(&f.source).unwrap(), before);
    assert!(!f.target.join("webview").exists());
    assert!(!f.target.join("diag.log").exists());
    let key = crate::platform::master_key(&f.target).unwrap();
    let storage = Storage::open(&f.target, &key).unwrap();
    assert!(storage.load_settings().unwrap().compact);
    let item = storage.preview_item(&id).unwrap();
    assert!(item.favorite && item.pinned && item.group_id.is_some());
    assert_eq!(f.locations.read().unwrap().root, f.target);
    drop(storage);
    drop(guard);
}
#[test]
fn missing_blob_fails_before_copy_or_switch() {
    let f = Fixture::new();
    f.seed();
    let blob = fs::read_dir(f.source.join("data/blobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::remove_file(blob).unwrap();
    assert!(f.migrate(&f.task()).is_err());
    assert_eq!(f.locations.read().unwrap().root, f.source);
    assert!(!f.target.join("history.db").exists());
}
#[test]
fn mismatched_key_never_overwrites_source_or_switches() {
    let f = Fixture::new();
    f.seed();
    let other = f.root.join("other-key");
    crate::platform::master_key(&other).unwrap();
    fs::copy(other.join("key.bin"), f.source.join("key.bin")).unwrap();
    let before = fs::read(f.source.join("history.db")).unwrap();
    assert!(f.migrate(&f.task()).is_err());
    assert_eq!(fs::read(f.source.join("history.db")).unwrap(), before);
    assert_eq!(f.locations.read().unwrap().root, f.source);
}
#[test]
fn unexpected_target_file_blocks_migration_without_overwrite() {
    let f = Fixture::new();
    f.seed();
    fs::write(f.target.join("history.db"), b"unrelated database").unwrap();
    assert!(f.migrate(&f.task()).is_err());
    assert_eq!(
        fs::read(f.target.join("history.db")).unwrap(),
        b"unrelated database"
    );
}
#[test]
fn interrupted_promotion_resumes_against_manifest() {
    let f = Fixture::new();
    f.seed();
    let task = f.task();
    let interrupted = catch_unwind(AssertUnwindSafe(|| {
        migration::execute(&f.locations, &task, &mut |step, done, _| {
            if step == "正在复制并校验加密数据" && done > 0 {
                panic!("合成中断");
            }
        })
        .unwrap();
    }));
    assert!(interrupted.is_err());
    assert_eq!(f.locations.read().unwrap().root, f.source);
    assert!(f.stage(&task).join(".clipraft-manifest.json").exists());
    let (_, guard) = f.migrate(&task).unwrap();
    assert_eq!(
        files::inventory(&f.source).unwrap(),
        files::inventory(&f.target).unwrap()
    );
    assert!(!f.stage(&task).exists());
    drop(guard);
}
#[test]
fn partial_staged_file_is_recopied_but_unknown_file_is_not_removed() {
    let f = Fixture::new();
    f.seed();
    let task = f.task();
    let _ = catch_unwind(AssertUnwindSafe(|| {
        migration::execute(&f.locations, &task, &mut |step, _, _| {
            if step == "正在复制并校验加密数据" {
                panic!("复制前中断");
            }
        })
        .unwrap();
    }));
    let stage = f.stage(&task);
    fs::write(stage.join("key.bin"), b"partial").unwrap();
    fs::write(stage.join("unrelated"), b"keep").unwrap();
    assert!(f.migrate(&task).is_err());
    assert_eq!(fs::read(stage.join("unrelated")).unwrap(), b"keep");
    fs::remove_file(stage.join("unrelated")).unwrap();
    let (_, guard) = f.migrate(&task).unwrap();
    drop(guard);
}
#[test]
fn failed_config_commit_can_resume_without_overwriting_completed_payload() {
    let f = Fixture::new();
    f.seed();
    let task = f.task();
    let mut blocked = f.locations.clone();
    blocked.config = Some(f.root.join("profile"));
    assert!(migration::execute(&blocked, &task, &mut |_, _, _| {}).is_err());
    assert!(f.target.join("history.db").exists());
    assert_eq!(f.locations.read().unwrap().root, f.source);
    let (_, guard) = f.migrate(&task).unwrap();
    assert_eq!(f.locations.read().unwrap().root, f.target);
    drop(guard);
}
#[test]
fn source_change_after_interruption_is_not_silently_merged() {
    let f = Fixture::new();
    f.seed();
    let task = f.task();
    let _ = catch_unwind(AssertUnwindSafe(|| {
        migration::execute(&f.locations, &task, &mut |step, _, _| {
            if step == "正在复制并校验加密数据" {
                panic!("合成中断");
            }
        })
        .unwrap();
    }));
    fs::write(f.source.join("history.db.bak-v4"), b"changed snapshot").unwrap();
    assert!(f.migrate(&task).is_err());
    assert_eq!(f.locations.read().unwrap().root, f.source);
}
#[test]
fn target_lock_blocks_migration_and_source_remains_authoritative() {
    let f = Fixture::new();
    f.seed();
    let guard = files::lock(&f.target).unwrap();
    assert!(f.migrate(&f.task()).is_err());
    assert_eq!(f.locations.read().unwrap().root, f.source);
    drop(guard);
}
#[test]
fn move_new_refuses_to_replace_existing_files() {
    let f = Fixture::new();
    let from = f.source.join("from");
    let to = f.target.join("to");
    fs::write(&from, "new").unwrap();
    fs::write(&to, "old").unwrap();
    assert!(config::move_new(&from, &to).is_err());
    assert_eq!(fs::read_to_string(to).unwrap(), "old");
    assert_eq!(fs::read_to_string(from).unwrap(), "new");
}

#[test]
fn configured_missing_data_never_regenerates_database_or_key() {
    let f = Fixture::new();
    f.locations
        .save(&Configuration::initial(f.target.clone()))
        .unwrap();
    assert!(load_data(&f.locations).is_err());
    assert!(!f.target.join("history.db").exists());
    assert!(!f.target.join("key.bin").exists());
}
#[test]
fn session_uses_configured_root_and_keeps_cache_at_profile() {
    let f = Fixture::new();
    f.seed();
    let (config, guard) = f.migrate(&f.task()).unwrap();
    drop(guard);
    let session = Session::open_at(f.locations.clone()).unwrap();
    assert_eq!(session.root, config.root);
    assert_eq!(session.cache, f.root.join("profile/webview"));
    assert!(files::lock(&f.target).is_err());
    drop(session);
    assert!(files::lock(&f.target).is_ok());
}
#[test]
fn locked_key_failure_does_not_switch_or_change_ciphertext() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    f.seed();
    let before = fs::read(f.source.join("history.db")).unwrap();
    let key = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(f.source.join("key.bin"))
        .unwrap();
    assert!(f.migrate(&f.task()).is_err());
    assert_eq!(fs::read(f.source.join("history.db")).unwrap(), before);
    assert_eq!(f.locations.read().unwrap().root, f.source);
    drop(key);
}
#[test]
fn active_reader_blocks_wal_checkpoint_and_preserves_pending_source() {
    let f = Fixture::new();
    f.seed();
    let key = crate::platform::master_key(&f.source).unwrap();
    let reader = rusqlite::Connection::open(f.source.join("history.db")).unwrap();
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    reader
        .pragma_update(None, "key", format!("x'{hex}'"))
        .unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM items;")
        .unwrap();
    let mut writer = Storage::open(&f.source, &key).unwrap();
    writer.create_group("WAL 中的新分组").unwrap();
    assert!(f.source.join("history.db-wal").metadata().unwrap().len() > 0);
    let task = f.task();
    assert!(f.migrate(&task).is_err());
    assert!(!f.target.join("history.db").exists());
    reader.execute_batch("ROLLBACK").unwrap();
    drop(reader);
    drop(writer);
    let (_, guard) = f.migrate(&task).unwrap();
    drop(guard);
}
#[test]
fn junction_inside_payload_is_rejected_without_following_target() {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let f = Fixture::new();
    f.seed();
    let outside = f.root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("user-file"), "保留").unwrap();
    let junction = f.source.join("data/junction");
    let script = format!(
        "New-Item -ItemType Junction -Path '{}' -Target '{}' -ErrorAction Stop | Out-Null",
        junction.display().to_string().replace('\'', "''"),
        outside.display().to_string().replace('\'', "''")
    );
    let exit = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(exit.success());
    assert!(files::inventory(&f.source).is_err());
    assert_eq!(
        fs::read_to_string(outside.join("user-file")).unwrap(),
        "保留"
    );
    fs::remove_dir(junction).unwrap();
}
#[test]
#[ignore = "需两个本地固定卷；按 S1 验收文档单独执行"]
fn cross_volume_copy_preserves_source_and_complete_target() {
    let mut f = Fixture::new();
    f.seed();
    let temp = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
        .join("Temp")
        .join(format!("clipraft-s1-{}", config::nonce().unwrap()));
    fs::create_dir(&temp).unwrap();
    let temp = fs::canonicalize(temp).unwrap();
    assert_ne!(
        temp.components().next(),
        f.root.components().next(),
        "本用例需要两个不同的本地卷"
    );
    let original = f.target.clone();
    f.target = temp.join("跨盘目标");
    fs::create_dir(&f.target).unwrap();
    let (committed, guard) = f.migrate(&f.task()).unwrap();
    assert_eq!(committed.root, f.target);
    assert_eq!(
        files::inventory(&f.source).unwrap(),
        files::inventory(&f.target).unwrap()
    );
    drop(guard);
    fs::remove_dir_all(&temp).unwrap();
    f.target = original;
}

#[test]
fn installed_profile_is_separate_and_imports_legacy_location_before_default_cleanup() {
    let f = Fixture::new();
    f.seed();
    let locations = Locations::installed(&f.root).unwrap();
    fs::create_dir_all(&locations.default_root).unwrap();
    let legacy = Configuration::initial(f.source.clone());
    legacy
        .save(locations.legacy_config.as_ref().unwrap())
        .unwrap();
    assert_eq!(locations.read().unwrap().root, f.source);
    assert!(!locations
        .config
        .as_ref()
        .unwrap()
        .starts_with(&locations.default_root));
    let session = Session::open_at(locations.clone()).unwrap();
    session.persist_location().unwrap();
    drop(session);
    // 只清理本用例创建的默认目录，独立配置仍指向原有完整历史。
    fs::remove_dir_all(&locations.default_root).unwrap();
    assert_eq!(locations.read().unwrap().root, f.source);
    let reopened = Session::open_at(locations).unwrap();
    assert_eq!(reopened.root, f.source);
}
#[test]
fn deleting_current_locator_never_falls_back_even_with_stale_legacy_file() {
    let f = Fixture::new();
    f.seed();
    let locations = Locations::installed(&f.root).unwrap();
    fs::create_dir_all(&locations.default_root).unwrap();
    fs::create_dir_all(locations.config.as_ref().unwrap().parent().unwrap()).unwrap();
    Configuration::initial(locations.default_root.clone())
        .save(locations.legacy_config.as_ref().unwrap())
        .unwrap();
    locations
        .save(&Configuration::initial(f.source.clone()))
        .unwrap();
    fs::remove_file(locations.config.as_ref().unwrap()).unwrap();
    assert!(locations
        .read()
        .unwrap_err()
        .to_string()
        .contains("启动定位文件丢失"));
    assert!(load_data(&locations).is_err());
    assert!(!locations.default_root.join("history.db").exists());
    assert!(!locations.default_root.join("key.bin").exists());
}
#[test]
fn legacy_cache_without_locator_or_database_is_not_a_fresh_install() {
    let f = Fixture::new();
    let locations = Locations::installed(&f.root).unwrap();
    fs::create_dir_all(&locations.cache).unwrap();
    assert!(load_data(&locations).is_err());
    assert!(!locations.default_root.join("history.db").exists());
}
#[test]
fn deleted_locator_while_running_blocks_preview_and_confirmation_without_restart_hint() {
    let f = Fixture::new();
    f.seed();
    f.locations
        .save(&Configuration::initial(f.source.clone()))
        .unwrap();
    let control = f.control();
    let plan = control.preview(&f.target).unwrap();
    fs::remove_file(f.locations.config.as_ref().unwrap()).unwrap();
    let error = control.preview(&f.target).err().unwrap().to_string();
    assert!(error.contains("当前数据仍在"));
    assert!(!error.contains("重新启动"));
    assert!(control.schedule(&plan.ticket).is_err());
    assert!(!f.target.join("history.db").exists());
}
fn returning(f: &Fixture) -> (Locations, Configuration) {
    let id = f.seed();
    let (_, guard) = f.migrate(&f.task()).unwrap();
    drop(guard);
    let key = crate::platform::master_key(&f.target).unwrap();
    let mut storage = Storage::open(&f.target, &key).unwrap();
    storage.create_group("自定义目录新增分组").unwrap();
    assert!(storage.preview_item(&id).is_ok());
    drop(storage);
    let archive = restore::preview(&f.source, &f.locations).unwrap().unwrap();
    let task = Configuration {
        version: 1,
        startup_capture: Default::default(),
        root: f.target.clone(),
        previous: Some(f.source.clone()),
        cleanup: None,
        displaced: None,
        replacement: None,
        pending: Some(Migration {
            source: f.target.clone(),
            target: f.source.clone(),
            id: config::nonce().unwrap(),
            displaced: Some(archive),
        }),
    };
    (f.locations.clone(), task)
}
#[test]
fn restore_default_preserves_old_history_and_keeps_latest_custom_records() {
    let f = Fixture::new();
    let (locations, task) = returning(&f);
    fs::create_dir(f.source.join("webview")).unwrap();
    fs::write(f.source.join("webview/cache"), b"held cache").unwrap();
    let before = files::inventory(&f.source).unwrap();
    let current = files::inventory(&f.target).unwrap();
    let archive = task.pending.as_ref().unwrap().displaced.as_ref().unwrap();
    let (committed, guard) = migration::execute(&locations, &task, &mut |_, _, _| {}).unwrap();
    assert_eq!(files::inventory(archive).unwrap(), before);
    assert_eq!(files::inventory(&f.source).unwrap(), current);
    assert_eq!(committed.displaced.as_ref(), Some(archive));
    assert_eq!(
        fs::read(f.source.join("webview/cache")).unwrap(),
        b"held cache"
    );
    assert_eq!(locations.read().unwrap().root, f.source);
    database::verify(archive).unwrap();
    drop(guard);
}
#[test]
fn restore_default_resume_does_not_archive_new_payload_a_second_time() {
    let f = Fixture::new();
    let (locations, task) = returning(&f);
    let archive = task.pending.as_ref().unwrap().displaced.as_ref().unwrap();
    let old = files::inventory(&f.source).unwrap();
    let _ = catch_unwind(AssertUnwindSafe(|| {
        migration::execute(&locations, &task, &mut |step, done, _| {
            if step == "正在复制并校验加密数据" && done > 0 {
                panic!("保留旧数据后中断");
            }
        })
        .unwrap();
    }));
    assert_eq!(files::inventory(archive).unwrap(), old);
    let (_, guard) = migration::execute(&locations, &task, &mut |_, _, _| {}).unwrap();
    assert_eq!(files::inventory(archive).unwrap(), old);
    assert_eq!(
        files::inventory(&f.source).unwrap(),
        files::inventory(&f.target).unwrap()
    );
    drop(guard);
}
#[test]
fn ordinary_nonempty_directory_still_rejected_without_automatic_archival() {
    let f = Fixture::new();
    f.seed();
    fs::write(f.target.join("user.txt"), b"keep").unwrap();
    assert!(f.control().preview(&f.target).is_err());
    assert_eq!(fs::read(f.target.join("user.txt")).unwrap(), b"keep");
}
#[test]
fn unknown_default_file_blocks_restore_before_moving_history() {
    let f = Fixture::new();
    f.seed();
    let before = files::inventory(&f.source).unwrap();
    fs::write(f.source.join("user.txt"), b"keep").unwrap();
    assert!(restore::preview(&f.source, &f.locations).is_err());
    assert_eq!(files::inventory(&f.source).unwrap(), before);
}
#[test]
fn first_successful_start_persists_location_and_marker() {
    let f = Fixture::new();
    let session = Session::open_at(f.locations.clone()).unwrap();
    let key = crate::platform::master_key(&session.root).unwrap();
    let storage = Storage::open(&session.root, &key).unwrap();
    session.persist_location().unwrap();
    assert!(f.locations.config.as_ref().unwrap().is_file());
    assert!(f.root.join("profile/initialized").is_file());
    drop(storage);
    drop(session);
}

#[test]
fn locator_removed_between_presence_check_and_read_is_an_error() {
    let f = Fixture::new();
    let path = f.locations.config.as_ref().unwrap();
    f.locations
        .save(&Configuration::initial(f.source.clone()))
        .unwrap();
    assert!(path.try_exists().unwrap());
    fs::remove_file(path).unwrap();
    assert!(Configuration::load(path).is_err());
}
#[test]
fn interrupted_old_data_move_resumes_and_preserves_original_archive() {
    let f = Fixture::new();
    let (locations, task) = returning(&f);
    let archive = task.pending.as_ref().unwrap().displaced.as_ref().unwrap();
    database::checkpoint(&f.source).unwrap();
    let old = files::inventory(&f.source).unwrap();
    let mut names: Vec<String> = fs::read_dir(&f.source)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| files::managed(name))
        .collect();
    names.sort();
    fs::create_dir(archive).unwrap();
    let journal =
        serde_json::json!({"target": f.source, "names": names, "entries": old, "complete": false});
    config::atomic_write(
        &archive.join("restore-journal.json"),
        &serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
    config::move_new(&f.source.join("data"), &archive.join("data")).unwrap();
    let (_, guard) = migration::execute(&locations, &task, &mut |_, _, _| {}).unwrap();
    assert_eq!(files::inventory(archive).unwrap(), old);
    assert_eq!(
        files::inventory(&f.source).unwrap(),
        files::inventory(&f.target).unwrap()
    );
    drop(guard);
}
#[test]
fn archive_path_outside_default_sibling_is_rejected_without_moving_data() {
    let f = Fixture::new();
    let (locations, mut task) = returning(&f);
    let before = files::inventory(&f.source).unwrap();
    task.pending.as_mut().unwrap().displaced = Some(f.target.join("outside"));
    assert!(migration::execute(&locations, &task, &mut |_, _, _| {}).is_err());
    assert_eq!(files::inventory(&f.source).unwrap(), before);
    assert!(!f.target.join("outside").exists());
}
#[test]
fn roundtrip_via_real_control_waits_for_cleanup_then_reuses_clean_source() {
    let f = Fixture::new();
    let id = f.seed();
    let (_, guard) = f.migrate(&f.task()).unwrap();
    drop(guard);
    let control = DirectoryControl {
        locations: f.locations.clone(),
        root: f.target.clone(),
        plan: Mutex::new(None),
    };
    assert!(control.preview(&f.source).is_err());
    control.mark_migration_initialized().unwrap();
    control.retry_source_cleanup().unwrap();
    assert!(!f.source.join("history.db").exists());
    let plan = control.preview(&f.source).unwrap();
    control.schedule(&plan.ticket).unwrap();
    let task = f.locations.read().unwrap();
    assert!(task.pending.as_ref().unwrap().displaced.is_none());
    let (_, guard) = f.migrate(&task).unwrap();
    let key = crate::platform::master_key(&f.source).unwrap();
    let storage = Storage::open(&f.source, &key).unwrap();
    assert!(storage.preview_item(&id).is_ok());
    drop(storage);
    drop(guard);
}

#[test]
fn restored_activation_preserves_source_and_next_session_opens_target() {
    let f = Fixture::new();
    let id = f.seed();
    f.locations
        .save(&Configuration::initial(f.source.clone()))
        .unwrap();
    let master = crate::platform::master_key(&f.source).unwrap();
    let mut source = Storage::open(&f.source, &master).unwrap();
    let restored = f.root.join("restored");
    source.create_portable_snapshot(&restored, &master).unwrap();
    crate::platform::install_master_key(&restored, &master).unwrap();
    let before = files::inventory(&f.source).unwrap();
    let guard = f.control().activate_restored(&restored).unwrap();
    let config = f.locations.read().unwrap();
    assert_eq!(config.root, fs::canonicalize(&restored).unwrap());
    assert_eq!(config.previous, Some(f.source.clone()));
    assert!(config.pending.is_none());
    assert_eq!(files::inventory(&f.source).unwrap(), before);
    assert!(files::lock(&restored).is_err());
    drop(guard);
    let session = Session::open_at(f.locations.clone()).unwrap();
    let reopened = Storage::open(
        &session.root,
        &crate::platform::master_key(&session.root).unwrap(),
    )
    .unwrap();
    assert_eq!(reopened.load(&id).unwrap().hash(), id);
    assert!(source.load(&id).is_ok());
}

#[test]
fn corrupt_or_busy_restored_database_never_changes_startup_location() {
    let f = Fixture::new();
    f.seed();
    f.locations
        .save(&Configuration::initial(f.source.clone()))
        .unwrap();
    let master = crate::platform::master_key(&f.source).unwrap();
    let mut source = Storage::open(&f.source, &master).unwrap();
    let restored = f.root.join("restored");
    source.create_snapshot(&restored, &master).unwrap();
    crate::platform::install_master_key(&restored, &master).unwrap();
    let config_path = f.locations.config.as_ref().unwrap();
    let before = fs::read(config_path).unwrap();
    let guard = files::lock(&restored).unwrap();
    assert!(f.control().activate_restored(&restored).is_err());
    drop(guard);
    assert_eq!(fs::read(config_path).unwrap(), before);
    fs::write(restored.join("history.db"), b"corrupted").unwrap();
    assert!(f.control().activate_restored(&restored).is_err());
    assert_eq!(fs::read(config_path).unwrap(), before);
    assert!(f.control().activate_restored(&f.source).is_err());
}

#[test]
fn activation_rejects_pending_migration_and_isolated_configuration() {
    let f = Fixture::new();
    f.seed();
    f.locations.save(&f.task()).unwrap();
    assert!(f.control().activate_restored(&f.target).is_err());
    assert!(f.locations.read().unwrap().pending.is_some());
    let mut control = f.control();
    control.locations.config = None;
    assert!(control.activate_restored(&f.target).is_err());
}

#[test]
fn failed_activation_config_write_keeps_original_location_and_both_databases() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let id = f.seed();
    f.locations
        .save(&Configuration::initial(f.source.clone()))
        .unwrap();
    let master = crate::platform::master_key(&f.source).unwrap();
    let mut source = Storage::open(&f.source, &master).unwrap();
    let restored = f.root.join("restored");
    source.create_portable_snapshot(&restored, &master).unwrap();
    crate::platform::install_master_key(&restored, &master).unwrap();
    let config_path = f.locations.config.as_ref().unwrap();
    let before = fs::read(config_path).unwrap();
    let hold = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(config_path)
        .unwrap();
    assert!(f.control().activate_restored(&restored).is_err());
    assert_eq!(fs::read(config_path).unwrap(), before);
    assert!(source.load(&id).is_ok());
    Storage::verify_snapshot(&restored, &master).unwrap();
    assert!(files::lock(&restored).is_ok());
    drop(hold);
}

#[test]
fn modified_restored_object_is_rechecked_before_activation() {
    let f = Fixture::new();
    f.seed();
    f.locations
        .save(&Configuration::initial(f.source.clone()))
        .unwrap();
    let master = crate::platform::master_key(&f.source).unwrap();
    let mut source = Storage::open(&f.source, &master).unwrap();
    let restored = f.root.join("restored");
    source.create_snapshot(&restored, &master).unwrap();
    crate::platform::install_master_key(&restored, &master).unwrap();
    let before = fs::read(f.locations.config.as_ref().unwrap()).unwrap();
    let object = fs::read_dir(restored.join("data/blobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(object, b"modified after successful restore").unwrap();
    assert!(f.control().activate_restored(&restored).is_err());
    assert_eq!(
        fs::read(f.locations.config.as_ref().unwrap()).unwrap(),
        before
    );
}

#[test]
fn b1_migration_backup_precedes_switch_and_source_cleanup_waits_for_initialization() {
    let f = Fixture::new();
    let id = f.seed();
    fs::write(f.source.join("notes.txt"), b"user notes").unwrap();
    fs::write(f.source.join("data/user.txt"), b"nested user file").unwrap();
    fs::write(f.source.join("diag.log"), b"keep log").unwrap();
    let (committed, guard) = f.migrate(&f.task()).unwrap();
    let cleanup = committed.cleanup.as_ref().unwrap();
    assert!(cleanup.backup.join("history.db").exists());
    assert!(!cleanup.backup.starts_with(&f.source));
    assert!(!cleanup.initialized);
    let control = DirectoryControl {
        locations: f.locations.clone(),
        root: f.target.clone(),
        plan: Mutex::new(None),
    };
    assert!(control.retry_source_cleanup().is_err());
    assert!(f.source.join("history.db").exists());
    let key = crate::platform::master_key(&f.target).unwrap();
    let new_store = Storage::open(&f.target, &key).unwrap();
    assert!(new_store.load(&id).is_ok());
    control.mark_migration_initialized().unwrap();
    control.retry_source_cleanup().unwrap();
    let after = f.locations.read().unwrap();
    assert_eq!(after.root, f.target);
    assert!(after.cleanup.as_ref().unwrap().complete);
    assert!(!f.source.join("history.db").exists());
    assert!(!f.source.join("key.bin").exists());
    assert_eq!(fs::read(f.source.join("notes.txt")).unwrap(), b"user notes");
    assert!(f.source.join("diag.log").exists());
    assert_eq!(
        fs::read(f.source.join("data/user.txt")).unwrap(),
        b"nested user file"
    );
    database::verify(&cleanup.backup).unwrap();
    assert!(new_store.load(&id).is_ok());
    control.retry_source_cleanup().unwrap();
    drop(new_store);
    drop(guard);
}
#[test]
fn b1_cleanup_changed_source_is_retained_and_retry_never_rolls_back_target() {
    let f = Fixture::new();
    f.seed();
    let (committed, guard) = f.migrate(&f.task()).unwrap();
    let cleanup = committed.cleanup.unwrap();
    let control = DirectoryControl {
        locations: f.locations.clone(),
        root: f.target.clone(),
        plan: Mutex::new(None),
    };
    let original = fs::read(f.source.join("history.db.bak-v4")).unwrap();
    fs::write(f.source.join("history.db.bak-v4"), b"changed source").unwrap();
    control.mark_migration_initialized().unwrap();
    assert!(control.retry_source_cleanup().is_err());
    let after = f.locations.read().unwrap();
    assert_eq!(after.root, f.target);
    assert!(!after.cleanup.unwrap().complete);
    assert_eq!(
        fs::read(f.source.join("history.db.bak-v4")).unwrap(),
        b"changed source"
    );
    assert!(f.source.join("history.db").exists(), "源变化时整批保留");
    fs::write(f.source.join("history.db.bak-v4"), original).unwrap();
    control.retry_source_cleanup().unwrap();
    assert!(f.locations.read().unwrap().cleanup.unwrap().complete);
    database::verify(&cleanup.backup).unwrap();
    drop(guard);
}
#[test]
fn b1_missing_backup_or_new_source_wal_prevents_source_deletion() {
    let f = Fixture::new();
    f.seed();
    let (committed, guard) = f.migrate(&f.task()).unwrap();
    let cleanup = committed.cleanup.unwrap();
    let control = DirectoryControl {
        locations: f.locations.clone(),
        root: f.target.clone(),
        plan: Mutex::new(None),
    };
    control.mark_migration_initialized().unwrap();
    fs::write(f.source.join("history.db-wal"), b"new transactions").unwrap();
    assert!(control.retry_source_cleanup().is_err());
    assert!(f.source.join("key.bin").exists());
    fs::remove_file(f.source.join("history.db-wal")).unwrap();
    fs::write(cleanup.backup.join("history.db"), b"corrupted backup").unwrap();
    assert!(control.retry_source_cleanup().is_err());
    assert!(f.source.join("history.db").exists());
    assert_eq!(f.locations.read().unwrap().root, f.target);
    drop(guard);
}
#[test]
fn b1_same_content_replacement_is_not_deleted_and_reboot_retries_pending_cleanup() {
    let f = Fixture::new();
    f.seed();
    let (_, guard) = f.migrate(&f.task()).unwrap();
    let path = f.source.join("history.db.bak-v4");
    let bytes = fs::read(&path).unwrap();
    fs::rename(&path, f.source.join("user-kept-copy")).unwrap();
    fs::write(&path, bytes).unwrap();
    drop(guard);
    let session = Session::open_at(f.locations.clone()).unwrap();
    session.control.mark_migration_initialized().unwrap();
    assert!(session.control.retry_source_cleanup().is_err());
    assert!(path.exists());
    assert!(f.source.join("user-kept-copy").exists());
    assert_eq!(f.locations.read().unwrap().root, f.target);
}

#[test]
fn b1_cleanup_retry_after_partial_delete_and_source_lock_failure_is_idempotent() {
    let f = Fixture::new();
    f.seed();
    let (committed, guard) = f.migrate(&f.task()).unwrap();
    let cleanup = committed.cleanup.unwrap();
    let control = DirectoryControl {
        locations: f.locations.clone(),
        root: f.target.clone(),
        plan: Mutex::new(None),
    };
    control.mark_migration_initialized().unwrap();
    let source_lock = files::lock(&f.source).unwrap();
    assert!(control.retry_source_cleanup().is_err());
    assert!(f.source.join("history.db").exists());
    drop(source_lock);
    crate::owned_files::remove(&f.source, &cleanup.entries[0]).unwrap();
    control.retry_source_cleanup().unwrap();
    control.retry_source_cleanup().unwrap();
    assert!(f.locations.read().unwrap().cleanup.unwrap().complete);
    assert_eq!(f.locations.read().unwrap().root, f.target);
    drop(guard);
}
#[test]
fn b1_invalid_backup_content_prevents_target_publication_and_configuration_switch() {
    let f = Fixture::new();
    f.seed();
    let task = f.task();
    f.locations.save(&task).unwrap();
    let before = fs::read(f.locations.config.as_ref().unwrap()).unwrap();
    let blob = fs::read_dir(f.source.join("data/blobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(blob, b"synthetic corrupt blob").unwrap();
    assert!(f.migrate(&task).is_err());
    assert!(!f.target.join("history.db").exists());
    assert!(f.source.join("history.db").exists());
    assert_eq!(
        fs::read(f.locations.config.as_ref().unwrap()).unwrap(),
        before
    );
}

#[test]
fn deferred_restored_activation_skips_old_clipboard_then_accepts_new_copy() {
    use crate::capture_policy::{self, Snapshot, StartupCapture};
    use std::sync::atomic::AtomicU32;
    let f = Fixture::new();
    let original_id = f.seed();
    f.locations
        .save(&Configuration::initial(f.source.clone()))
        .unwrap();
    let master = crate::platform::master_key(&f.source).unwrap();
    let mut source = Storage::open(&f.source, &master).unwrap();
    let restored = f.root.join("deferred-restored");
    source.create_portable_snapshot(&restored, &master).unwrap();
    crate::platform::install_master_key(&restored, &master).unwrap();
    let later = Clip {
        formats: vec![FormatData {
            name: "CF_UNICODETEXT".into(),
            bytes: "after export"
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
        }],
    };
    source.save(&later).unwrap();
    // 模拟暂不替换期间旧库继续使用；重新进入界面不会改变恢复副本。
    assert!(source.load(&later.hash()).is_ok());
    let guard = f.control().activate_restored(&restored).unwrap();
    drop(guard);
    let session = Session::open_at(f.locations.clone()).unwrap();
    assert_eq!(session.startup_capture, StartupCapture::ChangesOnly);
    let mut target = Storage::open(&session.root, &master).unwrap();
    let last = AtomicU32::new(0);
    session.startup_capture.initialize_sequence(&last, || 100);
    let mut reads = 0;
    let skipped = capture_policy::run(
        &last,
        || Snapshot {
            sequence: 100,
            self_write: false,
        },
        |_| {
            reads += 1;
            Ok(Some(later.clone()))
        },
        |_| {},
        |_| {},
    )
    .unwrap();
    assert!(skipped.is_none());
    assert_eq!(reads, 0);
    assert!(target.load(&later.hash()).is_err());
    assert!(target.load(&original_id).is_ok());
    let new = capture_policy::run(
        &last,
        || Snapshot {
            sequence: 101,
            self_write: false,
        },
        |_| Ok(Some(later.clone())),
        |_| {},
        |_| {},
    )
    .unwrap()
    .unwrap();
    target.save(&new).unwrap();
    assert!(target.load(&later.hash()).is_ok());
    session.control.acknowledge_restored_startup().unwrap();
    assert_eq!(
        f.locations.read().unwrap().startup_capture,
        StartupCapture::Existing
    );
}
