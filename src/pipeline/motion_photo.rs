//! Motion Photo（图内嵌视频，安卓 Google/Samsung）检测。
//! EXIF 标志门控 + 在原始字节中定位内嵌 MP4（ftyp box）。
//! 注：上游还会读 ContainerDirectory 的精确 offset/length，但 exiftool JSON 表示形式多样，
//! 这里采用更稳健的「字节扫描 ftyp」，对结构/语义一致足够。

use serde_json::Value;

use crate::manifest::VideoSource;

const MIN_VIDEO_SIZE: usize = 8 * 1024;

/// 检测 Motion Photo；命中返回 `VideoSource::MotionPhoto`，否则 None。
pub fn detect_motion_photo(raw: &[u8], exif: Option<&Value>) -> Option<VideoSource> {
    let e = exif?;
    let is_motion = to_bool(e.get("MotionPhoto")) || to_bool(e.get("MicroVideo"));
    if !is_motion {
        return None;
    }
    let pts = to_i64(e.get("MotionPhotoPresentationTimestampUs"))
        .or_else(|| to_i64(e.get("MicroVideoPresentationTimestampUs")));

    // 内嵌 MP4 以 ftyp box 开头：[4 字节 box size]["ftyp"]...
    let ftyp = find_subsequence(raw, b"ftyp")?;
    if ftyp < 4 {
        return None;
    }
    let offset = ftyp - 4; // box size 字段在 "ftyp" 前 4 字节
    if raw.len().saturating_sub(offset) < MIN_VIDEO_SIZE {
        return None;
    }
    Some(VideoSource::MotionPhoto {
        offset: offset as u64,
        size: Some((raw.len() - offset) as u64),
        presentation_timestamp: pts,
    })
}

fn to_bool(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => {
            n.as_i64().is_some_and(|i| i != 0) || n.as_f64().is_some_and(|f| f != 0.0)
        }
        Some(Value::String(s)) => matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        _ => false,
    }
}

fn to_i64(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_with_embedded_video(prefix: usize, payload: usize) -> Vec<u8> {
        let mut v = vec![0xAAu8; prefix];
        v.extend_from_slice(&[0, 0, 0, 0]); // box size 占位
        v.extend_from_slice(b"ftyp");
        v.extend(std::iter::repeat_n(0xBBu8, payload));
        v
    }

    #[test]
    fn none_without_flag() {
        let raw = raw_with_embedded_video(10, 9000);
        assert!(detect_motion_photo(&raw, None).is_none());
        assert!(detect_motion_photo(&raw, Some(&serde_json::json!({"Make": "X"}))).is_none());
    }

    #[test]
    fn detects_with_flag_and_ftyp() {
        let raw = raw_with_embedded_video(10, 9000); // ftyp 在 pos=14 → offset=10
        let exif =
            serde_json::json!({"MotionPhoto": 1, "MotionPhotoPresentationTimestampUs": 500000});
        match detect_motion_photo(&raw, Some(&exif)) {
            Some(VideoSource::MotionPhoto {
                offset,
                size,
                presentation_timestamp,
            }) => {
                assert_eq!(offset, 10);
                assert_eq!(size, Some((raw.len() - 10) as u64));
                assert_eq!(presentation_timestamp, Some(500000));
            }
            _ => panic!("expected motion photo"),
        }
    }

    #[test]
    fn none_when_video_too_small() {
        let raw = raw_with_embedded_video(10, 100); // payload < 8KB
        let exif = serde_json::json!({"MicroVideo": "1"});
        assert!(detect_motion_photo(&raw, Some(&exif)).is_none());
    }
}
