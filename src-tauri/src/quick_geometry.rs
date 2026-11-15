//! 与前端共用几何源，偶数条数预留对称舞台，中心卡片不偏移。
use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Geometry {
    width: f64,
    card_height: f64,
    pitch: f64,
    padding: f64,
}

pub fn dimensions(count: u32) -> Result<(f64, f64)> {
    let geometry: Geometry = serde_json::from_str(include_str!("../../src/quick-geometry.json"))?;
    let height =
        geometry.card_height + geometry.padding + f64::from(count / 2) * geometry.pitch * 2.0;
    Ok((geometry.width, height))
}

/// 模式与会话都匹配才允许调整尺寸，避免旧提示或旧会话的迟到请求。
pub fn accepts_resize(
    visible: bool,
    error: bool,
    session: u64,
    notice: bool,
    requested: u64,
) -> bool {
    session == requested && notice == (!visible && error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_geometry_keeps_center_and_legacy_even_counts() {
        for (count, height) in [(3, 256.0), (4, 408.0), (5, 408.0), (7, 560.0), (15, 1168.0)] {
            assert_eq!(dimensions(count).unwrap(), (420.0, height));
        }
    }

    #[test]
    fn stale_notice_and_session_cannot_resize_new_wheel() {
        assert!(accepts_resize(true, false, 2, false, 2));
        assert!(!accepts_resize(true, false, 2, true, 2));
        assert!(!accepts_resize(true, false, 2, false, 1));
        assert!(accepts_resize(false, true, 2, true, 2));
        assert!(!accepts_resize(false, true, 2, false, 2));
    }
}
