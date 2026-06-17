use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::manifest::PhotoManifestItem;
use crate::storage::StorageObject;

/// 是否需要更新（基于 lastModified 时间戳字符串比较）。
pub fn needs_update(existing: Option<&PhotoManifestItem>, obj: &StorageObject) -> bool {
    let Some(existing) = existing else { return true };
    let Some(lm) = obj.last_modified else { return true };
    let obj_iso = lm.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    obj_iso != existing.last_modified // 字符串不等即视为变化（含更新/回退）
}

/// 缩略图文件是否存在。
pub fn thumbnail_exists(thumb_dir: &Path, photo_id: &str) -> bool {
    thumb_dir.join(format!("{photo_id}.jpg")).exists()
}

/// key → 纯 basename（去目录去扩展名），用于缩略图存在性判定（与上游一致，不含 digest 后缀）。
fn basename_id(key: &str) -> &str {
    let base = key.rsplit('/').next().unwrap_or(key);
    base.rsplit_once('.').map(|(s, _)| s).filter(|s| !s.is_empty()).unwrap_or(base)
}

/// 从存储图片列表筛选出需要处理的对象。force 时全量。
pub fn filter_tasks<'a>(
    images: &'a [StorageObject],
    existing_by_key: &HashMap<String, &PhotoManifestItem>,
    thumb_dir: &Path,
    force: bool,
) -> Vec<&'a StorageObject> {
    if force {
        return images.iter().collect();
    }
    images
        .iter()
        .filter(|obj| {
            let existing = existing_by_key.get(&obj.key).copied();
            if existing.is_none() {
                return true;
            }
            if needs_update(existing, obj) {
                return true;
            }
            !thumbnail_exists(thumb_dir, basename_id(&obj.key))
        })
        .collect()
}

/// 删除不在 manifest 中的缩略图，返回删除数量。
pub fn handle_deleted(thumb_dir: &Path, items: &[PhotoManifestItem]) -> usize {
    if items.is_empty() {
        let _ = std::fs::remove_dir_all(thumb_dir);
        return 0;
    }
    let keep: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let mut deleted = 0;
    if let Ok(rd) = std::fs::read_dir(thumb_dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(stem) = name.strip_suffix(".jpg")
                && !keep.contains(stem)
                && std::fs::remove_file(entry.path()).is_ok()
            {
                deleted += 1;
            }
        }
    }
    deleted
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    fn obj(key: &str, secs: i64) -> StorageObject {
        StorageObject {
            key: key.into(),
            size: Some(1),
            last_modified: Some(Utc.timestamp_opt(secs, 0).unwrap()),
            etag: None,
        }
    }

    fn item(id: &str, key: &str, last_modified: &str) -> PhotoManifestItem {
        PhotoManifestItem {
            id: id.into(),
            format: "JPG".into(),
            title: id.into(),
            description: String::new(),
            date_taken: last_modified.into(),
            tags: vec![],
            original_url: String::new(),
            thumbnail_url: format!("/thumbnails/{id}.jpg"),
            thumb_hash: None,
            width: 1,
            height: 1,
            aspect_ratio: 1.0,
            s3_key: key.into(),
            last_modified: last_modified.into(),
            size: 1,
            digest: None,
            exif: None,
            tone_analysis: None,
            location: None,
            video: None,
            is_hdr: false,
            og_image_url: None,
        }
    }

    #[test]
    fn new_and_unchanged_selection() {
        let dir = tempdir().unwrap();
        // a 的缩略图存在（id=a → a.jpg）
        std::fs::write(dir.path().join("a.jpg"), b"x").unwrap();
        let existing_item = item("a", "a.jpg", "1970-01-01T00:00:10.000Z");
        let mut map = HashMap::new();
        map.insert("a.jpg".to_string(), &existing_item);
        // a 未变(10s) 且缩略图存在 → 跳过；b 新增 → 选中
        let images = vec![obj("a.jpg", 10), obj("b.jpg", 5)];
        let tasks = filter_tasks(&images, &map, dir.path(), false);
        let keys: Vec<&str> = tasks.iter().map(|o| o.key.as_str()).collect();
        assert_eq!(keys, vec!["b.jpg"]);
    }

    #[test]
    fn deletes_orphan_thumbnails() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.jpg"), b"x").unwrap();
        std::fs::write(dir.path().join("gone.jpg"), b"x").unwrap();
        let items = vec![item("a", "a.jpg", "x")];
        let n = handle_deleted(dir.path(), &items);
        assert_eq!(n, 1);
        assert!(!dir.path().join("gone.jpg").exists());
        assert!(dir.path().join("a.jpg").exists());
    }
}
