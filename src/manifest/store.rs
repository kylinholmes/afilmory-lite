use std::path::Path;

use crate::error::{Error, Result};
use crate::manifest::model::*;

/// 读取现有 manifest；不存在/解析失败/版本不符返回默认空 manifest（M1a 不做迁移）。
pub fn load_manifest(path: &Path) -> Result<AfilmoryManifest> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(AfilmoryManifest::default());
    };
    match serde_json::from_str::<AfilmoryManifest>(&content) {
        Ok(m) if m.version == CURRENT_MANIFEST_VERSION => Ok(m),
        Ok(_) => Ok(AfilmoryManifest::default()),
        Err(_) => Ok(AfilmoryManifest::default()),
    }
}

/// 保存 manifest：data 按 dateTaken 降序，聚合 cameras/lenses，2 空格缩进。
pub fn save_manifest(path: &Path, mut items: Vec<PhotoManifestItem>) -> Result<()> {
    // ISO8601 字符串字典序 == 时间序；降序（最新在前）
    items.sort_by(|a, b| b.date_taken.cmp(&a.date_taken));
    let cameras = generate_cameras(&items);
    let lenses = generate_lenses(&items);
    let manifest = AfilmoryManifest {
        version: CURRENT_MANIFEST_VERSION.to_string(),
        data: items,
        cameras,
        lenses,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Io { path: parent.to_path_buf(), source: e })?;
    }
    let json = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(path, json).map_err(|e| Error::Io { path: path.to_path_buf(), source: e })?;
    Ok(())
}

fn exif_str<'a>(item: &'a PhotoManifestItem, key: &str) -> Option<&'a str> {
    item.exif.as_ref()?.get(key)?.as_str()
}

pub fn generate_cameras(items: &[PhotoManifestItem]) -> Vec<CameraInfo> {
    let mut seen: std::collections::BTreeMap<String, CameraInfo> = std::collections::BTreeMap::new();
    for item in items {
        let (Some(make), Some(model)) = (exif_str(item, "Make"), exif_str(item, "Model")) else {
            continue;
        };
        let make = make.trim().to_string();
        let model = model.trim().to_string();
        let display_name = format!("{make} {model}");
        seen.entry(display_name.clone())
            .or_insert(CameraInfo { make, model, display_name });
    }
    seen.into_values().collect() // BTreeMap 已按 displayName 升序
}

pub fn generate_lenses(items: &[PhotoManifestItem]) -> Vec<LensInfo> {
    let mut seen: std::collections::BTreeMap<String, LensInfo> = std::collections::BTreeMap::new();
    for item in items {
        let Some(model) = exif_str(item, "LensModel") else { continue };
        let model = model.trim().to_string();
        let make = exif_str(item, "LensMake").map(|s| s.trim().to_string());
        let display_name = match &make {
            Some(m) => format!("{m} {model}"),
            None => model.clone(),
        };
        seen.entry(display_name.clone())
            .or_insert(LensInfo { make, model, display_name });
    }
    seen.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn item(id: &str, date: &str, make: Option<&str>, model: Option<&str>) -> PhotoManifestItem {
        let exif = match (make, model) {
            (Some(mk), Some(md)) => Some(serde_json::json!({ "Make": mk, "Model": md })),
            _ => None,
        };
        PhotoManifestItem {
            id: id.into(),
            format: "JPG".into(),
            title: id.into(),
            description: String::new(),
            date_taken: date.into(),
            tags: vec![],
            original_url: String::new(),
            thumbnail_url: format!("/thumbnails/{id}.jpg"),
            thumb_hash: None,
            width: 100,
            height: 100,
            aspect_ratio: 1.0,
            s3_key: format!("{id}.jpg"),
            last_modified: date.into(),
            size: 0,
            digest: None,
            exif,
            tone_analysis: None,
            location: None,
            video: None,
            is_hdr: false,
            og_image_url: None,
        }
    }

    #[test]
    fn sorts_desc_and_aggregates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let items = vec![
            item("a", "2024-01-01T00:00:00.000Z", Some("Sony"), Some("A7")),
            item("b", "2024-03-01T00:00:00.000Z", Some("Sony"), Some("A7")),
            item("c", "2024-02-01T00:00:00.000Z", Some("Canon"), Some("R5")),
        ];
        save_manifest(&path, items).unwrap();
        let loaded = load_manifest(&path).unwrap();
        // 降序：b(3月) > c(2月) > a(1月)
        assert_eq!(
            loaded.data.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["b", "c", "a"]
        );
        // cameras 去重 + 升序：Canon R5, Sony A7
        assert_eq!(
            loaded.cameras.iter().map(|c| c.display_name.as_str()).collect::<Vec<_>>(),
            ["Canon R5", "Sony A7"]
        );
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempdir().unwrap();
        let m = load_manifest(&dir.path().join("nope.json")).unwrap();
        assert_eq!(m.version, "v10");
        assert!(m.data.is_empty());
    }
}
