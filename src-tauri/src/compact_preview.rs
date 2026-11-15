use crate::{item_detail::ItemDetail, model::ItemView, runtime::State};
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
    pub fn has_content(&self) -> tauri::Result<bool> {
        Ok(self
            .content
            .lock()
            .map_err(|_| state_error("内容"))?
            .is_some())
    }
    fn clear(&self) -> tauri::Result<u64> {
        let mut content = self.content.lock().map_err(|_| state_error("内容"))?;
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *content = None;
        // 保留最后有效高度作为下一条内容的初始尺寸，避免先跳回固定高度。
        Ok(generation)
    }
    fn report_height(&self, generation: u64, height: f64) -> tauri::Result<bool> {
        if !height.is_finite() {
            return Ok(false);
        }
        let height = height.clamp(120.0, 640.0);
        let content = self.content.lock().map_err(|_| state_error("内容"))?;
        if self.generation.load(Ordering::Acquire) != generation
            || content.as_ref().is_none_or(|c| c.generation != generation)
        {
            return Ok(false);
        }
        let mut current = self.height.lock().map_err(|_| state_error("高度"))?;
        if current.is_some_and(|old| (old - height).abs() < 1.0) {
            return Ok(false);
        }
        *current = Some(height);
        Ok(true)
    }
    fn publish(
        &self,
        generation: u64,
        value: Content,
        show: impl FnOnce(Content),
    ) -> tauri::Result<bool> {
        {
            let mut content = self.content.lock().map_err(|_| state_error("内容"))?;
            if self.generation.load(Ordering::Acquire) != generation {
                return Ok(false);
            }
            *content = Some(value.clone());
        }
        // Win32 窗口 API 可同步重入事件处理；外部回调绝不能占用快照锁。
        if self.generation.load(Ordering::Acquire) != generation {
            return Ok(false);
        }
        show(value);
        Ok(self.generation.load(Ordering::Acquire) == generation)
    }

    fn update_geometry(&self, geometry: Geometry) -> tauri::Result<bool> {
        let mut current = self.geometry.lock().map_err(|_| state_error("几何"))?;
        if *current == Some(geometry) {
            return Ok(false);
        }
        *current = Some(geometry);
        Ok(true)
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
    *state.anchor.lock().map_err(|_| "浮窗锚点状态不可用")? = anchor;
    reposition(&window)
        .map(|_| ())
        .map_err(|error| format!("浮窗定位失败：{error}"))
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
    if !state
        .report_height(generation, height)
        .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    if let Some(main) = window.app_handle().get_webview_window("main") {
        reposition(&main).map_err(|error| format!("预览高度调整失败：{error}"))?;
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
    Ok(*state.geometry.lock().map_err(|_| "浮窗几何状态不可用")?)
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
        match state.clear() {
            Ok(next) => generation = next,
            Err(error) => crate::diag::write(format!("preview clear failed: {error}")),
        }
    }
    let Some(window) = app.get_webview_window("preview") else {
        return generation;
    };
    if window.is_visible().unwrap_or(false) {
        if let Ok(handle) = window.hwnd() {
            crate::platform::hide(handle.0 as isize);
        }
    }
    let _ = window.emit("compact-preview-changed", Option::<Content>::None);
    generation
}

struct Layout {
    main: Rect,
    work: Rect,
    scale: f64,
    inner_y: i32,
}

fn preview_layout(main: &WebviewWindow) -> tauri::Result<Option<Layout>> {
    let Some(monitor) = main.current_monitor()? else {
        return Ok(None);
    };
    let pos = main.outer_position()?;
    let size = main.outer_size()?;
    let area = monitor.work_area();
    Ok(Some(Layout {
        main: Rect {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
        },
        work: Rect {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        scale: main.scale_factor()?,
        inner_y: main.inner_position()?.y,
    }))
}

impl Layout {
    fn anchored(&self, anchor: &Anchor, height: f64) -> Option<(Rect, Geometry)> {
        let mut bounds = placement(self.main, self.work, self.scale, height)?;
        let center = f64::from(self.inner_y) + (anchor.top + anchor.bottom) * self.scale / 2.0;
        // 移动浮窗使箭头接近目标；工作区边缘仍保留圆角安全距离。
        bounds.y = (center - 60.0 * self.scale).round().clamp(
            f64::from(self.work.y),
            f64::from(self.work.y) + f64::from(self.work.height) - f64::from(bounds.height),
        ) as i32;
        let geometry = Geometry {
            right: bounds.x > self.main.x,
            arrow: ((center - f64::from(bounds.y)) / self.scale)
                .clamp(26.0, f64::from(bounds.height) / self.scale - 26.0),
            width: bounds.width,
            height: bounds.height,
            scale: self.scale,
        };
        Some((bounds, geometry))
    }
}

pub fn reposition(main: &WebviewWindow) -> tauri::Result<bool> {
    let Some(panel) = main.app_handle().get_webview_window("preview") else {
        return Ok(false);
    };
    let state = main.state::<PreviewState>();
    let anchor = state
        .anchor
        .lock()
        .map_err(|_| state_error("锚点"))?
        .clone();
    let content_id = state
        .content
        .lock()
        .map_err(|_| state_error("内容"))?
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
    apply_layout(main, &panel, &state, &anchor)
}

fn apply_layout(
    main: &WebviewWindow,
    panel: &WebviewWindow,
    state: &PreviewState,
    anchor: &Anchor,
) -> tauri::Result<bool> {
    let Some(layout) = preview_layout(main)? else {
        return Ok(false);
    };
    let height = state
        .height
        .lock()
        .map_err(|_| state_error("高度"))?
        .unwrap_or(180.0);
    let Some((bounds, geometry)) = layout.anchored(anchor, height) else {
        crate::platform::hide(panel.hwnd()?.0 as isize);
        return Ok(false);
    };
    if state.update_geometry(geometry)? {
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
    Ok(state
        .content
        .lock()
        .map_err(|_| "浮窗内容状态不可用")?
        .clone())
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
    let generation = _preview.clear().map_err(|e| e.to_string())?;
    if let Some(panel) = window.app_handle().get_webview_window("preview") {
        let _ = panel.emit("compact-preview-changed", Option::<Content>::None);
    }
    if !state.settings.lock().map_err(|_| "设置状态不可用")?.compact {
        return Ok(());
    }
    let shared = state.inner().clone();
    let content =
        tauri::async_runtime::spawn_blocking(move || load_content(&shared, &id, generation))
            .await
            .map_err(|_| "预览任务失败".to_string())??;
    let main = window.clone();
    window
        .run_on_main_thread(move || {
            if let Err(error) = present(&main, generation, content) {
                crate::diag::write(format!("preview presentation failed: {error}"));
            }
        })
        .map_err(|_| "预览窗口更新失败".to_string())
}

fn load_content(shared: &State, id: &str, generation: u64) -> Result<Content, String> {
    let (item, detail) = crate::item_detail::read::display(&shared.storage, id)
        .map_err(|_| "预览记录读取失败或状态已变化".to_string())?;
    Ok::<_, String>(Content {
        generation,
        item,
        detail,
    })
}

fn state_error(name: &str) -> tauri::Error {
    tauri::Error::Anyhow(anyhow::anyhow!("浮窗{name}状态不可用"))
}

fn present(main: &WebviewWindow, generation: u64, content: Content) -> tauri::Result<()> {
    let preview = main.state::<PreviewState>();
    if preview.generation.load(Ordering::Acquire) != generation {
        return Ok(());
    }
    let state = main.state::<Arc<State>>();
    let compact = state
        .settings
        .lock()
        .map_err(|_| state_error("设置"))?
        .compact;
    if !compact
        || !main.is_visible()?
        || main.is_minimized()?
        || !crate::platform::main_has_foreground(state.main.load(Ordering::Acquire))
    {
        return Ok(());
    }
    let Some(panel) = main.app_handle().get_webview_window("preview") else {
        return Ok(());
    };
    preview.publish(generation, content, |content| {
        if let Err(error) = panel.emit("compact-preview-changed", Some(content)) {
            crate::diag::write(format!("preview content event failed: {error}"));
        }
        if let Err(error) = reposition(main) {
            crate::diag::write(format!("preview window update failed: {error}"));
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn poison<T>(mutex: &Mutex<T>) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = mutex.lock().unwrap();
            panic!("模拟浮窗状态锁中毒");
        }));
        assert!(result.is_err());
    }

    fn content(generation: u64) -> Content {
        Content {
            generation,
            item: ItemView::default(),
            detail: ItemDetail {
                rich: None,
                image: None,
                text: String::new(),
                truncated: false,
                files: vec![],
            },
        }
    }

    #[test]
    fn poisoned_content_rejects_reads_clear_height_and_publish() {
        let state = PreviewState::default();
        let generation = state.clear().unwrap();
        poison(&state.content);
        assert!(state.has_content().is_err());
        assert!(state.clear().is_err());
        assert!(state.report_height(generation, 180.0).is_err());
        assert!(state
            .publish(generation, content(generation), |_| panic!(
                "中毒快照不能显示"
            ))
            .is_err());
        assert_eq!(state.generation.load(Ordering::Acquire), generation);
    }

    #[test]
    fn poisoned_height_keeps_content_readable_and_clearable() {
        let state = PreviewState::default();
        let generation = state.clear().unwrap();
        state
            .publish(generation, content(generation), |_| {})
            .unwrap();
        poison(&state.height);
        assert!(state.report_height(generation, 180.0).is_err());
        assert!(state.has_content().unwrap());
        state.clear().unwrap();
        assert!(!state.has_content().unwrap());
    }

    #[test]
    fn geometry_changes_are_deduplicated_and_poison_is_reported() {
        let state = PreviewState::default();
        let geometry = Geometry {
            right: true,
            arrow: 60.0,
            width: 360,
            height: 180,
            scale: 1.0,
        };
        assert!(state.update_geometry(geometry).unwrap());
        assert!(!state.update_geometry(geometry).unwrap());
        assert!(state.geometry.try_lock().is_ok());
        poison(&state.geometry);
        assert!(state.update_geometry(geometry).is_err());
    }

    #[test]
    fn presentation_callback_can_reenter_snapshot_without_deadlock() {
        let state = PreviewState::default();
        let generation = state.clear().unwrap();
        let content = Content {
            generation,
            item: ItemView::default(),
            detail: ItemDetail {
                rich: None,
                image: None,
                text: String::new(),
                truncated: false,
                files: vec![],
            },
        };
        assert!(!state
            .publish(generation, content, |_| {
                assert!(state.has_content().unwrap());
                state.clear().unwrap();
            })
            .unwrap());
        assert!(!state.has_content().unwrap());
    }
    const WORK: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };
    #[test]
    fn anchored_layout_clamps_to_work_area_and_points_to_selected_row() {
        for scale in [1.0, 1.5, 2.0] {
            for x in [100, 1500] {
                let layout = Layout {
                    main: Rect {
                        x,
                        y: 20,
                        width: 390,
                        height: 700,
                    },
                    work: WORK,
                    scale,
                    inner_y: 60,
                };
                let anchor = Anchor {
                    id: "synthetic".into(),
                    top: 1000.0,
                    bottom: 1040.0,
                };
                let (bounds, geometry) = layout.anchored(&anchor, 420.0).unwrap();
                assert_eq!(geometry.right, x == 100);
                assert!(bounds.y >= WORK.y);
                assert!(bounds.y + bounds.height as i32 <= WORK.y + WORK.height as i32);
                assert_eq!(geometry.arrow, f64::from(bounds.height) / scale - 26.0);
                assert_eq!(geometry.width, bounds.width);
                assert_eq!(geometry.height, bounds.height);
            }
        }
    }
    #[test]
    fn late_content_cannot_reopen_after_hide_or_new_selection() {
        let state = PreviewState::default();
        let content = || Content {
            generation: state.generation.load(Ordering::Acquire),
            item: ItemView::default(),
            detail: ItemDetail {
                rich: None,
                image: None,
                text: "synthetic".into(),
                truncated: false,
                files: vec![],
            },
        };
        let old = state.clear().unwrap();
        let new = state.clear().unwrap();
        assert!(!state
            .publish(old, content(), |_| panic!("旧选择不能显示"))
            .unwrap());
        assert!(state.publish(new, content(), |_| {}).unwrap());
        assert!(state.has_content().unwrap());
        state.clear().unwrap();
        assert!(!state
            .publish(new, content(), |_| panic!("隐藏后不能重新显示"))
            .unwrap());
        assert!(!state.has_content().unwrap());
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
                rich: None,
                image: None,
                text: String::new(),
                truncated: false,
                files: vec![],
            },
        };
        let old = state.clear().unwrap();
        state.publish(old, content(old), |_| {}).unwrap();
        assert!(state.report_height(old, 180.0).unwrap());
        assert!(!state.report_height(old, 180.2).unwrap());
        assert!(!state.report_height(old, f64::NAN).unwrap());
        let new = state.clear().unwrap();
        state.publish(new, content(new), |_| {}).unwrap();
        assert!(!state.report_height(old, 640.0).unwrap());
        assert_eq!(*state.height.lock().unwrap(), Some(180.0));
        assert!(state.report_height(new, 9000.0).unwrap());
        assert_eq!(*state.height.lock().unwrap(), Some(640.0));
        state.clear().unwrap();
        assert!(!state.report_height(new, 120.0).unwrap());
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
