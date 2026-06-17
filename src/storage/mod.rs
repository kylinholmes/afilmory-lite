mod local;
pub use local::LocalProvider;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};

use crate::error::Result;

pub const IMAGE_EXTS: &[&str] =
    &["jpg", "jpeg", "png", "webp", "bmp", "tiff", "tif", "heic", "heif", "hif"];

pub fn is_image_key(key: &str) -> bool {
    match key.rsplit('.').next() {
        Some(ext) if ext.len() < key.len() => IMAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
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

#[cfg(test)]
mod tests {
    use super::is_image_key;
    #[test]
    fn image_key_detection() {
        assert!(is_image_key("a/b.JPG"));
        assert!(is_image_key("c.png"));
        assert!(is_image_key("d.HEIC"));
        assert!(!is_image_key("e.txt"));
        assert!(!is_image_key("noext"));
    }
}
