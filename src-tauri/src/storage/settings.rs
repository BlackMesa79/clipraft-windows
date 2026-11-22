use super::Storage;
use crate::settings::Settings;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
impl Storage {
    pub fn load_settings(&self) -> Result<Settings> {
        let mut query = self
            .conn
            .prepare("SELECT key,value FROM settings WHERE key NOT IN (?1,?2)")?;
        let rows = query
            .query_map(
                [
                    super::background::REGULAR_KEY,
                    super::background::COMPACT_KEY,
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
        Settings::from_rows(&rows, crate::diag::write)
    }
    pub fn save_settings(&mut self, settings: &Settings) -> Result<()> {
        let settings = settings.clone().normalized()?;
        let json = serde_json::to_value(&settings)?;
        let tx = self.conn.transaction()?;
        for (key, value) in json.as_object().context("序列化设置不是 JSON 对象")? {
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [key, &value.to_string()])?;
        }
        if settings.sensitive_ttl_minutes == 0 {
            tx.execute("UPDATE items SET expires_at=NULL WHERE sensitive=1", [])?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    // 仅供运行时集成测试使用；不向发布构建暴露数据库连接。
    impl Storage {
        pub(crate) fn test_settings_rows(&self) -> BTreeMap<String, String> {
            raw_rows(self)
        }
        pub(crate) fn test_reject_settings_update(&self) {
            self.conn.execute_batch("CREATE TRIGGER reject_runtime_settings BEFORE UPDATE ON settings WHEN new.key='quick_visible_count' BEGIN SELECT RAISE(ABORT,'synthetic runtime settings failure'); END;").unwrap();
        }
    }
    #[test]
    fn removed_background_rows_are_ignored_without_rewriting() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let settings = Settings {
            max_items: 4321,
            ..Default::default()
        };
        storage.save_settings(&settings).unwrap();
        for value in [r#""soft""#, r#""frosted""#, r#""native""#, "{broken"] {
            storage.conn.execute("INSERT INTO settings(key,value) VALUES('quick_background',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [value]).unwrap();
            let before = raw_rows(&storage);
            let loaded = storage.load_settings().unwrap();
            assert_eq!(loaded, settings);
            assert!(serde_json::to_value(&loaded)
                .unwrap()
                .get("quick_background")
                .is_none());
            storage.save_settings(&loaded).unwrap();
            assert_eq!(raw_rows(&storage), before);
        }
    }

    fn raw_rows(storage: &Storage) -> BTreeMap<String, String> {
        storage
            .conn
            .prepare("SELECT key,value FROM settings ORDER BY key")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    // T3-1：通过真实 settings 表验证容错，不只直接调用反序列化函数。
    #[test]
    fn database_each_corrupt_or_out_of_range_row_falls_back_independently() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let saved = Settings {
            autostart: true,
            pause_on_start: true,
            max_items: 4321,
            max_bytes: 20 * crate::settings::MB,
            max_days: 17,
            ignored_apps: vec!["custom.exe".into()],
            sensitive_detection: false,
            sensitive_high_entropy: false,
            sensitive_ttl_minutes: 21,
            main_hotkey: crate::settings::Hotkey::CtrlBacktick,
            quick_hotkey: crate::settings::Hotkey::AltShiftV,
            quick_visible_count: 9,
            ..Settings::default()
        };
        let saved_json = serde_json::to_value(&saved).unwrap();
        let defaults = serde_json::to_value(Settings::default()).unwrap();
        for key in saved_json.as_object().unwrap().keys() {
            let mut invalid = vec!["{broken".to_string(), "null".into(), "{}".into()];
            if let Some(range) = crate::settings::ranges().get(key.as_str()) {
                invalid.extend(["\"42\"".into(), "-1".into(), (range.max + 1).to_string()]);
                if let Some(below) = range.min.checked_sub(1) {
                    invalid.push(below.to_string());
                }
            }
            for bad in invalid {
                storage.save_settings(&saved).unwrap();
                storage
                    .conn
                    .execute("UPDATE settings SET value=?1 WHERE key=?2", [&bad, key])
                    .unwrap();
                let before = raw_rows(&storage);
                let actual = storage.load_settings().unwrap();
                let mut expected = saved_json.clone();
                expected[key] = defaults[key].clone();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    expected,
                    "{key}={bad}"
                );
                assert_eq!(raw_rows(&storage), before, "加载回退不应修写数据库");
            }
        }
    }

    #[test]
    fn every_validation_failure_preserves_exact_database_rows() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let old = Settings {
            autostart: true,
            ..Default::default()
        };
        storage.save_settings(&old).unwrap();
        storage
            .conn
            .execute("INSERT INTO settings VALUES('future','opaque')", [])
            .unwrap();
        let before = raw_rows(&storage);
        for (key, bad) in [
            ("max_items", serde_json::json!(9)),
            ("max_bytes", serde_json::json!(1)),
            ("max_days", serde_json::json!(3651)),
            ("sensitive_ttl_minutes", serde_json::json!(1441)),
            ("quick_visible_count", serde_json::json!(16)),
            ("ignored_apps", serde_json::json!(["valid.exe", "bad.dll"])),
            ("ignored_apps", serde_json::json!([r"C:\Bad?.exe"])),
            ("main_hotkey", serde_json::json!("ctrl_shift_v")),
            ("quick_hotkey", serde_json::json!("alt_v")),
        ] {
            let mut json = serde_json::to_value(&old).unwrap();
            json["autostart"] = false.into();
            json[key] = bad;
            let invalid: Settings = serde_json::from_value(json).unwrap();
            assert!(storage.save_settings(&invalid).is_err(), "{key}");
            assert_eq!(raw_rows(&storage), before, "{key}");
        }
    }

    #[test]
    fn database_normalizes_saved_names_and_loads_valid_entries_independently() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let settings = Settings {
            ignored_apps: vec![r" C:\Apps\Good.EXE ".into(), "good.exe".into(), " ".into()],
            ..Default::default()
        };
        storage.save_settings(&settings).unwrap();
        assert_eq!(raw_rows(&storage)["ignored_apps"], r#"["Good.EXE"]"#);
        assert_eq!(storage.load_settings().unwrap().ignored_apps, ["Good.EXE"]);
        storage
            .conn
            .execute(
                "UPDATE settings SET value=?1 WHERE key='ignored_apps'",
                [r#"["Good.EXE","bad.dll","D:/Apps/Other.exe"]"#],
            )
            .unwrap();
        let before = raw_rows(&storage);
        assert_eq!(
            storage.load_settings().unwrap().ignored_apps,
            ["Good.EXE", "Other.exe"]
        );
        assert_eq!(raw_rows(&storage), before);
    }

    #[test]
    fn database_mid_insert_failure_rolls_back_new_rows_and_preserves_unknown_key() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        storage
            .conn
            .execute("INSERT INTO settings VALUES('future','opaque')", [])
            .unwrap();
        let before = raw_rows(&storage);
        storage.conn.execute_batch("CREATE TRIGGER reject_setting BEFORE INSERT ON settings WHEN new.key='quick_visible_count' BEGIN SELECT RAISE(ABORT,'synthetic insert failure'); END;").unwrap();
        assert!(storage.save_settings(&Settings::default()).is_err());
        assert_eq!(raw_rows(&storage), before);
        drop(storage);
        assert_eq!(raw_rows(&fixture.open()), before);
    }

    #[test]
    fn ignored_apps_database_roundtrip_deduplicates_and_preserves_first_spelling() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let saved = Settings {
            ignored_apps: vec!["App.exe".into(), "app.exe".into(), "APP.EXE".into()],
            ..Default::default()
        };
        storage.save_settings(&saved).unwrap();
        drop(storage);
        assert_eq!(
            fixture.open().load_settings().unwrap(),
            Settings {
                ignored_apps: vec!["App.exe".into()],
                ..saved
            }
        );
    }

    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../.test-data")
                    .join(format!(
                        "t3-settings-{}-{}",
                        std::process::id(),
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_nanos()
                    )),
            )
        }
        fn open(&self) -> Storage {
            Storage::open(&self.0, &[72; 32]).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn persistence_reopen_and_json_per_key() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let s = Settings {
            autostart: true,
            max_items: 4321,
            ..Default::default()
        };
        storage.save_settings(&s).unwrap();
        let raw: String = storage
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key='autostart'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, "true");
        drop(storage);
        assert_eq!(fixture.open().load_settings().unwrap(), s);
    }
    #[test]
    fn invalid_settings_never_write() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let old = Settings::default();
        storage.save_settings(&old).unwrap();
        let invalid = Settings {
            max_items: 9,
            autostart: true,
            ..Default::default()
        };
        assert!(storage.save_settings(&invalid).is_err());
        assert_eq!(storage.load_settings().unwrap(), old);
    }
    #[test]
    fn database_write_failure_rolls_back_all_keys() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let old = Settings::default();
        storage.save_settings(&old).unwrap();
        storage.conn.execute_batch("CREATE TRIGGER fail_settings BEFORE UPDATE ON settings WHEN new.key='max_items' BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END;").unwrap();
        assert!(storage
            .save_settings(&Settings {
                autostart: true,
                max_items: 1234,
                ..Default::default()
            })
            .is_err());
        assert_eq!(storage.load_settings().unwrap(), old);
    }
    #[test]
    fn damaged_and_unknown_database_values_are_tolerated() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        storage.save_settings(&Settings::default()).unwrap();
        storage.conn.execute_batch("UPDATE settings SET value='bad' WHERE key='max_days'; UPDATE settings SET value='1234' WHERE key='max_items'; DELETE FROM settings WHERE key='autostart'; INSERT INTO settings VALUES('future','broken');").unwrap();
        let loaded = storage.load_settings().unwrap();
        assert_eq!(loaded.max_days, 30);
        assert_eq!(loaded.max_items, 1234);
        assert!(!loaded.autostart);
        storage.save_settings(&loaded).unwrap();
        assert_eq!(
            storage
                .conn
                .query_row("SELECT value FROM settings WHERE key='future'", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "broken"
        );
    }
}
