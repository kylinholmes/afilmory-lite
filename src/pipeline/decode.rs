use image::{DynamicImage, imageops};

use crate::error::{Error, Result};

pub struct Decoded {
    pub image: DynamicImage,
    /// orientation 应用后的逻辑宽高（写入 manifest 的 width/height）
    pub width: u32,
    pub height: u32,
}

/// 解码图片字节；按 EXIF orientation 校正图像与尺寸。
/// orientation 来自 EXIF（1..=8，缺省按 1 处理）。
pub fn decode(bytes: &[u8], key: &str, orientation: u32) -> Result<Decoded> {
    let img = decode_raw(bytes, key)?;
    let img = apply_orientation(img, orientation);
    let (width, height) = (img.width(), img.height());
    Ok(Decoded {
        image: img,
        width,
        height,
    })
}

/// 按扩展名选择解码后端：HEIC/HEIF/HIF 走 libheif（`heic` feature）；其余走 `image` crate。
fn decode_raw(bytes: &[u8], key: &str) -> Result<image::DynamicImage> {
    let ext = key.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if matches!(ext.as_str(), "heic" | "heif" | "hif") {
        #[cfg(feature = "heic")]
        {
            return crate::pipeline::heic::decode_heic(bytes, key);
        }
        #[cfg(not(feature = "heic"))]
        {
            return Err(Error::Storage(format!(
                "{key}: HEIC not supported in this build (build with `--features heic` and install libheif)"
            )));
        }
    }
    image::load_from_memory(bytes).map_err(|e| Error::Image {
        key: key.to_string(),
        source: e,
    })
}

/// 按 EXIF orientation（1..=8）做几何校正。
pub fn apply_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => DynamicImage::ImageRgba8(imageops::flip_horizontal(&img)),
        3 => DynamicImage::ImageRgba8(imageops::rotate180(&img)),
        4 => DynamicImage::ImageRgba8(imageops::flip_vertical(&img)),
        5 => {
            let r = imageops::rotate90(&img);
            DynamicImage::ImageRgba8(imageops::flip_horizontal(&r))
        }
        6 => DynamicImage::ImageRgba8(imageops::rotate90(&img)),
        7 => {
            let r = imageops::rotate270(&img);
            DynamicImage::ImageRgba8(imageops::flip_horizontal(&r))
        }
        8 => DynamicImage::ImageRgba8(imageops::rotate270(&img)),
        _ => img, // 1 或未知
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn solid(w: u32, h: u32) -> Vec<u8> {
        let mut img = RgbaImage::new(w, h);
        for p in img.pixels_mut() {
            *p = Rgba([10, 20, 30, 255]);
        }
        let dyn_img = DynamicImage::ImageRgba8(img);
        let mut buf = std::io::Cursor::new(Vec::new());
        dyn_img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn decode_keeps_dims_for_orientation_1() {
        let d = decode(&solid(40, 30), "a.png", 1).unwrap();
        assert_eq!((d.width, d.height), (40, 30));
    }

    #[test]
    fn orientation_6_swaps_dims() {
        let d = decode(&solid(40, 30), "a.png", 6).unwrap();
        assert_eq!((d.width, d.height), (30, 40));
    }
}
