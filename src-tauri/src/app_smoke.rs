//! 隔离启动检查：合成图片、前端就绪等待及结果落盘。
use crate::{app_events, model, runtime::State, storage::Storage};
use anyhow::Result;
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

pub(crate) fn seed(storage: &mut Storage) -> Result<()> {
    let mut encoded = std::io::Cursor::new(Vec::new());
    let image = image::RgbaImage::from_pixel(1000, 400, image::Rgba([32, 128, 96, 128]));
    image::DynamicImage::ImageRgba8(image).write_to(&mut encoded, image::ImageFormat::Png)?;
    storage.save(&model::Clip {
        formats: vec![model::FormatData {
            name: "PNG".into(),
            bytes: encoded.into_inner(),
        }],
    })
}

fn ready(state: &State) -> bool {
    state.main_ready.load(Ordering::Acquire)
        && state.quick_ready.load(Ordering::Acquire)
        && state.thumbnail_ready.load(Ordering::Acquire)
        && state.preview_ready.load(Ordering::Acquire)
}

fn write_report(state: &State, root: &Path) -> Result<()> {
    let report = serde_json::json!({
        "main_frontend_ready": state.main_ready.load(Ordering::Acquire),
        "quick_frontend_ready": state.quick_ready.load(Ordering::Acquire),
        "thumbnail_protocol_loaded": state.thumbnail_ready.load(Ordering::Acquire),
        "preview_protocol_loaded": state.preview_ready.load(Ordering::Acquire),
        "clipboard_listener_enabled": false, "keyboard_hook_enabled": false, "visual_verified": false
    });
    std::fs::write(
        root.join("smoke-result.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

pub(crate) fn start_report(app: tauri::AppHandle, state: Arc<State>, root: PathBuf) -> Result<()> {
    std::thread::Builder::new()
        .name("smoke-report".into())
        .spawn(move || {
            for _ in 0..100 {
                std::thread::sleep(Duration::from_millis(100));
                if ready(&state) {
                    break;
                }
            }
            let result = write_report(&state, &root);
            let success = ready(&state) && result.is_ok();
            app_events::record("smoke result write failed", result);
            app.exit(if success { 0 } else { 1 });
        })?;
    Ok(())
}
