use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub site: serde_json::Value, // 原样透传给 __SITE_CONFIG__
    pub storage: StorageConfig,
    #[serde(default)]
    pub processing: ProcessingConfig,
    #[serde(default)]
    pub exif: ExifConfig,
    #[serde(default)]
    pub triggers: TriggersConfig,
    #[serde(default)]
    pub geocoding: GeocodingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_workdir")]
    pub workdir: PathBuf,
    #[serde(default)]
    pub dist_dir: PathBuf,
    /// 管理后台（/admin + 配置读写）的 Bearer token；None = 关闭 admin 接口。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_token: Option<String>,
}
fn default_listen() -> String {
    "0.0.0.0:8080".to_string()
}
fn default_workdir() -> PathBuf {
    PathBuf::from("./data")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageConfig {
    Local {
        base_path: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exclude_regex: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_file_limit: Option<usize>,
    },
    /// S3 及 S3 兼容（AWS / MinIO / Cloudflare R2 / Wasabi 等，通过 endpoint）。
    S3 {
        bucket: String,
        #[serde(default = "default_region")]
        region: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        access_key_id: String,
        secret_access_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_domain: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exclude_regex: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// 反查地理编码（GPS → 城市/国家）。默认关闭：构建期会发网络请求、拖慢构建。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeocodingConfig {
    #[serde(default)]
    pub enabled: bool,
    /// auto = 有 mapbox_token 用 Mapbox，否则回退 Nominatim。
    #[serde(default)]
    pub provider: GeoProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapbox_token: Option<String>,
    #[serde(default = "default_nominatim_url")]
    pub nominatim_base_url: String,
    /// 可选 BCP47 语言（如 "zh" / "en"）；None = 用服务默认。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// 坐标缓存精度（小数位）；4 ≈ 11m，相同坐标只查一次。
    #[serde(default = "default_cache_precision")]
    pub cache_precision: usize,
}
fn default_nominatim_url() -> String {
    "https://nominatim.openstreetmap.org".to_string()
}
fn default_cache_precision() -> usize {
    4
}
impl Default for GeocodingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: GeoProvider::Auto,
            mapbox_token: None,
            nominatim_base_url: default_nominatim_url(),
            language: None,
            cache_precision: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GeoProvider {
    #[default]
    Auto,
    Mapbox,
    Nominatim,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TriggersConfig {
    /// 定时轮询间隔（秒）；0 = 关闭轮询。
    #[serde(default)]
    pub poll_interval_secs: u64,
    /// webhook / 手动触发的 Bearer token；None = 关闭 webhook 与 admin 触发。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_token: Option<String>,
    /// 是否启用 S3 事件通知端点。
    #[serde(default)]
    pub enable_s3_event: bool,
}

impl Config {
    pub fn from_toml_str(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(|e| Error::Config(e.to_string()))
    }
    pub fn to_toml_string(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))
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
            admin_token = "admin-secret"
            [storage.local]
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
        assert_eq!(c.server.admin_token.as_deref(), Some("admin-secret"));
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
            [storage.s3]
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
            [storage.local]
            base_path = "/p"
        "#;
        let c = Config::from_toml_str(toml).unwrap();
        assert_eq!(c.triggers.poll_interval_secs, 0);
        assert!(c.triggers.webhook_token.is_none());
    }

    #[test]
    fn geocoding_defaults_and_parse() {
        // 缺省段 → 默认关闭、auto、Nominatim 公共实例
        let base = r#"
            [server]
            workdir = "/d"
            [storage.local]
            base_path = "/p"
        "#;
        let c = Config::from_toml_str(base).unwrap();
        assert!(!c.geocoding.enabled);
        assert_eq!(c.geocoding.provider, GeoProvider::Auto);
        assert_eq!(c.geocoding.cache_precision, 4);
        assert!(c.geocoding.mapbox_token.is_none());

        let with = r#"
            [server]
            workdir = "/d"
            [storage.local]
            base_path = "/p"
            [geocoding]
            enabled = true
            provider = "mapbox"
            mapbox_token = "pk.xxx"
            language = "zh"
            cache_precision = 3
        "#;
        let c = Config::from_toml_str(with).unwrap();
        assert!(c.geocoding.enabled);
        assert_eq!(c.geocoding.provider, GeoProvider::Mapbox);
        assert_eq!(c.geocoding.mapbox_token.as_deref(), Some("pk.xxx"));
        assert_eq!(c.geocoding.language.as_deref(), Some("zh"));
        assert_eq!(c.geocoding.cache_precision, 3);
        // 写回再解析等价
        let c2 = Config::from_toml_str(&c.to_toml_string().unwrap()).unwrap();
        assert_eq!(c2.geocoding.provider, GeoProvider::Mapbox);
        assert!(c2.geocoding.enabled);
    }

    #[test]
    fn toml_round_trip() {
        // 写回 TOML 再解析应等价（externally-tagged storage + Option 跳过 None）
        let toml = r#"
            [server]
            listen = "127.0.0.1:9000"
            workdir = "/d"
            [storage.s3]
            bucket = "b"
            access_key_id = "AK"
            secret_access_key = "SK"
        "#;
        let c = Config::from_toml_str(toml).unwrap();
        let out = c.to_toml_string().unwrap();
        let c2 = Config::from_toml_str(&out).unwrap();
        assert_eq!(c2.server.listen, "127.0.0.1:9000");
        match c2.storage {
            StorageConfig::S3 { bucket, region, .. } => {
                assert_eq!(bucket, "b");
                assert_eq!(region, "us-east-1");
            }
            _ => panic!("expected S3"),
        }
    }
}
