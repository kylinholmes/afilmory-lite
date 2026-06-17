use image::{DynamicImage, imageops::FilterType};

use crate::error::Result;

/// 由缩略图 JPEG 字节计算 thumbHash，返回小写 hex 字符串。
/// 复刻上游：resize 到 100x100(fit inside) → RGBA → rgba_to_thumb_hash → hex。
pub fn compute_thumbhash(thumbnail_jpeg: &[u8]) -> Result<Option<String>> {
    let img = match image::load_from_memory(thumbnail_jpeg) {
        Ok(i) => i,
        Err(_) => return Ok(None),
    };
    let small: DynamicImage = img.resize(100, 100, FilterType::Lanczos3);
    let rgba = small.to_rgba8();
    let hash = thumbhash::rgba_to_thumb_hash(rgba.width() as usize, rgba.height() as usize, rgba.as_raw());
    Ok(Some(hex::encode(hash)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut i = RgbaImage::new(w, h);
        for p in i.pixels_mut() {
            *p = Rgba([200, 50, 50, 255]);
        }
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(i)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg)
            .unwrap();
        out
    }

    #[test]
    fn produces_lowercase_hex() {
        let h = compute_thumbhash(&jpeg(600, 400)).unwrap().unwrap();
        assert!(!h.is_empty());
        assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // thumbhash 可被解回
        let bytes = hex::decode(&h).unwrap();
        assert!(!bytes.is_empty());
    }
}
