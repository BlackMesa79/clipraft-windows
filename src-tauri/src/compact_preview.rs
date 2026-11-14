use crate::{model::ItemView, runtime::State, ItemDetail};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager, WebviewWindow};

#[derive(Clone, serde::Serialize)]
pub struct Content {
    pub generation: u64,
    pub item: ItemView,
    pub detail: ItemDetail,
}
#[derive(Default)]
pub struct PreviewState {
    generation: AtomicU64,
    content: Mutex<Option<Content>>,
    anchor: Mutex<Option<Anchor>>,
    geometry: Mutex<Option<Geometry>>,
    height: Mutex<Option<f64>>,
}
impl PreviewState {
    pub fn has_content(&self) -> bool {
        self.content.lock().unwrap().is_some()
    }
    fn clear(&self) -> u64 {
        let mut content = self.content.lock().unwrap();
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *content = None;
        // 保留最后有效高度作为下一条内容的初始尺寸，避免先跳回固定高度。
        generation
    }
    fn report_height(&self, generation: u64, height: f64) -> bool {
        if !height.is_finite() {
            return false;
        }
        let height = height.clamp(120.0, 640.0);
        let content = self.content.lock().unwrap();
        if self.generation.load(Ordering::Acquire) != generation
            || content.as_ref().is_none_or(|c| c.generation != generation)
        {
            return false;
        }
        let mut current = self.height.lock().unwrap();
        if current.is_some_and(|old| (old - height).abs() < 1.0) {
            return false;
        }
        *current = Some(height);
        true
    }
    fn publish(&self, generation: u64, value: Content, show: impl FnOnce(Content)) -> bool {
        {
            let mut content = self.content.lock().unwrap();
            if self.generation.load(Ordering::Acquire) != generation {
                return false;
            }
            *content = Some(value.clone());
        }
        // Win32 窗口 API 可同步重入事件处理；外部回调绝不能占用快照锁。
        if self.generation.load(Ordering::Acquire) != generation {
            return false;
        }
        show(value);
        self.generation.load(Ordering::Acquire) == generation
    }
}

// 只传可见行几何，不传正文或文件载荷；坐标相对主窗口客户区。
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Anchor {
    pub id: String,
    pub top: f64,
    pub bottom: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Geometry {
    pub right: bool,
    pub arrow: f64,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

#[tauri::command]
pub fn position_compact_preview(
    anchor: Option<Anchor>,
    window: WebviewWindow,
    state: tauri::State<'_, PreviewState>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("仅主窗口可定位浮窗".into());
    }
    let anchor = anchor
        .filter(|a| a.top.is_finite() && a.bottom.is_finite() && a.bottom > a.top && a.top >= 0.0);
    *state.anchor.lock().unwrap() = anchor;
    reposition(&window)
        .map(|_| ())
        .map_err(|_| "浮窗定位失败".into())
}
#[tauri::command]
pub fn resize_compact_preview(
    generation: u64,
    height: f64,
    window: WebviewWindow,
    state: tauri::State<'_, PreviewState>,
) -> Result<(), String> {
    if window.label() != "preview" {
        return Err("仅浮窗可报告内容高度".into());
    }
    if !height.is_finite() {
        return Err("预览高度无效".into());
    }
    if !state.report_height(generation, height) {
        return Ok(());
    }
    if let Some(main) = window.app_handle().get_webview_window("main") {
        reposition(&main).map_err(|_| "预览高度调整失败")?;
    }
    Ok(())
}
#[tauri::command]
pub fn compact_preview_geometry(
    window: WebviewWindow,
    state: tauri::State<'_, PreviewState>,
) -> Result<Option<Geometry>, String> {
    if window.label() != "preview" {
        return Err("仅浮窗可读取几何".into());
    }
    Ok(*state.geometry.lock().unwrap())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

// 坐标均为物理像素；优先右侧、其次左侧，空间不足时缩小或隐藏，绝不覆盖主窗。
pub fn placement(main: Rect, work: Rect, scale: f64, height: f64) -> Option<Rect> {
    let scale = scale.clamp(0.5, 4.0);
    let gap = (10.0 * scale).round() as i64;
    let desired = (360.0 * scale).round() as i64;
    let minimum = (180.0 * scale).round() as i64;
    let left = (i64::from(main.x) - i64::from(work.x) - gap).max(0);
    let right = (i64::from(work.x) + i64::from(work.width)
        - i64::from(main.x)
        - i64::from(main.width)
        - gap)
        .max(0);
    let on_right = right >= desired || (left < desired && right >= left);
    let width = desired.min(if on_right { right } else { left });
    let maximum = (f64::from(work.height) / scale * 0.8).min(640.0);
    let height = (height.clamp(120.0_f64.min(maximum), maximum) * scale).round() as i64;
    if width < minimum || height < 100 {
        return None;
    }
    let x = if on_right {
        i64::from(main.x) + i64::from(main.width) + gap
    } else {
        i64::from(main.x) - width - gap
    };
    let y = (i64::from(main.y) + (44.0 * scale).round() as i64).clamp(
        i64::from(work.y),
        i64::from(work.y) + i64::from(work.height) - height,
    );
    if x < i64::from(work.x) || x + width > i64::from(work.x) + i64::from(work.width) {
        return None;
    }
    Some(Rect {
        x: x as i32,
        y: y as i32,
        width: width as u32,
        height: height as u32,
    })
}

pub fn hide(app: &tauri::AppHandle) -> u64 {
    let mut generation = 0;
    if let Some(state) = app.try_state::<PreviewState>() {
        generation = state.clear();
    }
    if let Some(window) = app.get_webview_window("preview") {
        if window.is_visible().unwrap_or(false) {
            if let Ok(handle) = window.hwnd() {
                crate::platform::hide(handle.0 as isize);
            }
        }
        let _ = window.emit("compact-preview-changed", Option::<Content>::None);
    }
    generation
}

pub fn reposition(main: &WebviewWindow) -> tauri::Result<bool> {
    let Some(panel) = main.app_handle().get_webview_window("preview") else {
        return Ok(false);
    };
    let state = main.state::<PreviewState>();
    let anchor = state.anchor.lock().unwrap().clone();
    let content_id = state
        .content
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.item.id.clone());
    // 内容切换期间保持原生显隐，避免选中一次就 hide/show；迟到几何不指向旧记录。
    let Some(content_id) = content_id else {
        return Ok(false);
    };
    let Some(anchor) = anchor else {
        if panel.is_visible().unwrap_or(false) {
            crate::platform::hide(panel.hwnd()?.0 as isize);
        }
        return Ok(false);
    };
    if anchor.id != content_id
        || !main.is_visible()?
        || main.is_minimized()?
        || !crate::platform::main_has_foreground(main.hwnd()?.0 as isize)
    {
        return Ok(false);
    }
    let Some(monitor) = main.current_monitor()? else {
        return Ok(false);
    };
    let pos = main.outer_position()?;
    let size = main.outer_size()?;
    let area = monitor.work_area();
    let bounds = placement(
        Rect {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
        },
        Rect {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        main.scale_factor()?,
        state.height.lock().unwrap().unwrap_or(180.0),
    );
    let Some(bounds) = bounds else {
        crate::platform::hide(panel.hwnd()?.0 as isize);
        return Ok(false);
    };
    let scale = main.scale_factor()?;
    let inner = main.inner_position()?;
    let center = f64::from(inner.y) + (anchor.top + anchor.bottom) * scale / 2.0;
    // 移动浮窗使箭头接近目标；工作区边缘仍保留圆角安全距离。
    let mut bounds = bounds;
    bounds.y = (center - 60.0 * scale).round().clamp(
        f64::from(area.position.y),
        f64::from(area.position.y) + f64::from(area.size.height) - f64::from(bounds.height),
    ) as i32;
    let geometry = Geometry {
        right: bounds.x > pos.x,
        arrow: ((center - f64::from(bounds.y)) / scale)
            .clamp(26.0, f64::from(bounds.height) / scale - 26.0),
        width: bounds.width,
        height: bounds.height,
        scale,
    };
    let changed = *state.geometry.lock().unwrap() != Some(geometry);
    if changed {
        *state.geometry.lock().unwrap() = Some(geometry);
        let _ = panel.emit("compact-preview-geometry", geometry);
    }
    crate::platform::position_preview(
        panel.hwnd()?.0 as isize,
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
    )
    .map_err(tauri::Error::Anyhow)?;
    Ok(true)
}

#[tauri::command]
pub fn compact_preview_state(
    window: WebviewWindow,
    state: tauri::State<'_, PreviewState>,
) -> Result<Option<Content>, String> {
    if window.label() != "preview" {
        return Err("仅预览窗口可读取浮窗快照".into());
    }
    Ok(state.content.lock().unwrap().clone())
}

#[tauri::command]
pub async fn sync_compact_preview(
    id: Option<String>,
    window: WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
    _preview: tauri::State<'_, PreviewState>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("仅主窗口可更新浮动预览".into());
    }
    let Some(id) = id else {
        hide(window.app_handle());
        return Ok(());
    };
    // 内容切换只清空快照，不对原生窗口做 hide/show，避免焦点反馈环。
    let generation = _preview.clear();
    if let Some(panel) = window.app_handle().get_webview_window("preview") {
        let _ = panel.emit("compact-preview-changed", Option::<Content>::None);
    }
    if !state.settings.lock().unwrap().compact {
        return Ok(());
    }
    let shared = state.inner().clone();
    let content = tauri::async_runtime::spawn_blocking(move || {
        let storage = shared
            .storage
            .lock()
            .map_err(|_| "存储不可用".to_string())?;
        let item = storage
            .preview_item(&id)
            .map_err(|_| "预览记录读取失败".to_string())?;
        let detail = storage
            .display_detail(&id, false)
            .map_err(|_| "预览内容读取失败".to_string())?;
        Ok::<_, String>(Content {
            generation,
            item,
            detail,
        })
    })
    .await
    .map_err(|_| "预览任务失败".to_string())??;
    let main = window.clone();
    window
        .run_on_main_thread(move || {
            let preview = main.state::<PreviewState>();
            let state = main.state::<Arc<State>>();
            if preview.generation.load(Ordering::Acquire) != generation
                || !state.settings.lock().unwrap().compact
                || !main.is_visible().unwrap_or(false)
                || main.is_minimized().unwrap_or(true)
                || !crate::platform::main_has_foreground(state.main.load(Ordering::Acquire))
            {
                return;
            }
            if let Some(panel) = main.app_handle().get_webview_window("preview") {
                preview.publish(generation, content, |content| {
                    let _ = panel.emit("compact-preview-changed", Some(content));
                    let _ = reposition(&main);
                });
            }
        })
        .map_err(|_| "预览窗口更新失败".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_callback_can_reenter_snapshot_without_deadlock() {
        let state = PreviewState::default();
        let generation = state.clear();
        let content = Content {
            generation,
            item: ItemView::default(),
            detail: ItemDetail {
                text: String::new(),
                truncated: false,
                files: vec![],
            },
        };
        assert!(!state.publish(generation, content, |_| {
            assert!(state.has_content());
            state.clear();
        }));
        assert!(!state.has_content());
    }
    const WORK: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };
    #[test]
    fn late_content_cannot_reopen_after_hide_or_new_selection() {
        let state = PreviewState::default();
        let content = || Content {
            generation: state.generation.load(Ordering::Acquire),
            item: ItemView::default(),
            detail: ItemDetail {
                text: "synthetic".into(),
                truncated: false,
                files: vec![],
            },
        };
        let old = state.clear();
        let new = state.clear();
        assert!(!state.publish(old, content(), |_| panic!("旧选择不能显示")));
        assert!(state.publish(new, content(), |_| {}));
        assert!(state.has_content());
        state.clear();
        assert!(!state.publish(new, content(), |_| panic!("隐藏后不能重新显示")));
        assert!(!state.has_content());
    }
    #[test]
    fn content_height_respects_minimum_maximum_and_monitor_limit() {
        let main = Rect {
            x: 100,
            y: 20,
            width: 390,
            height: 700,
        };
        for scale in [1.0, 1.5, 2.0] {
            let small = placement(main, WORK, scale, 20.0).unwrap();
            assert_eq!(small.height, (120.0 * scale).round() as u32);
            let large = placement(main, WORK, scale, 5000.0).unwrap();
            assert_eq!(
                large.height,
                (640.0 * scale).min(f64::from(WORK.height) * 0.8).round() as u32
            );
            assert!(
                large.y >= WORK.y
                    && i64::from(large.y) + i64::from(large.height) <= i64::from(WORK.height)
            );
        }
    }
    #[test]
    fn stale_and_duplicate_height_reports_cannot_resize_new_or_hidden_content() {
        let state = PreviewState::default();
        let content = |generation| Content {
            generation,
            item: ItemView::default(),
            detail: ItemDetail {
                text: String::new(),
                truncated: false,
                files: vec![],
            },
        };
        let old = state.clear();
        state.publish(old, content(old), |_| {});
        assert!(state.report_height(old, 180.0));
        assert!(!state.report_height(old, 180.2));
        assert!(!state.report_height(old, f64::NAN));
        let new = state.clear();
        state.publish(new, content(new), |_| {});
        assert!(!state.report_height(old, 640.0));
        assert_eq!(*state.height.lock().unwrap(), Some(180.0));
        assert!(state.report_height(new, 9000.0));
        assert_eq!(*state.height.lock().unwrap(), Some(640.0));
        state.clear();
        assert!(!state.report_height(new, 120.0));
    }
    #[test]
    fn right_then_left_and_bottom_clamp() {
        let main = Rect {
            x: 100,
            y: 900,
            width: 390,
            height: 700,
        };
        let right = placement(main, WORK, 1.0, 420.0).unwrap();
        assert_eq!(
            right,
            Rect {
                x: 500,
                y: 620,
                width: 360,
                height: 420
            }
        );
        let left = placement(Rect { x: 1500, ..main }, WORK, 1.0, 420.0).unwrap();
        assert_eq!(left.x, 1130);
    }
    #[test]
    fn negative_monitor_and_high_dpi() {
        let work = Rect {
            x: -2560,
            y: -100,
            width: 2560,
            height: 1400,
        };
        let main = Rect {
            x: -800,
            y: -100,
            width: 585,
            height: 1000,
        };
        let panel = placement(main, work, 1.5, 420.0).unwrap();
        assert_eq!(
            panel,
            Rect {
                x: -1355,
                y: -34,
                width: 540,
                height: 630
            }
        );
    }
    #[test]
    fn narrow_space_shrinks_or_hides_without_overlap() {
        let main = Rect {
            x: 0,
            y: 0,
            width: 390,
            height: 700,
        };
        assert_eq!(
            placement(main, Rect { width: 650, ..WORK }, 1.0, 420.0)
                .unwrap()
                .width,
            250
        );
        assert!(placement(main, Rect { width: 500, ..WORK }, 1.0, 420.0).is_none());
    }
}
