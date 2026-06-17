use std::path::Path;

use async_trait::async_trait;
use chrono::{NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde_json::{Map, Value};
use tokio::process::Command;

use crate::error::{Error, Result};
use crate::exif::{ExifExtractor, ExifResult, PICK_KEYS};

pub struct ExiftoolExtractor {
    exe: String,
}

impl ExiftoolExtractor {
    pub fn new(exe: impl Into<String>) -> Self {
        Self { exe: exe.into() }
    }
}

#[async_trait]
impl ExifExtractor for ExiftoolExtractor {
    async fn extract(&self, path: &Path) -> Result<Option<ExifResult>> {
        // -json：JSON 输出；保留文本化标签值（不加 -n）以贴近 exiftool-vendored 默认。
        let output = Command::new(&self.exe)
            .arg("-json")
            .arg("-api")
            .arg("largefilesupport=1")
            .arg(path)
            .output()
            .await
            .map_err(|e| Error::Exif {
                key: path.display().to_string(),
                message: format!("spawn exiftool failed: {e}"),
            })?;
        if !output.status.success() {
            return Err(Error::Exif {
                key: path.display().to_string(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
        let parsed: Vec<Value> = serde_json::from_slice(&output.stdout)?;
        let Some(Value::Object(raw)) = parsed.into_iter().next() else {
            return Ok(None);
        };

        let mut picked = Map::new();
        for key in PICK_KEYS {
            if let Some(v) = raw.get(*key) {
                picked.insert((*key).to_string(), v.clone());
            }
        }
        // 派生 ImageWidth/ImageHeight
        if let Some(w) = raw.get("ExifImageWidth") {
            picked.insert("ImageWidth".into(), w.clone());
        }
        if let Some(h) = raw.get("ExifImageHeight") {
            picked.insert("ImageHeight".into(), h.clone());
        }

        // 日期 → ISO
        let date_taken_iso = picked
            .get("DateTimeOriginal")
            .and_then(|v| v.as_str())
            .and_then(exif_date_to_iso);
        if let Some(iso) = &date_taken_iso {
            picked.insert("DateTimeOriginal".into(), Value::String(iso.clone()));
        }
        if let Some(iso) = picked
            .get("DateTimeDigitized")
            .and_then(|v| v.as_str())
            .and_then(exif_date_to_iso)
        {
            picked.insert("DateTimeDigitized".into(), Value::String(iso));
        }

        // GPSAltitudeRef 归一化 0/1
        if let Some(v) = picked.get("GPSAltitudeRef").cloned() {
            let norm = match v.as_str() {
                Some(s) if s.contains("Below") => 1,
                _ => 0,
            };
            picked.insert("GPSAltitudeRef".into(), Value::from(norm));
        }

        let orientation = picked
            .get("Orientation")
            .and_then(orientation_to_u32)
            .unwrap_or(1);

        Ok(Some(ExifResult {
            exif: Value::Object(picked),
            date_taken_iso,
            orientation,
        }))
    }
}

/// exiftool 默认日期格式 "YYYY:MM:DD HH:MM:SS"（可能带时区后缀）→ ISO（按 UTC，M1a 简化）。
fn exif_date_to_iso(s: &str) -> Option<String> {
    let head = &s[..s.len().min(19)];
    let ndt = NaiveDateTime::parse_from_str(head, "%Y:%m:%d %H:%M:%S").ok()?;
    Some(
        Utc.from_utc_datetime(&ndt)
            .to_rfc3339_opts(SecondsFormat::Millis, true),
    )
}

/// Orientation 可能是数字或文本（如 "Rotate 90 CW"）。文本时映射回 1..=8。
fn orientation_to_u32(v: &Value) -> Option<u32> {
    if let Some(n) = v.as_u64() {
        return Some(n as u32);
    }
    match v.as_str()? {
        "Horizontal (normal)" => Some(1),
        "Mirror horizontal" => Some(2),
        "Rotate 180" => Some(3),
        "Mirror vertical" => Some(4),
        "Mirror horizontal and rotate 270 CW" => Some(5),
        "Rotate 90 CW" => Some(6),
        "Mirror horizontal and rotate 90 CW" => Some(7),
        "Rotate 270 CW" => Some(8),
        _ => Some(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exiftool_available() -> bool {
        std::process::Command::new("exiftool")
            .arg("-ver")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn date_parsing() {
        assert_eq!(
            exif_date_to_iso("2024:05:01 10:11:12"),
            Some("2024-05-01T10:11:12.000Z".to_string())
        );
    }

    #[tokio::test]
    async fn reads_real_image_if_exiftool_present() {
        if !exiftool_available() {
            eprintln!("skip: exiftool not installed");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jpg");
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]));
        image::DynamicImage::ImageRgb8(img).save(&p).unwrap();
        let ex = ExiftoolExtractor::new("exiftool");
        let res = ex.extract(&p).await.unwrap();
        assert!(res.is_some());
    }
}
