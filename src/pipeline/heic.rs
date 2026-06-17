//! HEIC/HEIF 解码（libheif，`heic` feature）。
//!
//! ⚠️ 本文件仅在 `--features heic` 下编译，依赖系统 libheif（libheif-dev）。
//! 当前开发沙箱未安装 libheif，**此路径未经本地编译验证**——请在装了 libheif 的
//! 机器/CI 上 `cargo build --features heic` 确认 libheif-rs API（不同版本字段略有差异）。

use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

use crate::error::{Error, Result};

/// 把 HEIC/HEIF 字节解码为 RGB8 的 DynamicImage。
pub fn decode_heic(bytes: &[u8], key: &str) -> Result<image::DynamicImage> {
    let lib = LibHeif::new();
    let ctx = HeifContext::read_from_bytes(bytes)
        .map_err(|e| Error::Storage(format!("heic read {key}: {e}")))?;
    let handle = ctx
        .primary_image_handle()
        .map_err(|e| Error::Storage(format!("heic handle {key}: {e}")))?;
    let decoded = lib
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)
        .map_err(|e| Error::Storage(format!("heic decode {key}: {e}")))?;

    let width = decoded.width();
    let height = decoded.height();
    let planes = decoded.planes();
    let plane = planes
        .interleaved
        .ok_or_else(|| Error::Storage(format!("heic no interleaved plane {key}")))?;
    let stride = plane.stride;
    let data = plane.data;
    let row_bytes = (width as usize) * 3;

    // 按 stride 拷贝为紧凑 RGB（stride 可能大于 row_bytes）
    let mut buf = Vec::with_capacity(row_bytes * height as usize);
    for y in 0..height as usize {
        let start = y * stride;
        let end = start + row_bytes;
        if end > data.len() {
            return Err(Error::Storage(format!("heic plane truncated {key}")));
        }
        buf.extend_from_slice(&data[start..end]);
    }

    let rgb = image::RgbImage::from_raw(width, height, buf)
        .ok_or_else(|| Error::Storage(format!("heic buffer size mismatch {key}")))?;
    Ok(image::DynamicImage::ImageRgb8(rgb))
}
