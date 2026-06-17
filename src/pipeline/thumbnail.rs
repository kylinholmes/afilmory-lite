use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, ImageEncoder, imageops::FilterType};

use crate::error::{Error, Result};

/// 生成缩略图 JPEG 字节：宽 thumb_width，等比，不放大，质量 quality。
/// 输入 image 应已做过 orientation 校正。
pub fn make_thumbnail(image: &DynamicImage, thumb_width: u32, quality: u8) -> Result<Vec<u8>> {
    let resized = if image.width() > thumb_width {
        // resize 保持纵横比，约束到 (thumb_width, 极大)，等价于按宽缩放
        image.resize(thumb_width, u32::MAX, FilterType::Lanczos3)
    } else {
        image.clone() // withoutEnlargement：不放大
    };
    let rgb = resized.to_rgb8();
    let mut out = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut out, quality);
    encoder
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|e| Error::Image {
            key: "thumbnail".into(),
            source: e,
        })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn img(w: u32, h: u32) -> DynamicImage {
        let mut i = RgbaImage::new(w, h);
        for p in i.pixels_mut() {
            *p = Rgba([100, 120, 140, 255]);
        }
        DynamicImage::ImageRgba8(i)
    }

    #[test]
    fn downscales_wide_image_to_600() {
        let jpg = make_thumbnail(&img(1200, 900), 600, 100).unwrap();
        let decoded = image::load_from_memory(&jpg).unwrap();
        assert_eq!(decoded.width(), 600);
        assert_eq!(decoded.height(), 450);
    }

    #[test]
    fn does_not_enlarge_small_image() {
        let jpg = make_thumbnail(&img(300, 200), 600, 100).unwrap();
        let decoded = image::load_from_memory(&jpg).unwrap();
        assert_eq!(decoded.width(), 300);
    }
}
