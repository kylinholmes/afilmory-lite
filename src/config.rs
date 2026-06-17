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
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_workdir")]
    pub workdir: PathBuf,
    #[serde(default)]
    pub dist_dir: PathBuf,
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
    // M2 追加：S3 { ... }
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
}
fn default_concurrency() -> usize {
    10
}
fn default_thumb_width() -> u32 {
    600
}
fn default_thumb_quality() -> u8 {
    100
}
impl Default for ProcessingConfig {
    fn default() -> Self {
        Self { concurrency: 10, thumbnail_width: 600, thumbnail_quality: 100, digest_suffix_length: 0 }
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
        Self { exiftool_path: "exiftool".into() }
    }
}

impl Config {
    pub fn from_toml_str(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(|e| Error::Config(e.to_string()))
    }
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).map_err(|e| Error::Io { path: path.to_path_buf(), source: e })?;
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
        "#;
        let c = Config::from_toml_str(toml).unwrap();
        assert_eq!(c.processing.concurrency, 4);
        assert_eq!(c.processing.thumbnail_width, 600); // 默认
        match c.storage {
            StorageConfig::Local { base_path, .. } => assert_eq!(base_path, PathBuf::from("/photos")),
        }
    }
}
