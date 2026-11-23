//! U2：真实加密 settings 表往返、旧配置与损坏配置隔离；不启动窗口。
use clipraft_windows::settings::{Layout, MainPosition, Settings, Theme};
use clipraft_windows::storage::Storage;
use std::collections::BTreeMap;

#[test]
fn u2_ac1_legacy_json_defaults_without_losing_existing_preferences() {
    let old = Settings {
        max_items: 1234,
        ..Settings::default()
    };
    let mut json = serde_json::to_value(&old).unwrap();
    for key in [
        "skin",
        "theme",
        "auto_check_updates",
        "backdrop_strength",
        "layout",
        "compact",
        "hide_on_blur",
        "select_first_on_show",
        "main_position",
    ] {
        json.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(serde_json::from_value::<Settings>(json).unwrap(), old);
}

#[test]
fn u2_ac2_all_appearance_combinations_survive_database_reopen() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.test-data")
        .join(format!("u2-settings-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for theme in [Theme::Light, Theme::Dark, Theme::System] {
        for layout in [Layout::List, Layout::Cards] {
            for compact in [false, true] {
                for hide_on_blur in [false, true] {
                    for main_position in [MainPosition::Remember, MainPosition::Mouse] {
                        let expected = Settings {
                            skin: clipraft_windows::settings::Skin::Fluent,
                            theme,
                            auto_check_updates: compact,
                            layout,
                            compact,
                            hide_on_blur,
                            select_first_on_show: !hide_on_blur,
                            main_position,
                            max_items: 1234,
                            ..Settings::default()
                        };
                        {
                            let mut db = Storage::open(&root, &[91; 32]).unwrap();
                            db.save_settings(&expected).unwrap();
                        }
                        let db = Storage::open(&root, &[91; 32]).unwrap();
                        assert_eq!(db.load_settings().unwrap(), expected);
                    }
                }
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn u2_ac3_corrupt_fields_fall_back_independently_and_do_not_log_values() {
    for (key, invalid) in [
        ("skin", "\"SECRET_INVALID\""),
        ("theme", "\"SECRET_INVALID\""),
        ("auto_check_updates", "\"true\""),
        ("layout", "null"),
        ("compact", "1"),
        ("hide_on_blur", "\"true\""),
        ("select_first_on_show", "1"),
        ("main_position", "\"invalid\""),
    ] {
        let mut rows: BTreeMap<String, String> = serde_json::to_value(Settings {
            theme: Theme::Dark,
            auto_check_updates: true,
            layout: Layout::Cards,
            compact: true,
            hide_on_blur: true,
            select_first_on_show: true,
            main_position: MainPosition::Mouse,
            ..Settings::default()
        })
        .unwrap()
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.to_string()))
        .collect();
        rows.insert(key.into(), invalid.into());
        let mut warnings = vec![];
        let actual = Settings::from_rows(&rows, |w| warnings.push(w)).unwrap();
        let json = serde_json::to_value(actual).unwrap();
        let defaults = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(json[key], defaults[key]);
        for other in [
            "skin",
            "theme",
            "auto_check_updates",
            "layout",
            "compact",
            "hide_on_blur",
            "select_first_on_show",
            "main_position",
        ] {
            if other != key {
                assert_eq!(json[other].to_string(), rows[other]);
            }
        }
        assert_eq!(warnings.len(), 1);
        assert!(!warnings[0].contains("SECRET_INVALID"));
    }
}
