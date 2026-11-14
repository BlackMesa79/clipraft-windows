use std::sync::Mutex;
use tauri::{LogicalSize, PhysicalPosition, WebviewWindow};
#[derive(Default)]
pub struct WindowLayout(Mutex<LayoutState>);
#[derive(Default)]
struct LayoutState {
    compact: bool,
    regular: Option<(LogicalSize<f64>, bool)>,
}
impl WindowLayout {
    pub fn apply(&self, window: &WebviewWindow, compact: bool) -> tauri::Result<()> {
        let mut state = self.0.lock().unwrap();
        if state.compact == compact {
            return Ok(());
        }
        let scale = window.scale_factor()?;
        let current = window.inner_size()?;
        let maximized = window.is_maximized()?;
        if compact {
            state.regular = Some((current.to_logical(scale), maximized));
            if maximized {
                window.unmaximize()?;
            }
        }
        let mut size = if compact {
            LogicalSize::new(390.0, (current.height as f64 / scale).min(700.0))
                .to_physical::<u32>(scale)
        } else {
            state
                .regular
                .map(|v| v.0.to_physical(scale))
                .unwrap_or_else(|| LogicalSize::new(1000.0, 700.0).to_physical(scale))
        };
        if let Some(monitor) = window.current_monitor()? {
            let outer = window.outer_size()?;
            let inner = window.inner_size()?;
            let area = monitor.work_area();
            size.width = size.width.min(
                area.size
                    .width
                    .saturating_sub(outer.width.saturating_sub(inner.width))
                    .max(1),
            );
            size.height = size.height.min(
                area.size
                    .height
                    .saturating_sub(outer.height.saturating_sub(inner.height))
                    .max(1),
            );
        }
        window.set_size(size)?;
        // 使用调整后的外框大小约束位置，适配 DPI 与工作区边缘，不激活窗口。
        if let Some(monitor) = window.current_monitor()? {
            let area = monitor.work_area();
            let position = window.outer_position()?;
            let outer = window.outer_size()?;
            window.set_position(PhysicalPosition::new(
                position.x.clamp(
                    area.position.x,
                    area.position.x + area.size.width.saturating_sub(outer.width) as i32,
                ),
                position.y.clamp(
                    area.position.y,
                    area.position.y + area.size.height.saturating_sub(outer.height) as i32,
                ),
            ))?;
        }
        if !compact && state.regular.is_some_and(|v| v.1) {
            window.maximize()?;
        }
        state.compact = compact;
        Ok(())
    }
}
