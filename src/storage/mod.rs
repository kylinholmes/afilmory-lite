mod local;
mod s3;
mod sigv4;
pub use local::LocalProvider;
pub use s3::S3Provider;

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};

use crate::config::StorageConfig;
use crate::error::Result;

pub const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "bmp", "tiff", "tif", "heic", "heif", "hif",
];

pub fn is_image_key(key: &str) -> bool {
    match key.rsplit('.').next() {
        Some(ext) if ext.len() < key.len() => {
            IMAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str())
        }
        _ => false,
    }
}

#[derive(Debug, Clone)]
pub struct StorageObject {
    pub key: String,
    pub size: Option<u64>,
    pub last_modified: Option<DateTime<Utc>>,
    pub etag: Option<String>,
}

#[async_trait]
pub trait StorageProvider: Send + Sync {
    async fn list_images(&self) -> Result<Vec<StorageObject>>;
    async fn list_all_files(&self) -> Result<Vec<StorageObject>>;
    async fn get_file(&self, key: &str) -> Result<Option<Bytes>>;
    fn generate_public_url(&self, key: &str) -> String;
}

/// 从配置构建存储 provider（Builder 与「测试连接」端点共用）。
pub fn build_provider(cfg: &StorageConfig) -> Result<Arc<dyn StorageProvider>> {
    let provider: Arc<dyn StorageProvider> = match cfg {
        StorageConfig::Local {
            base_path,
            base_url,
            exclude_regex,
            max_file_limit,
        } => Arc::new(LocalProvider::new(
            base_path.clone(),
            base_url.clone(),
            exclude_regex.clone(),
            *max_file_limit,
        )?),
        StorageConfig::S3 {
            bucket,
            region,
            endpoint,
            access_key_id,
            secret_access_key,
            session_token,
            prefix,
            custom_domain,
            exclude_regex,
            max_file_limit,
            download_concurrency,
        } => Arc::new(S3Provider::new(
            bucket.clone(),
            region.clone(),
            endpoint.clone(),
            access_key_id.clone(),
            secret_access_key.clone(),
            session_token.clone(),
            prefix.clone(),
            custom_domain.clone(),
            exclude_regex.clone(),
            *max_file_limit,
            *download_concurrency,
        )?),
    };
    Ok(provider)
}

pub const VIDEO_EXTS: &[&str] = &["mov", "mp4"];

/// Live Photo 配对：同目录、同 basename（去扩展名），一图配一视频。
/// 视频扩展名仅 `.mov/.mp4`（与上游一致；扩展名小写比较，basename 大小写敏感）。
/// 返回 `图片 key -> 视频对象`。
pub fn detect_live_photos(
    all: &[StorageObject],
) -> std::collections::HashMap<String, StorageObject> {
    use std::collections::HashMap;
    let mut groups: HashMap<String, (Option<&StorageObject>, Option<&StorageObject>)> =
        HashMap::new();
    for obj in all {
        let (dir, stem, ext) = split_key(&obj.key);
        let ext = ext.to_ascii_lowercase();
        let gk = format!("{dir}/{stem}");
        let entry = groups.entry(gk).or_default();
        if IMAGE_EXTS.contains(&ext.as_str()) {
            entry.0 = Some(obj);
        } else if VIDEO_EXTS.contains(&ext.as_str()) {
            entry.1 = Some(obj);
        }
    }
    let mut map = HashMap::new();
    for (_, (img, vid)) in groups {
        if let (Some(i), Some(v)) = (img, vid) {
            map.insert(i.key.clone(), v.clone());
        }
    }
    map
}

/// 拆 key → (dirname, stem, ext)。
fn split_key(key: &str) -> (&str, &str, &str) {
    let (dir, file) = match key.rfind('/') {
        Some(i) => (&key[..i], &key[i + 1..]),
        None => ("", key),
    };
    match file.rfind('.') {
        Some(i) if i > 0 => (dir, &file[..i], &file[i + 1..]),
        _ => (dir, file, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(key: &str) -> StorageObject {
        StorageObject {
            key: key.into(),
            size: None,
            last_modified: None,
            etag: None,
        }
    }

    #[test]
    fn image_key_detection() {
        assert!(is_image_key("a/b.JPG"));
        assert!(is_image_key("c.png"));
        assert!(is_image_key("d.HEIC"));
        assert!(!is_image_key("e.txt"));
        assert!(!is_image_key("noext"));
    }

    #[test]
    fn live_photo_pairing() {
        let objs = vec![
            obj("trip/a.jpg"),
            obj("trip/a.mov"),
            obj("trip/b.png"),
            obj("c.heic"),
            obj("c.mp4"),
            obj("d.mov"), // 无配对图片
        ];
        let map = detect_live_photos(&objs);
        assert_eq!(
            map.get("trip/a.jpg").map(|o| o.key.as_str()),
            Some("trip/a.mov")
        );
        assert_eq!(map.get("c.heic").map(|o| o.key.as_str()), Some("c.mp4"));
        assert!(!map.contains_key("trip/b.png"));
        assert_eq!(map.len(), 2);
    }
}
