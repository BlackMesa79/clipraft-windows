use crate::{
    data_directory,
    runtime::State,
    storage::{BackgroundLayout, BackgroundOptions, MainBackground, Storage},
};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};
use tauri::WebviewWindow;

#[cfg(test)]
mod tests;

const MIB: usize = 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 24 * MIB;
const MAX_SOURCE_SIDE: u32 = 8192;
const MAX_SOURCE_PIXELS: u64 = 16_000_000;
const MAX_DISPLAY_SIDE: u32 = 1920;
const MAX_JPEG_BYTES: usize = 3 * MIB;
const JPEG_QUALITY: u8 = 85;

fn import_file(path: &Path) -> Result<MainBackground> {
    let file = File::open(path).context("无法打开背景图片")?;
    let metadata = file.metadata().context("无法读取背景图片信息")?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES as u64 {
        bail!("请选择不超过 24 MiB 的图片文件");
    }
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("无法读取背景图片")?;
    if bytes.len() > MAX_SOURCE_BYTES {
        bail!("请选择不超过 24 MiB 的图片文件");
    }
    let jpeg = normalize(&bytes).context("背景图片导入失败")?;
    Ok(MainBackground {
        options: BackgroundOptions::default(),
        jpeg: STANDARD.encode(jpeg),
    })
}

fn normalize(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
    ) {
        bail!("请选择 PNG、JPEG 或 WebP 图片");
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        bail!("图片总像素超过 1600 万");
    }
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    encode_display(image)
}

fn encode_display(mut image: DynamicImage) -> Result<Vec<u8>> {
    if image.width().max(image.height()) > MAX_DISPLAY_SIDE {
        image = image.thumbnail(MAX_DISPLAY_SIDE, MAX_DISPLAY_SIDE);
    }
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY).encode_image(&image.into_rgb8())?;
    if jpeg.len() > MAX_JPEG_BYTES {
        bail!("背景图片的显示副本过大，请选择较小的图片");
    }
    Ok(jpeg)
}

fn check_window(window: &WebviewWindow) -> Result<()> {
    if window.label() != "main" {
        bail!("仅主窗口可访问背景图片");
    }
    Ok(())
}

fn check_write(window: &WebviewWindow, state: &State) -> Result<()> {
    check_window(window)?;
    if state.smoke {
        bail!("启动检查模式不能修改背景图片");
    }
    Ok(())
}

async fn run<T: Send + 'static>(
    task: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| "背景图片任务失败".to_string())?
        .map_err(|error| format!("{error:#}"))
}

fn with_storage<T>(state: &State, action: impl FnOnce(&Storage) -> Result<T>) -> Result<T> {
    let store = state.storage.lock().map_err(|_| anyhow!("存储不可用"))?;
    action(&store)
}

#[tauri::command]
pub(crate) async fn get_main_background(
    layout: BackgroundLayout,
    window: WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<Option<MainBackground>, String> {
    check_window(&window).map_err(|error| error.to_string())?;
    let shared = state.inner().clone();
    run(move || with_storage(&shared, |store| store.load_main_background(layout))).await
}

#[tauri::command]
pub(crate) async fn import_main_background(
    layout: BackgroundLayout,
    window: WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<Option<MainBackground>, String> {
    check_write(&window, &state).map_err(|error| error.to_string())?;
    let owner = window.hwnd().map_err(|_| "主窗口不可用")?.0 as isize;
    let shared = state.inner().clone();
    run(move || {
        // 文件选择窗使用独立 STA，解码和数据库写入在后台执行。
        let selected = std::thread::spawn(move || data_directory::select_background(owner))
            .join()
            .map_err(|_| anyhow!("背景图片选择窗口失败"))??;
        let Some(path) = selected else {
            return Ok(None);
        };
        let background = import_file(&path)?;
        with_storage(&shared, |store| {
            store.save_main_background(layout, &background)
        })?;
        Ok(Some(background))
    })
    .await
}

#[tauri::command]
pub(crate) async fn set_main_background_options(
    layout: BackgroundLayout,
    options: BackgroundOptions,
    window: WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<(), String> {
    check_write(&window, &state).map_err(|error| error.to_string())?;
    let shared = state.inner().clone();
    run(move || {
        with_storage(&shared, |store| {
            store.update_main_background_options(layout, options)
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn remove_main_background(
    layout: BackgroundLayout,
    window: WebviewWindow,
    state: tauri::State<'_, Arc<State>>,
) -> Result<(), String> {
    check_write(&window, &state).map_err(|error| error.to_string())?;
    let shared = state.inner().clone();
    run(move || with_storage(&shared, |store| store.remove_main_background(layout))).await
}
