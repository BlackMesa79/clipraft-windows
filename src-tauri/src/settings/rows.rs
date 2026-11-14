use super::{normalize_ignored_app, validate_hotkeys, Settings, QUICK_HOTKEYS};
use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};

impl Settings {
    /// 无效用户字段逐项回退；内部序列化失败返回错误，日志不包含配置值。
    pub fn from_rows(
        rows: &BTreeMap<String, String>,
        mut warn: impl FnMut(String),
    ) -> Result<Self> {
        let defaults = Self::default();
        let default_json = serde_json::to_value(&defaults)?;
        let fields = default_json.as_object().context("默认设置不是 JSON 对象")?;
        let mut merged = fields.clone();
        for key in fields.keys() {
            let parsed = rows
                .get(key)
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
            merge_field(&mut merged, key, parsed, &mut warn)?;
        }
        let mut result: Self = serde_json::from_value(Value::Object(merged))?;
        repair_hotkeys(&mut result, rows, &mut warn)?;
        Ok(result)
    }
}

fn merge_field(
    merged: &mut Map<String, Value>,
    key: &str,
    parsed: Option<Value>,
    warn: &mut impl FnMut(String),
) -> Result<()> {
    if key == "ignored_apps" {
        if let Some(entries) = parsed.as_ref().and_then(Value::as_array) {
            merged.insert(
                key.into(),
                serde_json::to_value(ignored_apps(entries, warn))?,
            );
            return Ok(());
        }
    }
    let Some(candidate) = valid_field(key, parsed)? else {
        warn(format!(
            "settings fallback key={key} reason=missing_or_invalid"
        ));
        return Ok(());
    };
    merged.insert(key.into(), candidate);
    Ok(())
}

fn valid_field(key: &str, value: Option<Value>) -> Result<Option<Value>> {
    let Some(value) = value else { return Ok(None) };
    // 用完整默认配置校验单个字段，避免其他已加载字段的关联约束干扰回退。
    let mut json = serde_json::to_value(Settings::default())?;
    json.as_object_mut()
        .context("默认设置不是 JSON 对象")?
        .insert(key.into(), value.clone());
    let candidate = serde_json::from_value::<Settings>(json)
        .ok()
        .filter(|settings| settings.validate_fields().is_ok());
    Ok(candidate.map(|_| value))
}

fn ignored_apps(entries: &[Value], warn: &mut impl FnMut(String)) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let parsed = entry
            .as_str()
            .and_then(|raw| normalize_ignored_app(raw).ok());
        let Some(name) = parsed else {
            warn(format!(
                "settings discard key=ignored_apps index={index} reason=invalid_entry"
            ));
            continue;
        };
        let Some(name) = name else { continue };
        if !seen.insert(name.to_lowercase()) {
            continue;
        }
        if names.len() >= 256 {
            warn(format!(
                "settings discard key=ignored_apps index={index} reason=too_many"
            ));
            continue;
        }
        names.push(name.to_string());
    }
    names
}

fn repair_hotkeys(
    result: &mut Settings,
    rows: &BTreeMap<String, String>,
    warn: &mut impl FnMut(String),
) -> Result<()> {
    if validate_hotkeys(result.main_hotkey, result.quick_hotkey).is_ok() {
        return Ok(());
    }
    let unified = rows
        .get("unified_hotkey")
        .and_then(|raw| serde_json::from_str::<bool>(raw).ok())
        == Some(true);
    if unified {
        result.quick_hotkey = QUICK_HOTKEYS
            .into_iter()
            .find(|key| validate_hotkeys(result.main_hotkey, *key).is_ok())
            .context("没有可用的快速面板替代热键")?;
        warn("settings fallback key=quick_hotkey reason=removed_unified_mode".into());
        return Ok(());
    }
    let defaults = Settings::default();
    result.main_hotkey = defaults.main_hotkey;
    result.quick_hotkey = defaults.quick_hotkey;
    warn("settings fallback key=main_hotkey,quick_hotkey reason=conflict".into());
    Ok(())
}
