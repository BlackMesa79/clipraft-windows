use crate::storage::{validate_retention_policy, RetentionPolicy};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const MB: u64 = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hotkey {
    AltV,
    CtrlAltV,
    WinAltV,
    CtrlBacktick,
    CtrlShiftV,
    CtrlAltShiftV,
    AltShiftV,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub key: u32,
    pub modifiers: u16,
}
impl Hotkey {
    pub fn shortcut(self) -> Shortcut {
        let (key, modifiers) = match self {
            Self::AltV => (0x56, 4),
            Self::CtrlAltV => (0x56, 5),
            Self::WinAltV => (0x56, 12),
            Self::CtrlBacktick => (0xC0, 1),
            Self::CtrlShiftV => (0x56, 3),
            Self::CtrlAltShiftV => (0x56, 7),
            Self::AltShiftV => (0x56, 6),
        };
        Shortcut { key, modifiers }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::AltV => "Alt + V",
            Self::CtrlAltV => "Ctrl + Alt + V",
            Self::WinAltV => "Win + Alt + V",
            Self::CtrlBacktick => "Ctrl + `",
            Self::CtrlShiftV => "Ctrl + Shift + V",
            Self::CtrlAltShiftV => "Ctrl + Alt + Shift + V",
            Self::AltShiftV => "Alt + Shift + V",
        }
    }
    pub fn hold_label(self) -> &'static str {
        match self {
            Self::CtrlShiftV => "Ctrl + Shift",
            Self::CtrlAltShiftV => "Ctrl + Alt + Shift",
            Self::AltShiftV => "Alt + Shift",
            _ => self.label(),
        }
    }
}
pub const MAIN_HOTKEYS: [Hotkey; 4] = [
    Hotkey::AltV,
    Hotkey::CtrlAltV,
    Hotkey::WinAltV,
    Hotkey::CtrlBacktick,
];
pub const QUICK_HOTKEYS: [Hotkey; 3] =
    [Hotkey::CtrlShiftV, Hotkey::CtrlAltShiftV, Hotkey::AltShiftV];
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub autostart: bool,
    pub pause_on_start: bool,
    pub max_items: u32,
    pub max_bytes: u64,
    pub max_days: u32,
    pub ignored_apps: Vec<String>,
    pub sensitive_detection: bool,
    pub sensitive_ttl_minutes: u32,
    pub main_hotkey: Hotkey,
    pub quick_hotkey: Hotkey,
    pub quick_visible_count: u32,
}
impl Default for Settings {
    fn default() -> Self {
        let policy = RetentionPolicy::default();
        Self {
            autostart: false,
            pause_on_start: false,
            max_items: policy.max_items,
            max_bytes: policy.max_bytes,
            max_days: policy.max_days,
            ignored_apps: [
                "KeePass.exe",
                "KeePassXC.exe",
                "1Password.exe",
                "Bitwarden.exe",
            ]
            .map(String::from)
            .to_vec(),
            sensitive_detection: true,
            sensitive_ttl_minutes: 10,
            main_hotkey: Hotkey::AltV,
            quick_hotkey: Hotkey::CtrlShiftV,
            quick_visible_count: 5,
        }
    }
}
#[derive(Clone, Serialize)]
pub struct Range {
    pub min: u64,
    pub max: u64,
    pub step: u64,
}
pub fn ranges() -> BTreeMap<&'static str, Range> {
    [
        ("max_items", 10, 100_000, 1),
        ("max_bytes", 10 * MB, 100 * 1024 * MB, 1),
        ("max_days", 0, 3650, 1),
        ("sensitive_ttl_minutes", 1, 1440, 1),
        ("quick_visible_count", 3, 15, 1),
    ]
    .into_iter()
    .map(|(key, min, max, step)| (key, Range { min, max, step }))
    .collect()
}
fn shortcuts_conflict(main: Shortcut, quick: Shortcut) -> bool {
    main == quick || main.modifiers == quick.modifiers
}
pub fn validate_hotkeys(main: Hotkey, quick: Hotkey) -> Result<()> {
    if shortcuts_conflict(main.shortcut(), quick.shortcut()) {
        bail!("主窗口和快速面板快捷键不能相同或使用完全相同的修饰键");
    }
    if !MAIN_HOTKEYS.contains(&main) || !QUICK_HOTKEYS.contains(&quick) {
        bail!("请选择支持的快捷键候选项");
    }
    Ok(())
}
// 纯函数：同时识别 Windows/斜杠路径，空白项直接丢弃。
fn normalize_ignored_app(raw: &str) -> Result<Option<&str>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let name = raw.rsplit(['\\', '/']).next().unwrap().trim();
    if name.len() > 255
        || name.len() <= 4
        || !name.to_ascii_lowercase().ends_with(".exe")
        || name
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
    {
        bail!("忽略程序必须是有效的 exe 文件名，长度不超过 255 字节");
    }
    Ok(Some(name))
}
pub fn normalize_ignored_apps(names: &[String]) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for raw in names {
        if let Some(name) = normalize_ignored_app(raw)? {
            if seen.insert(name.to_lowercase()) {
                normalized.push(name.to_string());
            }
        }
    }
    if normalized.len() > 256 {
        bail!("忽略程序最多 256 个");
    }
    Ok(normalized)
}
impl Settings {
    pub fn retention_policy(&self) -> RetentionPolicy {
        RetentionPolicy {
            max_items: self.max_items,
            max_bytes: self.max_bytes,
            max_days: self.max_days,
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_retention_policy(&self.retention_policy())?;
        let values = serde_json::to_value(self)?;
        for (key, range) in ranges() {
            let value = values[key].as_u64().unwrap();
            if value < range.min || value > range.max {
                bail!(
                    "{}必须在 {} 到 {} 之间",
                    field_label(key),
                    range.min,
                    range.max
                );
            }
        }
        normalize_ignored_apps(&self.ignored_apps)?;
        validate_hotkeys(self.main_hotkey, self.quick_hotkey)
    }
    pub fn normalized(mut self) -> Result<Self> {
        self.ignored_apps = normalize_ignored_apps(&self.ignored_apps)?;
        self.validate()?;
        Ok(self)
    }
    /// 逐项解析与校验；日志不包含配置值。
    pub fn from_rows(rows: &BTreeMap<String, String>, mut warn: impl FnMut(String)) -> Self {
        let defaults = Self::default();
        let mut result = defaults.clone();
        let default_json = serde_json::to_value(&defaults).unwrap();
        for (key, _) in default_json.as_object().unwrap() {
            let parsed = rows
                .get(key)
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            if key == "ignored_apps" {
                if let Some(entries) = parsed.as_ref().and_then(|value| value.as_array()) {
                    let mut seen = HashSet::new();
                    result.ignored_apps.clear();
                    for (index, entry) in entries.iter().enumerate() {
                        match entry.as_str().and_then(|raw| normalize_ignored_app(raw).ok()) {
                            Some(Some(name)) => {
                                if seen.insert(name.to_lowercase()) {
                                    if result.ignored_apps.len() < 256 {
                                        result.ignored_apps.push(name.to_string());
                                    } else {
                                        warn(format!("settings discard key=ignored_apps index={index} reason=too_many"));
                                    }
                                }
                            }
                            Some(None) => {}
                            None => warn(format!("settings discard key=ignored_apps index={index} reason=invalid_entry")),
                        }
                    }
                    continue;
                }
            }
            let candidate = parsed.and_then(|value| {
                let mut json = default_json.clone();
                json[key] = value;
                serde_json::from_value::<Self>(json)
                    .ok()
                    .filter(|s| s.validate().is_ok())
            });
            if let Some(candidate) = candidate {
                let mut json = serde_json::to_value(&result).unwrap();
                json[key] = serde_json::to_value(candidate).unwrap()[key].clone();
                result = serde_json::from_value(json).unwrap();
            } else {
                warn(format!(
                    "settings fallback key={key} reason=missing_or_invalid"
                ));
            }
        }
        if validate_hotkeys(result.main_hotkey, result.quick_hotkey).is_err() {
            result.main_hotkey = defaults.main_hotkey;
            result.quick_hotkey = defaults.quick_hotkey;
            warn("settings fallback key=main_hotkey,quick_hotkey reason=conflict".into());
        }
        result
    }
}
fn field_label(key: &str) -> &str {
    match key {
        "max_items" => "最大条数",
        "max_bytes" => "最大空间（字节）",
        "max_days" => "保留天数",
        "sensitive_ttl_minutes" => "敏感内容保留分钟数",
        "quick_visible_count" => "快速面板可见条数",
        _ => key,
    }
}
#[derive(Serialize)]
pub struct HotkeyOption {
    id: Hotkey,
    label: &'static str,
    hold_label: &'static str,
}
#[derive(Serialize)]
pub struct SettingOptions {
    defaults: Settings,
    ranges: BTreeMap<&'static str, Range>,
    main_hotkeys: Vec<HotkeyOption>,
    quick_hotkeys: Vec<HotkeyOption>,
    pub bytes_per_mb: u64,
    ignored_apps_max: usize,
}
pub fn options() -> SettingOptions {
    let option = |id: Hotkey| HotkeyOption {
        id,
        label: id.label(),
        hold_label: id.hold_label(),
    };
    SettingOptions {
        defaults: Settings::default(),
        ranges: ranges(),
        main_hotkeys: MAIN_HOTKEYS.into_iter().map(option).collect(),
        quick_hotkeys: QUICK_HOTKEYS.into_iter().map(option).collect(),
        bytes_per_mb: MB,
        ignored_apps_max: 256,
    }
}

/// 注册失败恢复旧注册；落库失败也使用此入口反向切换。
pub trait HotkeyRegistrar {
    fn unregister(&mut self) -> Result<()>;
    fn register(&mut self, hotkey: Hotkey) -> Result<()>;
}
pub fn replace_hotkey(
    registrar: &mut impl HotkeyRegistrar,
    current: &mut Option<Hotkey>,
    new: Hotkey,
) -> Result<()> {
    if *current == Some(new) {
        return Ok(());
    }
    let old = *current;
    if old.is_some() {
        registrar.unregister()?;
        *current = None;
    }
    if registrar.register(new).is_ok() {
        *current = Some(new);
        return Ok(());
    }
    if let Some(old) = old {
        if registrar.register(old).is_err() {
            bail!("快捷键已被其他程序占用，旧快捷键恢复失败，请重新保存设置");
        }
        *current = Some(old);
    }
    bail!("快捷键已被其他程序占用")
}

#[cfg(test)]
mod tests {
    use super::*;
    // T3-1：逐项加载的数值边界与类型错误不能污染其他已保存设置。
    #[test]
    fn every_numeric_invalid_row_preserves_other_nondefault_fields() {
        let saved = Settings {
            autostart: true,
            pause_on_start: true,
            max_items: 4321,
            max_bytes: 20 * MB,
            max_days: 17,
            ignored_apps: vec!["custom.exe".into()],
            sensitive_detection: false,
            sensitive_ttl_minutes: 21,
            main_hotkey: Hotkey::CtrlBacktick,
            quick_hotkey: Hotkey::AltShiftV,
            quick_visible_count: 9,
        };
        for (key, range) in ranges() {
            let mut invalid = vec![
                "{broken".into(),
                "\"123\"".into(),
                "true".into(),
                "[]".into(),
                "null".into(),
                "-1".into(),
                "1.5".into(),
                (range.max + 1).to_string(),
                "18446744073709551616".into(),
            ];
            if let Some(value) = range.min.checked_sub(1) {
                invalid.push(value.to_string());
            }
            for bad in invalid {
                let mut data = rows(&saved);
                data.insert(key.into(), bad.clone());
                let mut warnings = Vec::new();
                let actual = Settings::from_rows(&data, |w| warnings.push(w));
                let mut expected = serde_json::to_value(&saved).unwrap();
                expected[key] = serde_json::to_value(Settings::default()).unwrap()[key].clone();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    expected,
                    "{key}={bad}"
                );
                assert_eq!(warnings.len(), 1, "{key}={bad}");
                assert!(warnings[0].contains(&format!("key={key} ")));
                assert!(!warnings[0].contains(&bad), "日志不应包含损坏值");
            }
        }
    }

    // T3-2：用独立的键值表校验全部 49 对候选，区分冲突与候选分组限制。
    #[test]
    fn all_candidate_pairs_have_expected_conflict_and_role_validation() {
        let candidates = [
            (Hotkey::AltV, 0x56, 4),
            (Hotkey::CtrlAltV, 0x56, 5),
            (Hotkey::WinAltV, 0x56, 12),
            (Hotkey::CtrlBacktick, 0xC0, 1),
            (Hotkey::CtrlShiftV, 0x56, 3),
            (Hotkey::CtrlAltShiftV, 0x56, 7),
            (Hotkey::AltShiftV, 0x56, 6),
        ];
        for (i, &(a, key, modifiers)) in candidates.iter().enumerate() {
            assert_eq!(a.shortcut(), Shortcut { key, modifiers });
            for (j, &(b, _, _)) in candidates.iter().enumerate() {
                assert_eq!(shortcuts_conflict(a.shortcut(), b.shortcut()), i == j);
                assert_eq!(validate_hotkeys(a, b).is_ok(), i < 4 && j >= 4);
            }
        }
    }

    #[test]
    fn ignored_apps_normalize_paths_whitespace_and_duplicates() {
        let raw = [
            " App.exe ",
            "App.exe",
            "APP.EXE",
            r" C:\Apps\Other.EXE ",
            "C:/Apps/other.exe",
            "",
            " ",
            "中文程序.exe",
            "Äpp.exe",
            "äpp.EXE",
        ]
        .map(String::from)
        .to_vec();
        let expected = vec!["App.exe", "Other.EXE", "中文程序.exe", "Äpp.exe"];
        assert_eq!(normalize_ignored_apps(&raw).unwrap(), expected);
        let saved = Settings {
            ignored_apps: raw,
            ..Default::default()
        };
        saved.validate().unwrap();
        let normalized = saved.clone().normalized().unwrap();
        assert_eq!(normalized.ignored_apps, expected);
        assert_eq!(normalized.clone().normalized().unwrap(), normalized);
        assert_eq!(
            Settings::from_rows(&rows(&saved), |_| panic!("有效配置")),
            normalized
        );
        let empty = Settings {
            ignored_apps: vec![" ".into()],
            ..Default::default()
        };
        assert!(empty.clone().normalized().unwrap().ignored_apps.is_empty());
        assert_eq!(
            Settings::from_rows(&rows(&empty), |_| panic!("空列表有效")),
            empty.normalized().unwrap()
        );
    }

    #[test]
    fn ignored_apps_invalid_entry_is_discarded_without_losing_valid_names() {
        for name in [
            "bad.dll", "a?.exe", "a*.exe", "a:.exe", "a\".exe", "a<.exe", "a>.exe", "a|.exe",
            "a\n.exe", ".exe", "C:/Apps/",
        ] {
            let saved = Settings {
                autostart: true,
                ignored_apps: vec!["valid.exe".into(), name.into(), "Other.EXE".into()],
                ..Default::default()
            };
            assert!(saved.validate().is_err(), "{name}");
            assert!(
                normalize_ignored_apps(&saved.ignored_apps).is_err(),
                "{name}"
            );
            let mut warnings = Vec::new();
            let actual = Settings::from_rows(&rows(&saved), |w| warnings.push(w));
            assert_eq!(
                actual,
                Settings {
                    ignored_apps: vec!["valid.exe".into(), "Other.EXE".into()],
                    ..saved
                }
            );
            assert_eq!(
                warnings,
                ["settings discard key=ignored_apps index=1 reason=invalid_entry"]
            );
        }
    }

    #[test]
    fn ignored_apps_loading_skips_bad_types_and_keeps_empty_result() {
        let mut data = rows(&Settings::default());
        data.insert(
            "ignored_apps".into(),
            r#"["Good.exe",42,null,"bad.dll","GOOD.EXE"," "]"#.into(),
        );
        let mut warnings = Vec::new();
        let actual = Settings::from_rows(&data, |w| warnings.push(w));
        assert_eq!(actual.ignored_apps, ["Good.exe"]);
        assert_eq!(warnings.len(), 3);
        data.insert("ignored_apps".into(), r#"["bad.dll"]"#.into());
        assert!(Settings::from_rows(&data, |_| {}).ignored_apps.is_empty());
    }

    #[test]
    fn unregister_failure_can_retry_without_losing_old_hotkey() {
        let mut fake = Fake {
            unregister_fails: true,
            ..Default::default()
        };
        let mut current = Some(Hotkey::AltV);
        assert!(replace_hotkey(&mut fake, &mut current, Hotkey::CtrlAltV).is_err());
        assert_eq!(current, Some(Hotkey::AltV));
        assert_eq!(fake.calls, [None]);
        fake.unregister_fails = false;
        replace_hotkey(&mut fake, &mut current, Hotkey::CtrlAltV).unwrap();
        assert_eq!(current, Some(Hotkey::CtrlAltV));
        assert_eq!(fake.calls, [None, None, Some(Hotkey::CtrlAltV)]);
    }

    fn rows(settings: &Settings) -> BTreeMap<String, String> {
        serde_json::to_value(settings)
            .unwrap()
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string()))
            .collect()
    }
    #[test]
    fn defaults_and_options_agree() {
        let s = Settings::default();
        s.validate().unwrap();
        assert_eq!(s.retention_policy(), RetentionPolicy::default());
        assert!(!s.autostart && !s.pause_on_start && s.sensitive_detection);
        assert_eq!(s.ignored_apps.len(), 4);
        assert_eq!((s.sensitive_ttl_minutes, s.quick_visible_count), (10, 5));
        assert_eq!(options().defaults, s);
    }
    #[test]
    fn serde_roundtrip() {
        let s = Settings {
            autostart: true,
            quick_hotkey: Hotkey::AltShiftV,
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_str::<Settings>(&serde_json::to_string(&s).unwrap()).unwrap(),
            s
        );
    }
    #[test]
    fn each_missing_field_falls_back_independently() {
        let saved = Settings {
            max_items: 4321,
            autostart: true,
            ..Default::default()
        };
        for key in rows(&saved).keys() {
            let mut data = rows(&saved);
            data.remove(key);
            let mut warnings = Vec::new();
            let actual = Settings::from_rows(&data, |w| warnings.push(w));
            assert_eq!(warnings.len(), 1, "{key}");
            let mut expected = serde_json::to_value(&saved).unwrap();
            expected[key] = serde_json::to_value(Settings::default()).unwrap()[key].clone();
            assert_eq!(serde_json::to_value(actual).unwrap(), expected);
        }
    }
    #[test]
    fn each_damaged_field_falls_back_independently() {
        let saved = Settings {
            max_items: 4321,
            autostart: true,
            ..Default::default()
        };
        for key in rows(&saved).keys() {
            for bad in ["{broken", "null", "{}"] {
                let mut data = rows(&saved);
                data.insert(key.clone(), bad.into());
                let mut warnings = Vec::new();
                let actual = Settings::from_rows(&data, |w| warnings.push(w));
                assert_eq!(warnings.len(), 1, "{key}");
                let mut expected = serde_json::to_value(&saved).unwrap();
                expected[key] = serde_json::to_value(Settings::default()).unwrap()[key].clone();
                assert_eq!(serde_json::to_value(actual).unwrap(), expected);
            }
        }
    }
    #[test]
    fn unknown_key_is_ignored() {
        let mut data = rows(&Settings::default());
        data.insert("future".into(), "broken json".into());
        assert_eq!(
            Settings::from_rows(&data, |_| panic!("unknown key is ignored")),
            Settings::default()
        );
    }
    #[test]
    fn all_numeric_boundaries() {
        for (key, r) in ranges() {
            for value in [r.min, r.max] {
                let mut json = serde_json::to_value(Settings::default()).unwrap();
                json[key] = value.into();
                serde_json::from_value::<Settings>(json)
                    .unwrap()
                    .validate()
                    .unwrap();
            }
            for value in [r.min.checked_sub(1), Some(r.max + 1)]
                .into_iter()
                .flatten()
            {
                let mut json = serde_json::to_value(Settings::default()).unwrap();
                json[key] = value.into();
                assert!(
                    serde_json::from_value::<Settings>(json)
                        .unwrap()
                        .validate()
                        .is_err(),
                    "{key}={value}"
                );
            }
        }
    }
    #[test]
    fn out_of_range_fallback_does_not_reset_other_fields() {
        let mut data = rows(&Settings {
            max_items: 1234,
            ..Default::default()
        });
        data.insert("max_days".into(), "3651".into());
        let loaded = Settings::from_rows(&data, |_| {});
        assert_eq!(loaded.max_days, 30);
        assert_eq!(loaded.max_items, 1234);
    }
    #[test]
    fn different_triggers_with_identical_modifiers_conflict() {
        assert!(shortcuts_conflict(
            Shortcut {
                key: 0x56,
                modifiers: 3
            },
            Shortcut {
                key: 0x42,
                modifiers: 3
            }
        ));
        assert!(!shortcuts_conflict(
            Shortcut {
                key: 0x56,
                modifiers: 3
            },
            Shortcut {
                key: 0x56,
                modifiers: 7
            }
        ));
    }
    #[test]
    fn candidate_matrix_and_conflicts() {
        for main in MAIN_HOTKEYS {
            for quick in QUICK_HOTKEYS {
                validate_hotkeys(main, quick).unwrap();
            }
        }
        for key in MAIN_HOTKEYS.into_iter().chain(QUICK_HOTKEYS) {
            assert!(validate_hotkeys(key, key).is_err());
        }
        assert!(validate_hotkeys(Hotkey::CtrlShiftV, Hotkey::AltV).is_err());
        assert_eq!(MAIN_HOTKEYS.len(), 4);
        assert_eq!(QUICK_HOTKEYS.len(), 3);
    }
    #[test]
    fn ignored_names_validate() {
        for name in ["a", ".exe", "a.dll", "a\n.exe", "a?.exe"] {
            assert!(
                Settings {
                    ignored_apps: vec![name.into()],
                    ..Default::default()
                }
                .validate()
                .is_err(),
                "{name}"
            );
        }
        Settings {
            ignored_apps: vec!["中文程序.EXE".into(), "a b.exe".into()],
            ..Default::default()
        }
        .validate()
        .unwrap();
        assert!(Settings {
            ignored_apps: (0..257).map(|i| format!("app{i}.exe")).collect(),
            ..Default::default()
        }
        .validate()
        .is_err());
        assert_eq!(
            normalize_ignored_apps(&vec!["a.exe".into(); 257]).unwrap(),
            ["a.exe"]
        );
        assert!(normalize_ignored_apps(&[format!("{}.exe", "a".repeat(252))]).is_err());
        Settings {
            ignored_apps: vec![],
            ..Default::default()
        }
        .validate()
        .unwrap();
    }
    #[derive(Default)]
    struct Fake {
        calls: Vec<Option<Hotkey>>,
        fail: Vec<Hotkey>,
        unregister_fails: bool,
    }
    impl HotkeyRegistrar for Fake {
        fn unregister(&mut self) -> Result<()> {
            self.calls.push(None);
            if self.unregister_fails {
                bail!("注销失败");
            }
            Ok(())
        }
        fn register(&mut self, key: Hotkey) -> Result<()> {
            self.calls.push(Some(key));
            if self.fail.contains(&key) {
                bail!("占用");
            }
            Ok(())
        }
    }
    #[test]
    fn registration_failure_restores_old_hotkey() {
        let mut fake = Fake {
            fail: vec![Hotkey::CtrlAltV],
            ..Default::default()
        };
        let mut current = Some(Hotkey::AltV);
        assert_eq!(
            replace_hotkey(&mut fake, &mut current, Hotkey::CtrlAltV)
                .unwrap_err()
                .to_string(),
            "快捷键已被其他程序占用"
        );
        assert_eq!(
            fake.calls,
            vec![None, Some(Hotkey::CtrlAltV), Some(Hotkey::AltV)]
        );
        assert_eq!(current, Some(Hotkey::AltV));
    }
    #[test]
    fn registration_success_unregisters_first() {
        let mut fake = Fake::default();
        let mut current = Some(Hotkey::AltV);
        replace_hotkey(&mut fake, &mut current, Hotkey::CtrlBacktick).unwrap();
        assert_eq!(fake.calls, vec![None, Some(Hotkey::CtrlBacktick)]);
        assert_eq!(current, Some(Hotkey::CtrlBacktick));
    }
    #[test]
    fn rollback_failure_is_explicit_and_retryable() {
        let mut fake = Fake {
            fail: vec![Hotkey::CtrlAltV, Hotkey::AltV],
            ..Default::default()
        };
        let mut current = Some(Hotkey::AltV);
        assert!(replace_hotkey(&mut fake, &mut current, Hotkey::CtrlAltV)
            .unwrap_err()
            .to_string()
            .contains("恢复失败"));
        assert_eq!(current, None);
        fake.fail.clear();
        replace_hotkey(&mut fake, &mut current, Hotkey::AltV).unwrap();
        assert_eq!(current, Some(Hotkey::AltV));
    }
    #[test]
    fn startup_registration_failure_does_not_unregister() {
        let mut fake = Fake {
            fail: vec![Hotkey::AltV],
            ..Default::default()
        };
        let mut current = None;
        assert!(replace_hotkey(&mut fake, &mut current, Hotkey::AltV).is_err());
        assert_eq!(fake.calls, vec![Some(Hotkey::AltV)]);
        assert_eq!(current, None);
    }
    #[test]
    fn unchanged_hotkey_does_not_reregister() {
        let mut fake = Fake::default();
        let mut current = Some(Hotkey::AltV);
        replace_hotkey(&mut fake, &mut current, Hotkey::AltV).unwrap();
        assert!(fake.calls.is_empty());
    }
    #[test]
    fn unregister_failure_keeps_current_registration() {
        let mut fake = Fake {
            unregister_fails: true,
            ..Default::default()
        };
        let mut current = Some(Hotkey::AltV);
        assert!(replace_hotkey(&mut fake, &mut current, Hotkey::CtrlAltV).is_err());
        assert_eq!(current, Some(Hotkey::AltV));
        assert_eq!(fake.calls, vec![None]);
    }
}
