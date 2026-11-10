use crate::model::{image_priority, Clip};
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage};
use std::{
    io::{Cursor, Read},
    path::Path,
};

pub struct Preview {
    pub png: Option<Vec<u8>>,
    pub large_png: Option<Vec<u8>>,
    pub placeholder: Option<String>,
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
pub fn dimensions(b: &[u8], name: &str) -> Option<(u32, u32)> {
    if name == "PNG" && b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((
            u32::from_be_bytes(b.get(16..20)?.try_into().ok()?),
            u32::from_be_bytes(b.get(20..24)?.try_into().ok()?),
        ));
    }
    if name.starts_with("CF_DIB") && u32_at(b, 0)? >= 40 {
        return Some((
            (u32_at(b, 4)? as i32).unsigned_abs(),
            (u32_at(b, 8)? as i32).unsigned_abs(),
        ));
    }
    None
}
fn component(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let v = (pixel & mask) >> mask.trailing_zeros();
    let max = mask >> mask.trailing_zeros();
    ((v as u64 * 255) / max as u64) as u8
}
pub fn decode_dib(b: &[u8]) -> Option<DynamicImage> {
    let header = u32_at(b, 0)? as usize;
    if ![40, 52, 56, 108, 124].contains(&header) || b.len() < header {
        return None;
    }
    let w = u32_at(b, 4)? as i32;
    let height = u32_at(b, 8)? as i32;
    if w <= 0 || height == 0 || height == i32::MIN {
        return None;
    }
    let h = height.unsigned_abs();
    let w = w as u32;
    if w > 16384 || h > 16384 || w as u64 * h as u64 > 32_000_000 {
        return None;
    }
    let bits = u16::from_le_bytes(b.get(14..16)?.try_into().ok()?);
    let compression = u32_at(b, 16)?;
    let colors = u32_at(b, 32)? as usize;
    let palette = if colors > 0 {
        colors
    } else if bits <= 8 {
        1usize << bits
    } else {
        0
    };
    let masks_extra = if header == 40 && (compression == 3 || compression == 6) {
        if compression == 6 {
            16
        } else {
            12
        }
    } else {
        0
    };
    let offset = header
        .checked_add(masks_extra)?
        .checked_add(palette.checked_mul(4)?)?;
    if offset > b.len() {
        return None;
    }
    if bits == 32 && [0, 3, 6].contains(&compression) {
        let count = (w as usize).checked_mul(h as usize)?;
        if b.len().checked_sub(offset)? < count.checked_mul(4)? {
            return None;
        }
        let masks = if compression == 3 || compression == 6 {
            [
                u32_at(b, 40)?,
                u32_at(b, 44)?,
                u32_at(b, 48)?,
                if header >= 56 || compression == 6 {
                    u32_at(b, 52)?
                } else {
                    0
                },
            ]
        } else {
            [
                0x00ff0000,
                0x0000ff00,
                0x000000ff,
                if header >= 108 {
                    u32_at(b, 52).unwrap_or(0)
                } else {
                    0
                },
            ]
        };
        let mut image = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let row = if height > 0 { h - 1 - y } else { y };
                let p = u32_at(b, offset + ((row * w + x) as usize) * 4)?;
                image.put_pixel(
                    x,
                    y,
                    Rgba([
                        component(p, masks[0]),
                        component(p, masks[1]),
                        component(p, masks[2]),
                        component(p, masks[3]),
                    ]),
                );
            }
        }
        return Some(DynamicImage::ImageRgba8(image));
    }
    // 其他位深补齐 BMP 文件头，再交给受限解码器。
    let mut bmp = Vec::with_capacity(b.len() + 14);
    bmp.extend(b"BM");
    bmp.extend(((b.len() + 14) as u32).to_le_bytes());
    bmp.extend([0; 4]);
    bmp.extend(((offset + 14) as u32).to_le_bytes());
    bmp.extend(b);
    decode_image(&bmp)
}
fn decode_image(bytes: &[u8]) -> Option<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().ok()
}
const FILE_LIMIT: u64 = 50 * 1024 * 1024;
fn encode_preview(image: DynamicImage) -> Option<Preview> {
    let encode = |width: u32, height: u32| {
        let mut out = Cursor::new(Vec::new());
        image
            .thumbnail(width.min(image.width()), height.min(image.height()))
            .write_to(&mut out, ImageFormat::Png)
            .ok()?;
        Some(out.into_inner())
    };
    Some(Preview {
        png: Some(encode(256u32, 160u32)?),
        large_png: Some(encode(800u32, 800u32)?),
        placeholder: None,
    })
}
fn unavailable(reason: String) -> Preview {
    Preview {
        png: None,
        large_png: None,
        placeholder: Some(reason),
    }
}
fn image_file(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp"
        )
    })
}
fn generate_file(path: &Path) -> Preview {
    let read = || -> Result<Vec<u8>, &'static str> {
        let file =
            std::fs::File::open(path).map_err(|_| "图片文件读取失败（不存在或无读取权限）")?;
        let metadata = file.metadata().map_err(|_| "无法读取图片文件信息")?;
        if !metadata.is_file() {
            return Err("图片路径不是普通文件");
        }
        if metadata.len() > FILE_LIMIT {
            return Err("图片文件超过 50 MiB，未生成预览");
        }
        let mut bytes = Vec::new();
        // 二次限制实际读取量，避免文件在检查大小后增长。
        file.take(FILE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "图片文件读取失败")?;
        if bytes.len() as u64 > FILE_LIMIT {
            return Err("图片文件超过 50 MiB，未生成预览");
        }
        Ok(bytes)
    };
    match read() {
        Ok(bytes) => decode_image(&bytes)
            .and_then(encode_preview)
            .unwrap_or_else(|| unavailable("图片文件无法解码或超过解码资源限制".into())),
        Err(reason) => unavailable(reason.into()),
    }
}
pub fn generate(clip: &Clip) -> Preview {
    if let Some(files) = clip.get("CF_HDROP").and_then(crate::model::decode_hdrop) {
        if let Some(path) = files.first().map(Path::new).filter(|p| image_file(p)) {
            return generate_file(path);
        }
        return Preview {
            png: None,
            large_png: None,
            placeholder: None,
        };
    }
    let mut failure = None;
    for name in image_priority() {
        if let Some(b) = clip.get(name) {
            let decoded = if name == "PNG" {
                decode_image(b)
            } else {
                decode_dib(b)
            };
            if let Some(preview) = decoded.and_then(encode_preview) {
                return preview;
            }
            failure.get_or_insert_with(|| match dimensions(b, name) {
                Some((w, h)) => format!("{name} · {w} × {h} · 无法解码或超过资源限制"),
                None => format!("{name} · 尺寸未知 · 无法解码"),
            });
        }
    }
    Preview {
        png: None,
        large_png: None,
        placeholder: failure,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FormatData;
    #[test]
    fn image_file_extension_filter_is_case_insensitive() {
        for ext in ["png", "jpg", "jpeg", "gif", "bmp", "webp", "PNG", "JpEg"] {
            assert!(image_file(Path::new(&format!("sample.{ext}"))));
        }
        for name in ["sample.svg", "sample.heic", "png", "sample.png.txt"] {
            assert!(!image_file(Path::new(name)));
        }
    }
    #[test]
    fn small_images_are_not_upscaled() {
        let result = encode_preview(DynamicImage::ImageRgba8(RgbaImage::new(10, 20))).unwrap();
        let image = image::load_from_memory(&result.large_png.unwrap()).unwrap();
        assert_eq!((image.width(), image.height()), (10, 20));
    }
    #[test]
    fn corrupt_has_placeholder() {
        let p = generate(&Clip {
            formats: vec![FormatData {
                name: "PNG".into(),
                bytes: vec![1, 2],
            }],
        });
        assert!(p.png.is_none());
        assert!(p.placeholder.unwrap().contains("PNG"));
    }
    #[test]
    fn dib_v5_alpha() {
        let mut b = vec![0u8; 128];
        for (i, v) in [
            (0, 124u32),
            (4, 1),
            (8, 1),
            (16, 3),
            (40, 0xff0000),
            (44, 0xff00),
            (48, 0xff),
            (52, 0xff000000),
        ] {
            b[i..i + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[12] = 1;
        b[14] = 32;
        b[124..128].copy_from_slice(&[3, 2, 1, 128]);
        assert_eq!(
            decode_dib(&b).unwrap().to_rgba8().get_pixel(0, 0).0,
            [1, 2, 3, 128]
        );
    }
    #[test]
    fn fallback_to_dib() {
        let mut b = vec![0u8; 44];
        b[0..4].copy_from_slice(&40u32.to_le_bytes());
        b[4] = 1;
        b[8] = 1;
        b[12] = 1;
        b[14] = 32;
        let p = generate(&Clip {
            formats: vec![
                FormatData {
                    name: "PNG".into(),
                    bytes: vec![0],
                },
                FormatData {
                    name: "CF_DIB".into(),
                    bytes: b,
                },
            ],
        });
        assert!(p.png.is_some());
    }
}
