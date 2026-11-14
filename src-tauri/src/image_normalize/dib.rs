//! packed DIB 的布局校验；规范化与存储预测各自约束接受范围。
#[derive(Clone, Copy, Debug)]
pub(crate) struct DibPixels {
    pub offset: usize,
    pub stride: usize,
    pub rows: usize,
    pub row_bytes: usize,
}

struct Header {
    size: usize,
    width: usize,
    rows: usize,
    depth: usize,
    compression: u32,
    declared: usize,
    colors: usize,
    masks_extra: usize,
}

pub(super) fn length(bytes: &[u8]) -> Option<usize> {
    let header = Header::read(bytes)?;
    // 24 位位域只供存储预测，不扩大采集规范化的接受范围。
    if header.compression == 3 && header.depth == 24 {
        return None;
    }
    header.layout(bytes).map(|(end, _)| end)
}

pub(super) fn pixels(bytes: &[u8]) -> Option<DibPixels> {
    Header::read(bytes)?.layout(bytes)?.1
}

impl Header {
    fn read(bytes: &[u8]) -> Option<Self> {
        let size = u32_at(bytes, 0)? as usize;
        if !matches!(size, 12 | 40 | 52 | 56 | 108 | 124) || size > bytes.len() {
            return None;
        }
        match size {
            12 => Self::core(bytes),
            _ => Self::info(bytes, size),
        }
    }

    fn core(bytes: &[u8]) -> Option<Self> {
        let width = u16_at(bytes, 4)? as usize;
        let rows = u16_at(bytes, 6)? as usize;
        let depth = u16_at(bytes, 10)? as usize;
        if u16_at(bytes, 8)? != 1 || !matches!(depth, 1 | 4 | 8 | 24) {
            return None;
        }
        Some(Self {
            size: 12,
            width,
            rows,
            depth,
            compression: 0,
            declared: 0,
            colors: if depth <= 8 { 1usize << depth } else { 0 },
            masks_extra: 0,
        })
    }

    fn info(bytes: &[u8], size: usize) -> Option<Self> {
        let width = u32_at(bytes, 4)? as i32;
        let height = u32_at(bytes, 8)? as i32;
        let depth = u16_at(bytes, 14)? as usize;
        let compression = u32_at(bytes, 16)?;
        let declared = u32_at(bytes, 20)? as usize;
        let used = u32_at(bytes, 32)? as usize;
        if width <= 0 || height == 0 || height == i32::MIN || u16_at(bytes, 12)? != 1 {
            return None;
        }
        if !supported_compression(compression, depth, height, declared)
            || (depth > 0 && depth <= 8 && used > 1usize << depth)
        {
            return None;
        }
        let masks_extra = validate_masks(bytes, size, compression, depth)?;
        let colors = match (used, depth) {
            (0, 1..=8) => 1usize << depth,
            _ => used,
        };
        Some(Self {
            size,
            width: width as usize,
            rows: height.unsigned_abs() as usize,
            depth,
            compression,
            declared,
            colors,
            masks_extra,
        })
    }

    fn layout(&self, bytes: &[u8]) -> Option<(usize, Option<DibPixels>)> {
        if self.width == 0 || self.rows == 0 {
            return None;
        }
        let palette_entry = if self.size == 12 { 3 } else { 4 };
        let offset = self
            .size
            .checked_add(self.masks_extra)?
            .checked_add(self.colors.checked_mul(palette_entry)?)?;
        let (payload, pixels) = self.payload(offset)?;
        let end = offset.checked_add(payload)?;
        let end = profile_end(bytes, self.size, end)?;
        (end <= bytes.len()).then_some((end, pixels))
    }

    fn payload(&self, offset: usize) -> Option<(usize, Option<DibPixels>)> {
        if !matches!(self.compression, 0 | 3 | 6) {
            return Some((self.declared, None));
        }
        let stride = self
            .width
            .checked_mul(self.depth)?
            .checked_add(31)?
            .checked_div(32)?
            .checked_mul(4)?;
        let expected = stride.checked_mul(self.rows)?;
        if self.declared != 0 && self.declared < expected {
            return None;
        }
        let pixels = match (self.compression, self.depth) {
            (0 | 3, 24 | 32) => Some(DibPixels {
                offset,
                stride,
                rows: self.rows,
                row_bytes: self.width.checked_mul(self.depth / 8)?,
            }),
            _ => None,
        };
        // 非零 sizeImage 可能包含调用者声明的有效数据，保守保留。
        Some((self.declared.max(expected), pixels))
    }
}

fn supported_compression(compression: u32, depth: usize, height: i32, declared: usize) -> bool {
    match compression {
        0 => matches!(depth, 1 | 4 | 8 | 16 | 24 | 32),
        1 => depth == 8 && height > 0 && declared > 0,
        2 => depth == 4 && height > 0 && declared > 0,
        3 => matches!(depth, 16 | 24 | 32),
        6 => matches!(depth, 16 | 32),
        4 | 5 => depth == 0 && height > 0 && declared > 0,
        _ => false,
    }
}

fn validate_masks(bytes: &[u8], header: usize, compression: u32, depth: usize) -> Option<usize> {
    let count = match compression {
        3 => 3,
        6 => 4,
        _ => 0,
    };
    if count == 0 {
        return Some(0);
    }
    if header != 40 && header < 40 + count * 4 {
        return None;
    }
    let mut seen = 0u32;
    for index in 0..count {
        let mask = u32_at(bytes, 40 + index * 4)?;
        let shifted = mask.checked_shr(mask.trailing_zeros()).unwrap_or(0);
        if mask == 0
            || mask & seen != 0
            || (depth < 32 && mask >> depth != 0)
            || shifted & shifted.wrapping_add(1) != 0
        {
            return None;
        }
        seen |= mask;
    }
    Some(if header == 40 { count * 4 } else { 0 })
}

fn profile_end(bytes: &[u8], header: usize, end: usize) -> Option<usize> {
    if header != 124 {
        return Some(end);
    }
    let profile = u32_at(bytes, 112)? as usize;
    let size = u32_at(bytes, 116)? as usize;
    let color_space = u32_at(bytes, 56)?;
    if profile == 0 && size == 0 && !matches!(color_space, 0x4c494e4b | 0x4d424544) {
        return Some(end);
    }
    // 剪贴板 packed DIB 的 ICC 数据位于像素之后，不能当作尾部填充丢弃。
    if profile < end || size == 0 {
        return None;
    }
    profile.checked_add(size)
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
