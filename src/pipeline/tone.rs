use image::{DynamicImage, imageops::FilterType};

use crate::manifest::ToneAnalysis;

pub fn analyze_tone(image: &DynamicImage) -> ToneAnalysis {
    analyze_inner(image).unwrap_or_else(fallback)
}

fn fallback() -> ToneAnalysis {
    ToneAnalysis {
        tone_type: "normal".into(),
        brightness: 50,
        contrast: 50,
        shadow_ratio: 0.33,
        highlight_ratio: 0.33,
    }
}

fn analyze_inner(image: &DynamicImage) -> Option<ToneAnalysis> {
    let small = image.resize(256, 256, FilterType::Lanczos3).to_rgb8();
    let total = (small.width() * small.height()) as f64;
    if total == 0.0 {
        return None;
    }
    let mut lum = [0f64; 256];
    for p in small.pixels() {
        let l =
            (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64).round() as usize;
        lum[l.min(255)] += 1.0;
    }
    for v in lum.iter_mut() {
        *v /= total; // 归一化为概率
    }

    let total_lum: f64 = lum.iter().sum();
    let weighted: f64 = lum.iter().enumerate().map(|(i, &p)| i as f64 * p).sum();
    let mean = weighted / total_lum;
    let brightness = (mean * (100.0 / 255.0)).round() as u32;
    let shadow_ratio: f64 = lum[0..86].iter().sum();
    let highlight_ratio: f64 = lum[170..256].iter().sum();
    let variance: f64 = lum
        .iter()
        .enumerate()
        .map(|(i, &p)| p * (i as f64 - mean).powi(2))
        .sum();
    let std_dev = variance.sqrt();
    let contrast = ((std_dev / 127.5) * 100.0).round().min(100.0) as u32;

    let tone_type = if brightness < 30 && shadow_ratio > 0.6 {
        "low-key"
    } else if brightness > 70 && highlight_ratio > 0.6 {
        "high-key"
    } else if contrast > 60 && shadow_ratio > 0.3 && highlight_ratio > 0.3 {
        "high-contrast"
    } else {
        "normal"
    };

    Some(ToneAnalysis {
        tone_type: tone_type.into(),
        brightness,
        contrast,
        shadow_ratio: (shadow_ratio * 100.0).round() / 100.0,
        highlight_ratio: (highlight_ratio * 100.0).round() / 100.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgb, RgbImage};

    fn solid(r: u8, g: u8, b: u8) -> DynamicImage {
        let mut i = RgbImage::new(64, 64);
        for p in i.pixels_mut() {
            *p = Rgb([r, g, b]);
        }
        DynamicImage::ImageRgb8(i)
    }

    #[test]
    fn dark_image_is_low_key() {
        let t = analyze_tone(&solid(0, 0, 0));
        assert_eq!(t.tone_type, "low-key");
        assert_eq!(t.brightness, 0);
    }

    #[test]
    fn bright_image_is_high_key() {
        let t = analyze_tone(&solid(255, 255, 255));
        assert_eq!(t.tone_type, "high-key");
        assert_eq!(t.brightness, 100);
    }
}
