use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub site: serde_json::Value, // 原样透传给 __SITE_CONFIG__（M1a 不强类型化）
    pub storage: StorageConfig,
    #[serde(default)]
    pub processing: ProcessingConfig,
    #[serde(default)]
    pub exif: ExifConfig,
    #[serde(default)]
    pub triggers: TriggersConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_workdir")]
    pub workdir: PathBuf,
    #[serde(default)]
    pub dist_dir: PathBuf,
}
fn default_listen() -> String {
    "0.0.0.0:8080".to_string()
}
fn default_workdir() -> PathBuf {
    PathBuf::from("./data")
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum StorageConfig {
    Local {
        base_path: PathBuf,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        exclude_regex: Option<String>,
        #[serde(default)]
        max_file_limit: Option<usize>,
    },
    /// S3 及 S3 兼容（AWS / MinIO / Cloudflare R2 / Wasabi 等，通过 endpoint）。
    S3 {
        bucket: String,
        #[serde(default = "default_region")]
        region: String,
        #[serde(default)]
        endpoint: Option<String>,
        access_key_id: String,
        secret_access_key: String,
        #[serde(default)]
        session_token: Option<String>,
        #[serde(default)]
        prefix: Option<String>,
        #[serde(default)]
        custom_domain: Option<String>,
        #[serde(default)]
        exclude_regex: Option<String>,
        #[serde(default)]
        max_file_limit: Option<usize>,
        #[serde(default = "default_download_concurrency")]
        download_concurrency: usize,
    },
}
fn default_region() -> String {
    "us-east-1".to_string()
}
fn default_download_concurrency() -> usize {
    16
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessingConfig {
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_thumb_width")]
    pub thumbnail_width: u32,
    #[serde(default = "default_thumb_quality")]
    pub thumbnail_quality: u8,
    #[serde(default)]
    pub digest_suffix_length: usize,
    #[serde(default = "default_true")]
    pub enable_live_photo: bool,
}
fn default_concurrency() -> usize {
    10
}
fn default_true() -> bool {
    true
}
fn default_thumb_width() -> u32 {
    600
}
fn default_thumb_quality() -> u8 {
    100
}
impl Default for ProcessingConfig {
    fn default() -> Self {
        Self {
            concurrency: 10,
            thumbnail_width: 600,
            thumbnail_quality: 100,
            digest_suffix_length: 0,
            enable_live_photo: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExifConfig {
    #[serde(default = "default_exiftool")]
    pub exiftool_path: String,
}
fn default_exiftool() -> String {
    "exiftool".to_string()
}
impl Default for ExifConfig {
    fn default() -> Self {
        Self {
            exiftool_path: "exiftool".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TriggersConfig {
    /// 定时轮询间隔（秒）；0 = 关闭轮询。
    #[serde(default)]
    pub poll_interval_secs: u64,
    /// webhook / 手动触发的 Bearer token；None = 关闭 webhook 与 admin 触发。
    #[serde(default)]
    pub webhook_token: Option<String>,
    /// 是否启用 S3 事件通知端点。
    #[serde(default)]
    pub enable_s3_event: bool,
}

impl Config {
    pub fn from_toml_str(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(|e| Error::Config(e.to_string()))
    }
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).map_err(|e| Error::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::from_toml_str(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_local_config() {
        let toml = r#"
            [server]
            workdir = "/app/data"
            dist_dir = "/app/web/dist"
            [storage]
            provider = "local"
            base_path = "/photos"
            [processing]
            concurrency = 4
            [triggers]
            poll_interval_secs = 300
            webhook_token = "secret"
        "#;
        let c = Config::from_toml_str(toml).unwrap();
        assert_eq!(c.processing.concurrency, 4);
        assert_eq!(c.processing.thumbnail_width, 600); // 默认
        assert_eq!(c.server.listen, "0.0.0.0:8080"); // 默认
        assert_eq!(c.triggers.poll_interval_secs, 300);
        assert_eq!(c.triggers.webhook_token.as_deref(), Some("secret"));
        assert!(!c.triggers.enable_s3_event); // 默认 false
        match c.storage {
            StorageConfig::Local { base_path, .. } => {
                assert_eq!(base_path, PathBuf::from("/photos"))
            }
            _ => panic!("expected Local"),
        }
    }

    #[test]
    fn parses_s3_config() {
        let toml = r#"
            [server]
            workdir = "/d"
            [storage]
            provider = "s3"
            bucket = "my-bucket"
            endpoint = "https://minio.example.com"
            access_key_id = "AK"
            secret_access_key = "SK"
            prefix = "photos/"
        "#;
        let c = Config::from_toml_str(toml).unwrap();
        match c.storage {
            StorageConfig::S3 {
                bucket,
                region,
                endpoint,
                download_concurrency,
                prefix,
                ..
            } => {
                assert_eq!(bucket, "my-bucket");
                assert_eq!(region, "us-east-1"); // 默认
                assert_eq!(endpoint.as_deref(), Some("https://minio.example.com"));
                assert_eq!(download_concurrency, 16); // 默认
                assert_eq!(prefix.as_deref(), Some("photos/"));
            }
            _ => panic!("expected S3"),
        }
    }

    #[test]
    fn triggers_default_when_absent() {
        let toml = r#"
            [server]
            workdir = "/d"
            [storage]
            provider = "local"
            base_path = "/p"
        "#;
        let c = Config::from_toml_str(toml).unwrap();
        assert_eq!(c.triggers.poll_interval_secs, 0);
        assert!(c.triggers.webhook_token.is_none());
    }
}
