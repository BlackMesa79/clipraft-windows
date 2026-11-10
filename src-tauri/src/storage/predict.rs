//! CM03 固定 Up：只改变像素字节，保留行末填充、头部、掩码、调色板与尾部。
use crate::image_normalize::dib_pixels;
use anyhow::{anyhow, bail, Result};

pub(super) const FILTER_UP: u8 = 2;

pub(super) fn encode(plain: &[u8]) -> Option<Vec<u8>> {
    let layout = dib_pixels(plain)?;
    let mut out = Vec::with_capacity(plain.len().checked_add(1)?);
    out.extend_from_slice(plain);
    // 首行的上一行为零；内存中的行顺序与正负高度无关。
    for row in 1..layout.rows {
        let start = layout.offset + row * layout.stride;
        let previous = &plain[start - layout.stride..start - layout.stride + layout.row_bytes];
        for ((output, &value), &up) in out[start..start + layout.row_bytes]
            .iter_mut()
            .zip(&plain[start..start + layout.row_bytes])
            .zip(previous)
        {
            *output = value.wrapping_sub(up);
        }
    }
    // 一个全局类型字节适用于所有行，随预测数据一起压缩并认证。
    out.push(FILTER_UP);
    Some(out)
}

pub(super) fn decode(mut data: Vec<u8>, original_len: usize) -> Result<Vec<u8>> {
    if data.len().checked_sub(1) != Some(original_len) || data.pop() != Some(FILTER_UP) {
        bail!("对象预测长度或滤波类型无效");
    }
    let layout = dib_pixels(&data).ok_or_else(|| anyhow!("对象预测位图头无效"))?;
    // 共用解析器已验证 stride * rows 与所有范围；split_at_mut 确保读写不交叠。
    for row in 1..layout.rows {
        let start = layout.offset + row * layout.stride;
        let (before, after) = data.split_at_mut(start);
        let previous = &before[start - layout.stride..start - layout.stride + layout.row_bytes];
        for (value, &up) in after[..layout.row_bytes].iter_mut().zip(previous) {
            *value = value.wrapping_add(up);
        }
    }
    Ok(data)
}
