//! 缩略图解码边界独立于采集规范化；保持现有受限头部与 BMP 回退范围。
use image::{DynamicImage, ImageReader, Rgba, RgbaImage};
use std::io::Cursor;

const MAX_IMAGE_SIDE: u32 = 16384;
const BMP_FILE_HEADER: usize = 14;

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

struct DibHeader {
    size: usize,
    width: u32,
    height: i32,
    bits: u16,
    compression: u32,
    offset: usize,
}

pub fn dimensions(bytes: &[u8], name: &str) -> Option<(u32, u32)> {
    if name == "PNG" && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((
            u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?),
            u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?),
        ));
    }
    if name.starts_with("CF_DIB") && u32_at(bytes, 0)? >= 40 {
        return Some((
            (u32_at(bytes, 4)? as i32).unsigned_abs(),
            (u32_at(bytes, 8)? as i32).unsigned_abs(),
        ));
    }
    None
}

pub fn decode_dib(bytes: &[u8]) -> Option<DynamicImage> {
    let header = DibHeader::read(bytes)?;
    if header.bits == 32 && matches!(header.compression, 0 | 3 | 6) {
        return header.decode_rgba(bytes);
    }
    decode_bmp(bytes, header.offset)
}

impl DibHeader {
    fn read(bytes: &[u8]) -> Option<Self> {
        let size = u32_at(bytes, 0)? as usize;
        if !matches!(size, 40 | 52 | 56 | 108 | 124) || bytes.len() < size {
            return None;
        }
        let width = u32_at(bytes, 4)? as i32;
        let height = u32_at(bytes, 8)? as i32;
        if width <= 0 || height == 0 || height == i32::MIN {
            return None;
        }
        let rows = height.unsigned_abs();
        let width = width as u32;
        if width > MAX_IMAGE_SIDE
            || rows > MAX_IMAGE_SIDE
            || width as u64 * rows as u64 > crate::storage::crypto::MAX_PLAIN_BYTES as u64 / 4
        {
            return None;
        }
        let bits = u16::from_le_bytes(bytes.get(14..16)?.try_into().ok()?);
        let compression = u32_at(bytes, 16)?;
        let offset = pixel_offset(bytes, size, bits, compression)?;
        Some(Self {
            size,
            width,
            height,
            bits,
            compression,
            offset,
        })
    }

    fn masks(&self, bytes: &[u8]) -> Option<[u32; 4]> {
        match self.compression {
            3 | 6 => Some([
                u32_at(bytes, 40)?,
                u32_at(bytes, 44)?,
                u32_at(bytes, 48)?,
                if self.size >= 56 || self.compression == 6 {
                    u32_at(bytes, 52)?
                } else {
                    0
                },
            ]),
            _ => Some([
                0x00ff0000,
                0x0000ff00,
                0x000000ff,
                if self.size >= 108 {
                    u32_at(bytes, 52).unwrap_or(0)
                } else {
                    0
                },
            ]),
        }
    }

    fn decode_rgba(&self, bytes: &[u8]) -> Option<DynamicImage> {
        let rows = self.height.unsigned_abs();
        let count = (self.width as usize).checked_mul(rows as usize)?;
        if bytes.len().checked_sub(self.offset)? < count.checked_mul(4)? {
            return None;
        }
        let masks = self.masks(bytes)?;
        let mut image = RgbaImage::new(self.width, rows);
        for y in 0..rows {
            let row = if self.height > 0 { rows - 1 - y } else { y };
            for x in 0..self.width {
                let pixel = u32_at(bytes, self.offset + ((row * self.width + x) as usize) * 4)?;
                image.put_pixel(x, y, Rgba(masks.map(|mask| component(pixel, mask))));
            }
        }
        Some(DynamicImage::ImageRgba8(image))
    }
}

fn pixel_offset(bytes: &[u8], header: usize, bits: u16, compression: u32) -> Option<usize> {
    let colors = u32_at(bytes, 32)? as usize;
    let palette = match (colors, bits) {
        (0, 0..=8) => 1usize << bits,
        _ => colors,
    };
    let masks_extra = match (header, compression) {
        (40, 3) => 12,
        (40, 6) => 16,
        _ => 0,
    };
    let offset = header
        .checked_add(masks_extra)?
        .checked_add(palette.checked_mul(4)?)?;
    (offset <= bytes.len()).then_some(offset)
}

fn component(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let value = (pixel & mask) >> mask.trailing_zeros();
    let max = mask >> mask.trailing_zeros();
    ((value as u64 * 255) / max as u64) as u8
}

fn decode_bmp(bytes: &[u8], offset: usize) -> Option<DynamicImage> {
    // 其他位深补齐 BMP 文件头，再交给受限解码器。
    let mut bmp = Vec::with_capacity(bytes.len() + BMP_FILE_HEADER);
    bmp.extend(b"BM");
    bmp.extend(((bytes.len() + BMP_FILE_HEADER) as u32).to_le_bytes());
    bmp.extend([0; 4]);
    bmp.extend(((offset + BMP_FILE_HEADER) as u32).to_le_bytes());
    bmp.extend(bytes);
    decode_image(&bmp)
}

pub(super) fn decode_image(bytes: &[u8]) -> Option<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(crate::storage::crypto::MAX_PLAIN_BYTES as u64);
    reader.limits(limits);
    reader.decode().ok()
}
