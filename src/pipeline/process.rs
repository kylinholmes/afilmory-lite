use sha2::{Digest, Sha256};

use crate::config::ProcessingConfig;
use crate::error::{Error, Result};
use crate::exif::ExifExtractor;
use crate::manifest::{PhotoManifestItem, VideoSource};
use crate::pipeline::{Geocoder, decode, info, motion_photo, thumbhash, thumbnail, tone};
use crate::storage::{StorageObject, StorageProvider};

pub struct PipelineDeps<'a> {
    pub storage: &'a dyn StorageProvider,
    pub exif: &'a dyn ExifExtractor,
    pub processing: &'a ProcessingConfig,
    /// 缩略图输出目录（thumbnails/）。
    pub thumb_dir: &'a std::path::Path,
    /// Live Photo 配对：图片 key -> 视频对象。
    pub live_map: &'a std::collections::HashMap<String, StorageObject>,
    /// 反查地理编码器（跨照片共享缓存/限速）；关闭时 locate() 恒返回 None。
    pub geocoder: &'a Geocoder,
}

/// 处理单张照片，写出缩略图文件，返回 manifest item。失败返回 Err（调用方记失败计数）。
/// EXIF 抽取失败是非致命的（降级为无 EXIF），以便缺少 exiftool 时仍能产出其余字段。
pub async fn process_photo(
    obj: &StorageObject,
    deps: &PipelineDeps<'_>,
) -> Result<PhotoManifestItem> {
    let key = &obj.key;
    let t_start = std::time::Instant::now();
    let raw = deps
        .storage
        .get_file(key)
        .await?
        .ok_or_else(|| Error::Storage(format!("missing file: {key}")))?;
    let ms_get = t_start.elapsed().as_millis();

    let id = photo_id(key, deps.processing.digest_suffix_length);
    // contentDigest（M1a 无格式转换 → 处理后字节 = 原始字节）
    let digest = hex::encode(Sha256::digest(&raw));

    // EXIF：写临时文件给 exiftool；失败则降级为 None
    let t_step = std::time::Instant::now();
    let exif_res = {
        let tmp = tempfile::Builder::new()
            .suffix(&dot_ext(key))
            .tempfile()
            .map_err(|e| Error::Io {
                path: std::path::PathBuf::from("exif-temp"),
                source: e,
            })?;
        std::fs::write(tmp.path(), &raw).map_err(|e| Error::Io {
            path: tmp.path().to_path_buf(),
            source: e,
        })?;
        match deps.exif.extract(tmp.path()).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("exif extract failed for {key}: {e}");
                None
            }
        }
    };
    let ms_exif = t_step.elapsed().as_millis();
    let orientation = exif_res.as_ref().map(|e| e.orientation).unwrap_or(1);
    let exif_value = exif_res.as_ref().map(|e| e.exif.clone());
    let exif_date = exif_res.as_ref().and_then(|e| e.date_taken_iso.clone());

    // 解码（按 orientation 校正）
    let t_step = std::time::Instant::now();
    let decoded = decode::decode(&raw, key, orientation)?;
    let ms_decode = t_step.elapsed().as_millis();

    // 缩略图 + thumbHash
    let t_step = std::time::Instant::now();
    let thumb_jpeg = thumbnail::make_thumbnail(
        &decoded.image,
        deps.processing.thumbnail_width,
        deps.processing.thumbnail_quality,
    )?;
    let ms_thumb = t_step.elapsed().as_millis();
    let t_step = std::time::Instant::now();
    let thumb_hash = thumbhash::compute_thumbhash(&thumb_jpeg)?;
    let ms_thumbhash = t_step.elapsed().as_millis();

    // 写缩略图文件
    std::fs::create_dir_all(deps.thumb_dir).map_err(|e| Error::Io {
        path: deps.thumb_dir.to_path_buf(),
        source: e,
    })?;
    let thumb_path = deps.thumb_dir.join(format!("{id}.jpg"));
    std::fs::write(&thumb_path, &thumb_jpeg).map_err(|e| Error::Io {
        path: thumb_path.clone(),
        source: e,
    })?;

    // 影调
    let t_step = std::time::Instant::now();
    let tone = tone::analyze_tone(&decoded.image);
    let ms_tone = t_step.elapsed().as_millis();

    // 地理编码（GPS → 城市/国家）；关闭或无 GPS 时为 None
    let location = deps.geocoder.locate(exif_value.as_ref()).await;

    // info
    let pinfo = info::extract_info(key, exif_date.as_deref());

    // 组装
    let extension = key
        .rsplit('.')
        .next()
        .filter(|e| e.len() < key.len())
        .map(|e| e.to_ascii_uppercase())
        .unwrap_or_else(|| "UNKNOWN".into());
    let is_hdr = compute_is_hdr(exif_value.as_ref());
    // 视频源：Motion Photo（图内嵌视频，优先）/ Live Photo（独立视频文件），二者互斥
    let live_video = deps
        .live_map
        .get(key.as_str())
        .map(|v| VideoSource::LivePhoto {
            video_url: deps.storage.generate_public_url(&v.key),
            s3_key: v.key.clone(),
        });
    let motion_video = motion_photo::detect_motion_photo(&raw[..], exif_value.as_ref());
    if live_video.is_some() && motion_video.is_some() {
        return Err(Error::Storage(format!(
            "{key} has both a Motion Photo and a Live Photo (not allowed)"
        )));
    }
    let video = motion_video.or(live_video);
    let thumbnail_url = format!("/thumbnails/{id}.jpg");
    let last_modified = obj
        .last_modified
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));

    let total_ms = t_start.elapsed().as_millis();
    let (w, h) = (decoded.width, decoded.height);
    tracing::info!(
        "processed {key} in {total_ms}ms (get={ms_get} exif={ms_exif} decode={ms_decode} thumb={ms_thumb} thumbhash={ms_thumbhash} tone={ms_tone}) {w}x{h}"
    );

    Ok(PhotoManifestItem {
        id,
        format: extension,
        title: pinfo.title,
        description: pinfo.description,
        date_taken: pinfo.date_taken,
        tags: pinfo.tags,
        original_url: deps.storage.generate_public_url(key),
        thumbnail_url,
        thumb_hash,
        width: decoded.width,
        height: decoded.height,
        aspect_ratio: decoded.width as f64 / decoded.height as f64,
        s3_key: key.clone(),
        last_modified,
        size: obj.size.unwrap_or(0),
        digest: Some(digest),
        exif: exif_value,
        tone_analysis: Some(tone),
        location,
        video,
        is_hdr,
        og_image_url: None,
    })
}

fn photo_id(key: &str, digest_suffix_length: usize) -> String {
    let base = key.rsplit('/').next().unwrap_or(key);
    let stem = base
        .rsplit_once('.')
        .map(|(s, _)| s)
        .filter(|s| !s.is_empty())
        .unwrap_or(base);
    if digest_suffix_length == 0 {
        return stem.to_string();
    }
    let hash = hex::encode(Sha256::digest(key.as_bytes()));
    format!("{stem}_{}", &hash[..digest_suffix_length.min(hash.len())])
}

fn dot_ext(key: &str) -> String {
    key.rsplit_once('.')
        .map(|(_, e)| format!(".{e}"))
        .unwrap_or_default()
}

fn compute_is_hdr(exif: Option<&serde_json::Value>) -> bool {
    let Some(e) = exif else { return false };
    if e.get("MPImageType").and_then(|v| v.as_str()) == Some("Gain Map Image") {
        return true;
    }
    if e.get("UniformResourceName").and_then(|v| v.as_str()) == Some("urn:iso:std:iso:ts:21496:-1")
    {
        return true;
    }
    // ContainerDirectory 含 GainMap 语义（exiftool 表示形式多样：数组/字符串，做容错子串匹配）
    if let Some(cd) = e.get("ContainerDirectory")
        && cd.to_string().contains("GainMap")
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id_without_suffix() {
        assert_eq!(photo_id("trip/DSC_0001.jpg", 0), "DSC_0001");
    }
    #[test]
    fn id_with_suffix() {
        let id = photo_id("trip/DSC_0001.jpg", 6);
        assert!(id.starts_with("DSC_0001_"));
        assert_eq!(id.len(), "DSC_0001_".len() + 6);
    }

    #[test]
    fn hdr_detection() {
        assert!(!compute_is_hdr(None));
        assert!(!compute_is_hdr(Some(&serde_json::json!({"Make": "X"}))));
        assert!(compute_is_hdr(Some(
            &serde_json::json!({"MPImageType": "Gain Map Image"})
        )));
        assert!(compute_is_hdr(Some(&serde_json::json!({
            "UniformResourceName": "urn:iso:std:iso:ts:21496:-1"
        }))));
        // ContainerDirectory 数组里含 GainMap 语义
        assert!(compute_is_hdr(Some(&serde_json::json!({
            "ContainerDirectory": [{"Item": {"Semantic": "Primary"}}, {"Item": {"Semantic": "GainMap", "Length": 123}}]
        }))));
    }
}
