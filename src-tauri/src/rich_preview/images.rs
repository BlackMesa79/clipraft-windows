use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
};
use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDriveTypeW};
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_TOTAL_READ: u64 = 40 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_PREVIEW: usize = 4 * 1024 * 1024;
const MAX_SIDE: u32 = 8192;
#[derive(Default)]
pub(super) struct Budget {
    read: u64,
    pixels: u64,
    output: usize,
}
pub(super) fn load(source: &str, budget: &mut Budget) -> Result<String, &'static str> {
    let bytes = read_source(source, budget)?;
    let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "图片格式无法识别")?
        .into_dimensions()
        .map_err(|_| "图片无法解码")?;
    let pixels = u64::from(width) * u64::from(height);
    if width > MAX_SIDE
        || height > MAX_SIDE
        || pixels > MAX_IMAGE_PIXELS
        || budget.pixels + pixels > MAX_TOTAL_PIXELS
    {
        return Err("图片超过预览解码限制");
    }
    budget.pixels += pixels;
    let preview = crate::thumbnail::rich_image(&bytes).ok_or("图片无法解码或生成预览")?;
    if budget.output + preview.len() > MAX_TOTAL_PREVIEW {
        return Err("图文图片总量超过预览限制");
    }
    budget.output += preview.len();
    let mime = crate::thumbnail::preview_mime(&preview).ok_or("图片预览格式无法识别")?;
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(preview)))
}
fn read_source(source: &str, budget: &mut Budget) -> Result<Vec<u8>, &'static str> {
    let url = tauri::Url::parse(source).map_err(|_| "图片地址不受支持，未读取资源")?;
    let bytes = match url.scheme() {
        "file" => read_local(&url, budget)?,
        "data" => read_embedded(source, budget)?,
        "http" | "https" => return Err("外部图片未加载（预览不联网）"),
        _ => return Err("图片地址不受支持，未读取资源"),
    };
    budget.read += bytes.len() as u64;
    Ok(bytes)
}
fn local_path(url: &tauri::Url) -> Result<PathBuf, &'static str> {
    if url.host_str().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("不读取网络或特殊文件地址");
    }
    let path = url.to_file_path().map_err(|()| "本地图片路径无效")?;
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err("本地图片路径无效");
    };
    if !matches!(prefix.kind(), std::path::Prefix::Disk(_)) {
        return Err("不读取网络或设备路径");
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp"
    ) {
        return Err("仅预览受支持的本地图片文件");
    }
    if path.components().any(|component| matches!(component, Component::Normal(name) if name.to_string_lossy().contains(':'))) {
        return Err("不读取文件附加数据流");
    }
    fixed_path(&path)?;
    Ok(path)
}
fn fixed_path(path: &Path) -> Result<(), &'static str> {
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
    let volume: PathBuf = path.components().take(2).collect();
    let wide: Vec<u16> = volume.as_os_str().encode_wide().chain([0]).collect();
    if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } != 3 {
        return Err("仅预览本地固定磁盘图片");
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if current.parent().is_none() {
            continue;
        }
        let metadata = fs::symlink_metadata(&current).map_err(|_| "原图片已失效或无读取权限")?;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("不读取链接或重解析路径中的图片");
        }
    }
    Ok(())
}
fn read_local(url: &tauri::Url, budget: &Budget) -> Result<Vec<u8>, &'static str> {
    let path = local_path(url)?;
    let file = fs::File::open(path).map_err(|_| "原图片已失效或无读取权限")?;
    let metadata = file.metadata().map_err(|_| "无法读取图片文件信息")?;
    if !metadata.is_file() {
        return Err("图片路径不是普通文件");
    }
    if metadata.len() > MAX_FILE_BYTES || budget.read + metadata.len() > MAX_TOTAL_READ {
        return Err("图片文件超过预览读取限制");
    }
    let remaining = MAX_FILE_BYTES.min(MAX_TOTAL_READ.saturating_sub(budget.read));
    let mut bytes = Vec::new();
    file.take(remaining + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "图片读取失败")?;
    if bytes.len() as u64 > remaining {
        return Err("图片文件超过预览读取限制");
    }
    Ok(bytes)
}
fn read_embedded(source: &str, budget: &Budget) -> Result<Vec<u8>, &'static str> {
    let (header, payload) = source.split_once(',').ok_or("内嵌图片数据无效")?;
    if !matches!(
        header,
        "data:image/png;base64"
            | "data:image/jpeg;base64"
            | "data:image/gif;base64"
            | "data:image/webp;base64"
            | "data:image/bmp;base64"
    ) {
        return Err("内嵌图片格式不受支持");
    }
    let remaining = MAX_FILE_BYTES.min(MAX_TOTAL_READ.saturating_sub(budget.read));
    if payload.len() as u64 > remaining.saturating_mul(4) / 3 + 4 {
        return Err("内嵌图片超过预览限制");
    }
    let bytes = STANDARD.decode(payload).map_err(|_| "内嵌图片数据无效")?;
    if bytes.len() as u64 > remaining {
        return Err("内嵌图片超过预览限制");
    }
    Ok(bytes)
}
