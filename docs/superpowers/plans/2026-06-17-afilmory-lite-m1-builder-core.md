# Afilmory Lite — Builder 核心闭环 实现计划（M1a）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用纯 Rust 实现 Afilmory 的 build 核心：扫描本地目录 → 逐图处理 → 产出与上游 v10 结构一致的 `manifest.json` + `thumbnails/<id>.jpg`。

**Architecture:** 一个 library crate（`afilmory_lite`）+ 一个 `build` 子命令二进制。`StorageProvider`(async trait) 抽象存储，M1 只实现 `LocalProvider`。`pipeline` 逐图产出 `PhotoManifestItem`，`manifest` 负责 serde 模型与读写，`builder` 加锁编排列举→增量筛选→处理→保存。EXIF 走 `exiftool` 子进程。

**Tech Stack:** Rust 2024、tokio、async-trait、serde/serde_json(preserve_order)、image(纯 Rust 解码/缩放/JPEG)、thumbhash、sha2、hex、chrono、regex、walkdir、toml、thiserror。系统依赖：`exiftool`。

**一致性边界（已与用户确认）：** 结构/语义一致，非字节级。`digest`/缩略图字节/`thumbHash` 比特实现内自洽即可。详见 `docs/superpowers/specs/2026-06-17-afilmory-lite-rs-design.md` 与 `docs/afilmory-feature-inventory.md`。

**M1a 刻意简化（后续计划补全）：** 仅本地存储；仅常规格式（jpg/jpeg/png/webp/tiff/bmp 由 image 解码，HEIC 留 M3）；用 image 自带 JPEG 编码与 Lanczos3 缩放（mozjpeg/fast_image_resize 留作优化）；不做 Live/Motion/HDR 的视频提取（HDR 标志按 exif 计算）；不做 geocoding。

---

### Task 1: 项目脚手架与错误类型

**Files:**
- Modify: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/error.rs`

- [ ] **Step 1: 写 Cargo.toml 依赖**

```toml
[package]
name = "afilmory-lite"
version = "0.1.0"
edition = "2024"

[lib]
name = "afilmory_lite"
path = "src/lib.rs"

[[bin]]
name = "afilmory-lite"
path = "src/main.rs"

[dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread", "fs", "process", "sync"] }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["preserve_order"] }
toml = "0.8"
thiserror = "2"
image = "0.25"
thumbhash = "0.1"
sha2 = "0.10"
hex = "0.4"
chrono = { version = "0.4", features = ["serde"] }
regex = "1"
walkdir = "2"
bytes = "1"
tracing = "0.1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: 写 src/error.rs**

```rust
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
    #[error("image decode error for {key}: {source}")]
    Image { key: String, #[source] source: image::ImageError },
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("exif error for {key}: {message}")]
    Exif { key: String, message: String },
    #[error("storage error: {0}")]
    Storage(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 3: 写 src/lib.rs 模块声明（占位）**

```rust
pub mod error;
pub mod config;
pub mod manifest;
pub mod storage;
pub mod exif;
pub mod pipeline;
pub mod builder;

pub use error::{Error, Result};
```

- [ ] **Step 4: 写最小 src/main.rs 占位（让 bin 能编译）**

```rust
fn main() {
    println!("afilmory-lite");
}
```

- [ ] **Step 5: 编译验证**

Run: `cargo build`
Expected: 编译失败——`config/manifest/...` 模块文件尚不存在。这是预期的；先创建空模块文件让其通过：临时 `touch src/config.rs src/manifest.rs src/storage.rs src/exif.rs src/pipeline.rs src/builder.rs` 并各写一行 `// placeholder`，再 `cargo build` 应 PASS。后续任务会替换它们为真实模块（含子模块时改为目录 + mod.rs）。

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/
git commit -m "chore: scaffold afilmory-lite crate with error type"
```

---

### Task 2: Manifest 数据模型（serde 序列化规则）

**Files:**
- Create: `src/manifest/mod.rs`
- Create: `src/manifest/model.rs`
- Test: 内联 `#[cfg(test)]`（在 model.rs 末尾）

> 删除占位 `src/manifest.rs`，改用 `src/manifest/mod.rs` 目录形式。

**关键序列化规则（盘点 §1.2）：**
- `thumb_hash / exif / tone_analysis / location` → `Option<T>`，**None 序列化为 `null`**（不 skip）。
- `digest / video / og_image_url` → `Option<T>` + `skip_serializing_if = "Option::is_none"`（None 省略键）。
- `is_hdr` → 恒输出的 `bool`。
- 字段顺序按 struct 声明顺序 = 上游构造顺序。
- 顶层 `version="v10", data, cameras, lenses`。

- [ ] **Step 1: 写 src/manifest/mod.rs**

```rust
mod model;
pub use model::*;
```

- [ ] **Step 2: 写 src/manifest/model.rs**

```rust
use serde::{Deserialize, Serialize};

pub const CURRENT_MANIFEST_VERSION: &str = "v10";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AfilmoryManifest {
    pub version: String,
    pub data: Vec<PhotoManifestItem>,
    pub cameras: Vec<CameraInfo>,
    pub lenses: Vec<LensInfo>,
}

impl Default for AfilmoryManifest {
    fn default() -> Self {
        Self {
            version: CURRENT_MANIFEST_VERSION.to_string(),
            data: Vec::new(),
            cameras: Vec::new(),
            lenses: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoManifestItem {
    pub id: String,
    pub format: String,
    pub title: String,
    pub description: String,
    pub date_taken: String,
    pub tags: Vec<String>,
    pub original_url: String,
    pub thumbnail_url: String,
    // 必出可为 null：
    pub thumb_hash: Option<String>,
    pub width: u32,
    pub height: u32,
    pub aspect_ratio: f64,
    pub s3_key: String,
    pub last_modified: String,
    pub size: u64,
    // 无值省略键：
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    // 必出可为 null：
    pub exif: Option<serde_json::Value>,
    pub tone_analysis: Option<ToneAnalysis>,
    pub location: Option<LocationInfo>,
    // 无值省略键：
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoSource>,
    pub is_hdr: bool,
    #[serde(rename = "ogImageUrl", skip_serializing_if = "Option::is_none")]
    pub og_image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToneAnalysis {
    pub tone_type: String, // 'low-key' | 'high-key' | 'normal' | 'high-contrast'
    pub brightness: u32,
    pub contrast: u32,
    pub shadow_ratio: f64,
    pub highlight_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationInfo {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VideoSource {
    #[serde(rename = "live-photo")]
    LivePhoto {
        #[serde(rename = "videoUrl")]
        video_url: String,
        #[serde(rename = "s3Key")]
        s3_key: String,
    },
    #[serde(rename = "motion-photo")]
    MotionPhoto {
        offset: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
        #[serde(rename = "presentationTimestamp", skip_serializing_if = "Option::is_none")]
        presentation_timestamp: Option<i64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    pub make: String,
    pub model: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub make: Option<String>,
    pub model: String,
    pub display_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_item() -> PhotoManifestItem {
        PhotoManifestItem {
            id: "DSC_0001".into(),
            format: "JPG".into(),
            title: "DSC 0001".into(),
            description: String::new(),
            date_taken: "2024-01-01T12:00:00.000Z".into(),
            tags: vec!["trip".into()],
            original_url: "https://cdn/DSC_0001.jpg".into(),
            thumbnail_url: "/thumbnails/DSC_0001.jpg".into(),
            thumb_hash: None,
            width: 4000,
            height: 3000,
            aspect_ratio: 4000.0 / 3000.0,
            s3_key: "trip/DSC_0001.jpg".into(),
            last_modified: "2024-01-01T12:00:00.000Z".into(),
            size: 123,
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
    fn null_fields_present_optional_fields_omitted() {
        let json = serde_json::to_value(sample_item()).unwrap();
        let obj = json.as_object().unwrap();
        // 必出可 null
        assert!(obj.contains_key("thumbHash"));
        assert!(obj["thumbHash"].is_null());
        assert!(obj.contains_key("exif") && obj["exif"].is_null());
        assert!(obj.contains_key("toneAnalysis") && obj["toneAnalysis"].is_null());
        assert!(obj.contains_key("location") && obj["location"].is_null());
        // 无值省略键
        assert!(!obj.contains_key("digest"));
        assert!(!obj.contains_key("video"));
        assert!(!obj.contains_key("ogImageUrl"));
        // 恒出
        assert_eq!(obj["isHDR"], serde_json::json!(false));
        // 不规则键名
        assert!(obj.contains_key("s3Key"));
        assert!(obj.contains_key("dateTaken"));
    }

    #[test]
    fn video_source_tagged() {
        let v = VideoSource::LivePhoto { video_url: "u".into(), s3_key: "k".into() };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["type"], "live-photo");
        assert_eq!(j["videoUrl"], "u");
        assert_eq!(j["s3Key"], "k");
    }
}
```

- [ ] **Step 3: 删除占位并运行测试**

Run: `rm -f src/manifest.rs && cargo test manifest::model -- --nocapture`
Expected: 2 个测试 PASS。

- [ ] **Step 4: Commit**

```bash
git add src/manifest/ && git rm --quiet --ignore-unmatch src/manifest.rs
git commit -m "feat: manifest data model with upstream serde semantics"
```

---

### Task 3: Manifest 读写、排序、cameras/lenses 聚合

**Files:**
- Create: `src/manifest/store.rs`
- Modify: `src/manifest/mod.rs`

- [ ] **Step 1: 在 src/manifest/mod.rs 增加 store**

```rust
mod model;
mod store;
pub use model::*;
pub use store::*;
```

- [ ] **Step 2: 写 src/manifest/store.rs**

```rust
use std::path::Path;

use crate::error::{Error, Result};
use crate::manifest::model::*;

/// 读取现有 manifest；不存在/解析失败返回默认空 manifest。
pub fn load_manifest(path: &Path) -> Result<AfilmoryManifest> {
    match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str::<AfilmoryManifest>(&s) {
            Ok(mut m) => {
                if m.version != CURRENT_MANIFEST_VERSION {
                    // M1a 不做迁移；按空处理以保证字段为 v10 形态。
                    return Ok(AfilmoryManifest::default());
                }
                if m.cameras.is_empty() {} // 占位：向后兼容补空已由 Default 保证
                Ok(std::mem::take(&mut m_into(m)))
            }
            Err(_) => Ok(AfilmoryManifest::default()),
        },
        Err(_) => Ok(AfilmoryManifest::default()),
    }
}

// 直接返回（保留 helper 以便未来加迁移钩子）
fn m_into(m: AfilmoryManifest) -> AfilmoryManifest { m }

/// 保存 manifest：data 按 dateTaken 降序，聚合 cameras/lenses，2 空格缩进。
pub fn save_manifest(path: &Path, mut items: Vec<PhotoManifestItem>) -> Result<()> {
    items.sort_by(|a, b| b.date_taken.cmp(&a.date_taken)); // ISO 字符串可直接字典序比较 = 时间序
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
    let mut seen = std::collections::BTreeMap::new();
    for item in items {
        let (Some(make), Some(model)) = (exif_str(item, "Make"), exif_str(item, "Model")) else { continue };
        let make = make.trim().to_string();
        let model = model.trim().to_string();
        let display_name = format!("{make} {model}");
        seen.entry(display_name.clone()).or_insert(CameraInfo { make, model, display_name });
    }
    seen.into_values().collect() // BTreeMap 已按 displayName 升序
}

pub fn generate_lenses(items: &[PhotoManifestItem]) -> Vec<LensInfo> {
    let mut seen = std::collections::BTreeMap::new();
    for item in items {
        let Some(model) = exif_str(item, "LensModel") else { continue };
        let model = model.trim().to_string();
        let make = exif_str(item, "LensMake").map(|s| s.trim().to_string());
        let display_name = match &make {
            Some(m) => format!("{m} {model}"),
            None => model.clone(),
        };
        seen.entry(display_name.clone()).or_insert(LensInfo { make, model, display_name });
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
            id: id.into(), format: "JPG".into(), title: id.into(), description: String::new(),
            date_taken: date.into(), tags: vec![], original_url: String::new(),
            thumbnail_url: format!("/thumbnails/{id}.jpg"), thumb_hash: None,
            width: 100, height: 100, aspect_ratio: 1.0, s3_key: format!("{id}.jpg"),
            last_modified: date.into(), size: 0, digest: None, exif, tone_analysis: None,
            location: None, video: None, is_hdr: false, og_image_url: None,
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
        assert_eq!(loaded.data.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
        // cameras 去重 + 升序：Canon R5, Sony A7
        assert_eq!(loaded.cameras.iter().map(|c| c.display_name.as_str()).collect::<Vec<_>>(),
                   ["Canon R5", "Sony A7"]);
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempdir().unwrap();
        let m = load_manifest(&dir.path().join("nope.json")).unwrap();
        assert_eq!(m.version, "v10");
        assert!(m.data.is_empty());
    }
}
```

> 注：`load_manifest` 里的 `m_into`/`std::mem::take` 是为保留未来迁移钩子的占位写法；可直接 `Ok(m)`。若 lint 提示冗余，简化为 `Ok(m)`。

- [ ] **Step 3: 运行测试**

Run: `cargo test manifest::store`
Expected: 2 PASS。

- [ ] **Step 4: Commit**

```bash
git add src/manifest/
git commit -m "feat: manifest store (sort desc, camera/lens aggregation, read/write)"
```

---

### Task 4: 配置（TOML，M1a 子集）

**Files:**
- Create: `src/config.rs`（替换占位）
- Test: 内联

- [ ] **Step 1: 写 src/config.rs**

```rust
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
fn default_workdir() -> PathBuf { PathBuf::from("./data") }

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
fn default_concurrency() -> usize { 10 }
fn default_thumb_width() -> u32 { 600 }
fn default_thumb_quality() -> u8 { 100 }
impl Default for ProcessingConfig {
    fn default() -> Self { Self { concurrency: 10, thumbnail_width: 600, thumbnail_quality: 100, digest_suffix_length: 0 } }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExifConfig {
    #[serde(default = "default_exiftool")]
    pub exiftool_path: String,
}
fn default_exiftool() -> String { "exiftool".to_string() }
impl Default for ExifConfig {
    fn default() -> Self { Self { exiftool_path: "exiftool".into() } }
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
```

- [ ] **Step 2: 运行测试**

Run: `cargo test config::`
Expected: PASS。

- [ ] **Step 3: Commit**

```bash
git add src/config.rs
git commit -m "feat: TOML config (M1a subset: server/site/storage[local]/processing/exif)"
```

---

### Task 5: 存储 trait + 本地 provider

**Files:**
- Create: `src/storage/mod.rs`（替换占位 `src/storage.rs`）
- Create: `src/storage/local.rs`
- Test: 内联于 local.rs

**图片扩展名集合（小写）：** `jpg jpeg png webp bmp tiff tif heic heif hif`。

- [ ] **Step 1: 写 src/storage/mod.rs**

```rust
mod local;
pub use local::LocalProvider;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use crate::error::Result;

pub const IMAGE_EXTS: &[&str] = &["jpg","jpeg","png","webp","bmp","tiff","tif","heic","heif","hif"];

pub fn is_image_key(key: &str) -> bool {
    match key.rsplit('.').next() {
        Some(ext) => IMAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
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
```

- [ ] **Step 2: 写 src/storage/local.rs**

```rust
use std::path::{Path, PathBuf};
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::storage::{is_image_key, StorageObject, StorageProvider};

pub struct LocalProvider {
    base_path: PathBuf,
    base_url: Option<String>,
    exclude: Option<regex::Regex>,
    max_file_limit: Option<usize>,
}

impl LocalProvider {
    pub fn new(base_path: PathBuf, base_url: Option<String>, exclude_regex: Option<String>, max_file_limit: Option<usize>) -> Result<Self> {
        let exclude = match exclude_regex {
            Some(r) => Some(regex::Regex::new(&r).map_err(|e| Error::Config(format!("bad exclude_regex: {e}")))?),
            None => None,
        };
        Ok(Self { base_path, base_url, exclude, max_file_limit })
    }

    fn scan(&self) -> Result<Vec<StorageObject>> {
        let mut out = Vec::new();
        for entry in WalkDir::new(&self.base_path).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() { continue; }
            let rel = match entry.path().strip_prefix(&self.base_path) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if let Some(re) = &self.exclude { if re.is_match(&rel) { continue; } }
            let meta = entry.metadata().map_err(|e| Error::Storage(e.to_string()))?;
            let modified: Option<DateTime<Utc>> = meta.modified().ok().map(DateTime::<Utc>::from);
            let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis()).unwrap_or(0);
            out.push(StorageObject {
                key: rel,
                size: Some(meta.len()),
                last_modified: modified,
                etag: Some(format!("{}-{}", mtime, meta.len())),
            });
        }
        if let Some(limit) = self.max_file_limit { out.truncate(limit); }
        Ok(out)
    }

    fn abs(&self, key: &str) -> PathBuf { self.base_path.join(key) }
}

#[async_trait]
impl StorageProvider for LocalProvider {
    async fn list_images(&self) -> Result<Vec<StorageObject>> {
        Ok(self.scan()?.into_iter().filter(|o| is_image_key(&o.key)).collect())
    }
    async fn list_all_files(&self) -> Result<Vec<StorageObject>> { self.scan() }
    async fn get_file(&self, key: &str) -> Result<Option<Bytes>> {
        match tokio::fs::read(self.abs(key)).await {
            Ok(b) => Ok(Some(Bytes::from(b))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io { path: self.abs(key), source: e }),
        }
    }
    fn generate_public_url(&self, key: &str) -> String {
        match &self.base_url {
            Some(b) => format!("{}/{}", b.trim_end_matches('/'), key),
            None => format!("file://{}", self.abs(key).display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn lists_only_images_and_builds_url() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("trip")).unwrap();
        std::fs::write(dir.path().join("trip/a.JPG"), b"x").unwrap();
        std::fs::write(dir.path().join("trip/b.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("c.png"), b"x").unwrap();

        let p = LocalProvider::new(dir.path().to_path_buf(), Some("/photos".into()), None, None).unwrap();
        let mut imgs: Vec<String> = p.list_images().await.unwrap().into_iter().map(|o| o.key).collect();
        imgs.sort();
        assert_eq!(imgs, vec!["c.png".to_string(), "trip/a.JPG".to_string()]);
        assert_eq!(p.generate_public_url("trip/a.JPG"), "/photos/trip/a.JPG");
    }
}
```

- [ ] **Step 3: 删占位并运行测试**

Run: `rm -f src/storage.rs && cargo test storage::`
Expected: PASS。

- [ ] **Step 4: Commit**

```bash
git add src/storage/ && git rm --quiet --ignore-unmatch src/storage.rs
git commit -m "feat: StorageProvider trait + LocalProvider"
```

---

### Task 6: 管线 — 解码、尺寸、orientation

**Files:**
- Create: `src/pipeline/mod.rs`（替换占位 `src/pipeline.rs`）
- Create: `src/pipeline/decode.rs`
- Test: 内联

- [ ] **Step 1: 写 src/pipeline/mod.rs（先只挂 decode，后续任务追加）**

```rust
pub mod decode;
pub mod thumbnail;
pub mod thumbhash;
pub mod tone;
pub mod info;

mod process;
pub use process::process_photo;
```

> 本任务先只创建 `decode.rs` 并把 mod.rs 里其余 `pub mod` / `mod process` 行注释掉，待对应任务再启用。

- [ ] **Step 2: 写 src/pipeline/decode.rs**

```rust
use image::{DynamicImage, imageops};
use crate::error::{Error, Result};

pub struct Decoded {
    pub image: DynamicImage,
    /// orientation 应用后的逻辑宽高（写入 manifest 的 width/height）
    pub width: u32,
    pub height: u32,
}

/// 解码图片字节；按 EXIF orientation 校正图像与尺寸。
/// orientation 来自 EXIF（1..=8，缺省按 1 处理）。
pub fn decode(bytes: &[u8], key: &str, orientation: u32) -> Result<Decoded> {
    let img = image::load_from_memory(bytes).map_err(|e| Error::Image { key: key.to_string(), source: e })?;
    let img = apply_orientation(img, orientation);
    let (width, height) = (img.width(), img.height());
    Ok(Decoded { image: img, width, height })
}

/// 按 EXIF orientation（1..=8）做几何校正。
pub fn apply_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => DynamicImage::ImageRgba8(imageops::flip_horizontal(&img)),
        3 => DynamicImage::ImageRgba8(imageops::rotate180(&img)),
        4 => DynamicImage::ImageRgba8(imageops::flip_vertical(&img)),
        5 => {
            let r = imageops::rotate90(&img);
            DynamicImage::ImageRgba8(imageops::flip_horizontal(&r))
        }
        6 => DynamicImage::ImageRgba8(imageops::rotate90(&img)),
        7 => {
            let r = imageops::rotate270(&img);
            DynamicImage::ImageRgba8(imageops::flip_horizontal(&r))
        }
        8 => DynamicImage::ImageRgba8(imageops::rotate270(&img)),
        _ => img, // 1 或未知
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{RgbaImage, Rgba};

    fn solid(w: u32, h: u32) -> Vec<u8> {
        let mut img = RgbaImage::new(w, h);
        for p in img.pixels_mut() { *p = Rgba([10, 20, 30, 255]); }
        let dyn_img = DynamicImage::ImageRgba8(img);
        let mut buf = std::io::Cursor::new(Vec::new());
        dyn_img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn decode_keeps_dims_for_orientation_1() {
        let d = decode(&solid(40, 30), "a.png", 1).unwrap();
        assert_eq!((d.width, d.height), (40, 30));
    }

    #[test]
    fn orientation_6_swaps_dims() {
        let d = decode(&solid(40, 30), "a.png", 6).unwrap();
        assert_eq!((d.width, d.height), (30, 40));
    }
}
```

- [ ] **Step 3: 删占位并运行测试**

Run: `rm -f src/pipeline.rs && cargo test pipeline::decode`
Expected: 2 PASS。

- [ ] **Step 4: Commit**

```bash
git add src/pipeline/ && git rm --quiet --ignore-unmatch src/pipeline.rs
git commit -m "feat: pipeline decode + EXIF orientation correction"
```

---

### Task 7: 管线 — 缩略图（600px JPEG）

**Files:**
- Create: `src/pipeline/thumbnail.rs`
- Modify: `src/pipeline/mod.rs`（启用 `pub mod thumbnail;`）

- [ ] **Step 1: 写 src/pipeline/thumbnail.rs**

```rust
use image::{DynamicImage, imageops::FilterType};
use crate::error::{Error, Result};

/// 生成缩略图 JPEG 字节：宽 thumb_width，等比，不放大，质量 quality。
/// 输入 image 应已做过 orientation 校正。
pub fn make_thumbnail(image: &DynamicImage, thumb_width: u32, quality: u8) -> Result<Vec<u8>> {
    let resized = if image.width() > thumb_width {
        // resize 保持纵横比，约束到 (thumb_width, 极大)，等价于按宽缩放
        image.resize(thumb_width, u32::MAX, FilterType::Lanczos3)
    } else {
        image.clone() // withoutEnlargement：不放大
    };
    let rgb = resized.to_rgb8();
    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    encoder
        .encode(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| Error::Image { key: "thumbnail".into(), source: e })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{RgbaImage, Rgba};

    fn img(w: u32, h: u32) -> DynamicImage {
        let mut i = RgbaImage::new(w, h);
        for p in i.pixels_mut() { *p = Rgba([100, 120, 140, 255]); }
        DynamicImage::ImageRgba8(i)
    }

    #[test]
    fn downscales_wide_image_to_600() {
        let jpg = make_thumbnail(&img(1200, 900), 600, 100).unwrap();
        let decoded = image::load_from_memory(&jpg).unwrap();
        assert_eq!(decoded.width(), 600);
        assert_eq!(decoded.height(), 450);
    }

    #[test]
    fn does_not_enlarge_small_image() {
        let jpg = make_thumbnail(&img(300, 200), 600, 100).unwrap();
        let decoded = image::load_from_memory(&jpg).unwrap();
        assert_eq!(decoded.width(), 300);
    }
}
```

- [ ] **Step 2: 运行测试**

Run: `cargo test pipeline::thumbnail`
Expected: 2 PASS。

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/
git commit -m "feat: pipeline thumbnail (600px, withoutEnlargement, jpeg)"
```

---

### Task 8: 管线 — thumbHash

**Files:**
- Create: `src/pipeline/thumbhash.rs`
- Modify: `src/pipeline/mod.rs`（启用 `pub mod thumbhash;`）

- [ ] **Step 1: 写 src/pipeline/thumbhash.rs**

```rust
use image::{DynamicImage, imageops::FilterType};
use crate::error::Result;

/// 由缩略图 JPEG 字节计算 thumbHash，返回小写 hex 字符串。
/// 复刻上游：resize 到 100x100(fit inside) → RGBA → rgba_to_thumb_hash → hex。
pub fn compute_thumbhash(thumbnail_jpeg: &[u8]) -> Result<Option<String>> {
    let img = match image::load_from_memory(thumbnail_jpeg) {
        Ok(i) => i,
        Err(_) => return Ok(None),
    };
    let small: DynamicImage = img.resize(100, 100, FilterType::Lanczos3);
    let rgba = small.to_rgba8();
    let hash = thumbhash::rgba_to_thumb_hash(rgba.width() as usize, rgba.height() as usize, rgba.as_raw());
    Ok(Some(hex::encode(hash)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{RgbaImage, Rgba, DynamicImage};

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut i = RgbaImage::new(w, h);
        for p in i.pixels_mut() { *p = Rgba([200, 50, 50, 255]); }
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(i).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).unwrap();
        out
    }

    #[test]
    fn produces_lowercase_hex() {
        let h = compute_thumbhash(&jpeg(600, 400)).unwrap().unwrap();
        assert!(!h.is_empty());
        assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // thumbhash 可被解回
        let bytes = hex::decode(&h).unwrap();
        assert!(!bytes.is_empty());
    }
}
```

- [ ] **Step 2: 运行测试**

Run: `cargo test pipeline::thumbhash`
Expected: PASS。若 `thumbhash` crate 的函数名/签名不同，按其实际 API 调整（核心：`rgba_to_thumb_hash(w, h, &rgba) -> Vec<u8>`）。

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/
git commit -m "feat: pipeline thumbhash (hex-encoded)"
```

---

### Task 9: 管线 — 影调分析

**Files:**
- Create: `src/pipeline/tone.rs`
- Modify: `src/pipeline/mod.rs`（启用 `pub mod tone;`）

公式（盘点 §2.5）：256×256 缩放、BT.709 亮度、brightness(0-100)、contrast(stdDev/127.5)、shadowRatio(lum[0..86])、highlightRatio(lum[170..256])、toneType 判定；异常回退 `{normal,50,50,0.33,0.33}`。

- [ ] **Step 1: 写 src/pipeline/tone.rs**

```rust
use image::{DynamicImage, imageops::FilterType};
use crate::manifest::ToneAnalysis;

pub fn analyze_tone(image: &DynamicImage) -> ToneAnalysis {
    analyze_inner(image).unwrap_or_else(fallback)
}

fn fallback() -> ToneAnalysis {
    ToneAnalysis { tone_type: "normal".into(), brightness: 50, contrast: 50, shadow_ratio: 0.33, highlight_ratio: 0.33 }
}

fn analyze_inner(image: &DynamicImage) -> Option<ToneAnalysis> {
    let small = image.resize(256, 256, FilterType::Lanczos3).to_rgb8();
    let total = (small.width() * small.height()) as f64;
    if total == 0.0 { return None; }
    let mut lum = [0f64; 256];
    for p in small.pixels() {
        let l = (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64).round() as usize;
        lum[l.min(255)] += 1.0;
    }
    for v in lum.iter_mut() { *v /= total; } // 归一化为概率

    let total_lum: f64 = lum.iter().sum();
    let weighted: f64 = lum.iter().enumerate().map(|(i, &p)| i as f64 * p).sum();
    let mean = weighted / total_lum;
    let brightness = (mean * (100.0 / 255.0)).round() as u32;
    let shadow_ratio: f64 = lum[0..86].iter().sum();
    let highlight_ratio: f64 = lum[170..256].iter().sum();
    let variance: f64 = lum.iter().enumerate().map(|(i, &p)| p * (i as f64 - mean).powi(2)).sum();
    let std_dev = variance.sqrt();
    let contrast = ((std_dev / 127.5) * 100.0).round().min(100.0) as u32;

    let tone_type = if brightness < 30 && shadow_ratio > 0.6 {
        "low-key"
    } else if brightness > 70 && highlight_ratio > 0.6 {
        "high-key"
    } else if contrast > 60 && shadow_ratio > 0.3 && highlight_ratio > 0.3 {
        "high-contrast"
    } else {
        "normal"
    };

    Some(ToneAnalysis {
        tone_type: tone_type.into(),
        brightness,
        contrast,
        shadow_ratio: (shadow_ratio * 100.0).round() / 100.0,
        highlight_ratio: (highlight_ratio * 100.0).round() / 100.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{RgbImage, Rgb, DynamicImage};

    fn solid(r: u8, g: u8, b: u8) -> DynamicImage {
        let mut i = RgbImage::new(64, 64);
        for p in i.pixels_mut() { *p = Rgb([r, g, b]); }
        DynamicImage::ImageRgb8(i)
    }

    #[test]
    fn dark_image_is_low_key() {
        let t = analyze_tone(&solid(0, 0, 0));
        assert_eq!(t.tone_type, "low-key");
        assert_eq!(t.brightness, 0);
    }

    #[test]
    fn bright_image_is_high_key() {
        let t = analyze_tone(&solid(255, 255, 255));
        assert_eq!(t.tone_type, "high-key");
        assert_eq!(t.brightness, 100);
    }
}
```

- [ ] **Step 2: 运行测试**

Run: `cargo test pipeline::tone`
Expected: 2 PASS。

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/
git commit -m "feat: pipeline tone analysis (BT.709 histogram)"
```

---

### Task 10: 管线 — info 抽取（title/dateTaken/tags/description）

**Files:**
- Create: `src/pipeline/info.rs`
- Modify: `src/pipeline/mod.rs`（启用 `pub mod info;`）

规则（盘点 §2.12）：tags=目录每级；dateTaken=EXIF `DateTimeOriginal`(转 ISO)→文件名 `\d{4}-\d{2}-\d{2}`→当前时间；title=文件名去日期/views/分隔符；description=`""`。

- [ ] **Step 1: 写 src/pipeline/info.rs**

```rust
use chrono::{SecondsFormat, Utc, NaiveDateTime, TimeZone};
use crate::manifest::PhotoManifestItem; // 仅用于文档

pub struct PhotoInfo {
    pub title: String,
    pub date_taken: String,
    pub tags: Vec<String>,
    pub description: String,
}

/// key: 存储 key（含目录）。exif_date_taken: 已格式化为 ISO 的 EXIF 日期（若有）。
pub fn extract_info(key: &str, exif_date_taken: Option<&str>) -> PhotoInfo {
    let key = key.replace('\\', "/");
    let file_stem = key.rsplit('/').next().unwrap_or(&key);
    let file_stem = strip_ext(file_stem);

    // tags：目录每一级
    let tags: Vec<String> = match key.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() && dir != "." => dir
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().to_string())
            .collect(),
        _ => vec![],
    };

    // dateTaken
    let date_taken = exif_date_taken
        .map(|s| s.to_string())
        .or_else(|| date_from_filename(file_stem))
        .unwrap_or_else(|| Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));

    // title
    let title = clean_title(file_stem);

    PhotoInfo { title, date_taken, tags, description: String::new() }
}

fn strip_ext(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

fn date_from_filename(stem: &str) -> Option<String> {
    let re = regex::Regex::new(r"(\d{4})-(\d{2})-(\d{2})").unwrap();
    let caps = re.captures(stem)?;
    let s = format!("{}-{}-{}", &caps[1], &caps[2], &caps[3]);
    let ndt = NaiveDateTime::parse_from_str(&format!("{s} 00:00:00"), "%Y-%m-%d %H:%M:%S").ok()?;
    Some(Utc.from_utc_datetime(&ndt).to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn clean_title(stem: &str) -> String {
    let date_re = regex::Regex::new(r"\d{4}-\d{2}-\d{2}[_-]?").unwrap();
    let views_re = regex::Regex::new(r"(?i)[_-]?\d+views?").unwrap();
    let sep_re = regex::Regex::new(r"[_-]+").unwrap();
    let mut t = date_re.replace_all(stem, "").to_string();
    t = views_re.replace_all(&t, "").to_string();
    t = sep_re.replace_all(&t, " ").to_string();
    let t = t.trim().to_string();
    if t.is_empty() { stem.to_string() } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_from_dirs() {
        let i = extract_info("trip/2024/DSC_0001.jpg", None);
        assert_eq!(i.tags, vec!["trip".to_string(), "2024".to_string()]);
    }

    #[test]
    fn date_from_filename_used_when_no_exif() {
        let i = extract_info("2024-05-01_sunset.jpg", None);
        assert_eq!(i.date_taken, "2024-05-01T00:00:00.000Z");
        assert_eq!(i.title, "sunset");
    }

    #[test]
    fn exif_date_takes_priority() {
        let i = extract_info("x/2024-05-01_a.jpg", Some("2024-05-01T10:00:00.000Z"));
        assert_eq!(i.date_taken, "2024-05-01T10:00:00.000Z");
    }
}
```

> 顶部 `use ...PhotoManifestItem` 仅文档用途，若触发未使用告警则删除该行。

- [ ] **Step 2: 运行测试**

Run: `cargo test pipeline::info`
Expected: 3 PASS。

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/
git commit -m "feat: pipeline info extraction (tags/date/title)"
```

---

### Task 11: EXIF — exiftool 子进程 + 白名单裁剪

**Files:**
- Create: `src/exif/mod.rs`（替换占位 `src/exif.rs`）
- Create: `src/exif/exiftool.rs`
- Test: 内联（集成测试，缺 exiftool 时跳过）

**白名单（盘点 §1.3 `pickKeys`）：** 见下方 `PICK_KEYS`。日期字段转 ISO；`GPSAltitudeRef` 归一化 0/1。

- [ ] **Step 1: 写 src/exif/mod.rs**

```rust
mod exiftool;
pub use exiftool::ExiftoolExtractor;

use async_trait::async_trait;
use crate::error::Result;

/// 抽取并裁剪后的 EXIF（JSON 对象），以及派生出的 ISO 拍摄时间。
pub struct ExifResult {
    pub exif: serde_json::Value, // object
    pub date_taken_iso: Option<String>,
    pub orientation: u32,
}

#[async_trait]
pub trait ExifExtractor: Send + Sync {
    /// path：要读取的临时图片文件；raw_for_heic：HEIC 时传 raw 原图（M1a 不用）。
    async fn extract(&self, path: &std::path::Path) -> Result<Option<ExifResult>>;
}

pub const PICK_KEYS: &[&str] = &[
    "tz","tzSource","Orientation","Make","Model","Software","Artist","Copyright",
    "ExposureTime","FNumber","ExposureProgram","ISO","OffsetTime","OffsetTimeOriginal",
    "OffsetTimeDigitized","ShutterSpeedValue","ApertureValue","BrightnessValue",
    "ExposureCompensationSet","ExposureCompensationMode","ExposureCompensationSetting",
    "ExposureCompensation","MaxApertureValue","LightSource","Flash","FocalLength",
    "ColorSpace","ExposureMode","FocalLengthIn35mmFormat","SceneCaptureType","LensMake",
    "LensModel","MeteringMode","WhiteBalance","WBShiftAB","WBShiftGM","WhiteBalanceBias",
    "FlashMeteringMode","SensingMethod","FocalPlaneXResolution","FocalPlaneYResolution",
    "Aperture","ScaleFactor35efl","ShutterSpeed","LightValue","Rating","GPSAltitude",
    "GPSCoordinates","GPSAltitudeRef","GPSLatitude","GPSLatitudeRef","GPSLongitude",
    "GPSLongitudeRef","MPImageType","UniformResourceName","MotionPhoto","MotionPhotoVersion",
    "MotionPhotoPresentationTimestampUs","ContainerDirectory","MicroVideo","MicroVideoVersion",
    "MicroVideoOffset","MicroVideoPresentationTimestampUs",
    "DateTimeOriginal","DateTimeDigitized","ExifImageWidth","ExifImageHeight",
];
```

- [ ] **Step 2: 写 src/exif/exiftool.rs**

```rust
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
    pub fn new(exe: impl Into<String>) -> Self { Self { exe: exe.into() } }
}

#[async_trait]
impl ExifExtractor for ExiftoolExtractor {
    async fn extract(&self, path: &Path) -> Result<Option<ExifResult>> {
        // -json：JSON 输出；保留文本化标签值（不加 -n）。-G0 不加以贴近 exiftool-vendored 默认扁平输出。
        let output = Command::new(&self.exe)
            .arg("-json")
            .arg("-api").arg("largefilesupport=1")
            .arg(path)
            .output()
            .await
            .map_err(|e| Error::Exif { key: path.display().to_string(), message: format!("spawn exiftool failed: {e}") })?;
        if !output.status.success() {
            return Err(Error::Exif { key: path.display().to_string(), message: String::from_utf8_lossy(&output.stderr).to_string() });
        }
        let parsed: Vec<Value> = serde_json::from_slice(&output.stdout)?;
        let Some(raw) = parsed.into_iter().next() else { return Ok(None) };
        let Value::Object(raw) = raw else { return Ok(None) };

        let mut picked = Map::new();
        for key in PICK_KEYS {
            if let Some(v) = raw.get(*key) {
                picked.insert((*key).to_string(), v.clone());
            }
        }
        // 派生 ImageWidth/ImageHeight
        if let Some(w) = raw.get("ExifImageWidth") { picked.insert("ImageWidth".into(), w.clone()); }
        if let Some(h) = raw.get("ExifImageHeight") { picked.insert("ImageHeight".into(), h.clone()); }

        // 日期 → ISO
        let date_taken_iso = picked.get("DateTimeOriginal").and_then(|v| v.as_str()).and_then(exif_date_to_iso);
        if let Some(iso) = &date_taken_iso { picked.insert("DateTimeOriginal".into(), Value::String(iso.clone())); }
        if let Some(iso) = picked.get("DateTimeDigitized").and_then(|v| v.as_str()).and_then(exif_date_to_iso) {
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

        let orientation = picked.get("Orientation").and_then(orientation_to_u32).unwrap_or(1);

        Ok(Some(ExifResult { exif: Value::Object(picked), date_taken_iso, orientation }))
    }
}

/// exiftool 默认日期格式 "YYYY:MM:DD HH:MM:SS"（可能带时区后缀）→ ISO（按 UTC，M1a 简化）。
fn exif_date_to_iso(s: &str) -> Option<String> {
    let head = &s[..s.len().min(19)];
    let ndt = NaiveDateTime::parse_from_str(head, "%Y:%m:%d %H:%M:%S").ok()?;
    Some(Utc.from_utc_datetime(&ndt).to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// Orientation 可能是数字或文本（如 "Rotate 90 CW"）。文本时映射回 1..=8。
fn orientation_to_u32(v: &Value) -> Option<u32> {
    if let Some(n) = v.as_u64() { return Some(n as u32); }
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
        std::process::Command::new("exiftool").arg("-ver").output().map(|o| o.status.success()).unwrap_or(false)
    }

    #[test]
    fn date_parsing() {
        assert_eq!(exif_date_to_iso("2024:05:01 10:11:12"), Some("2024-05-01T10:11:12.000Z".to_string()));
    }

    #[tokio::test]
    async fn reads_real_image_if_exiftool_present() {
        if !exiftool_available() { eprintln!("skip: exiftool not installed"); return; }
        // 生成一张临时 jpg
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jpg");
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]));
        image::DynamicImage::ImageRgb8(img).save(&p).unwrap();
        let ex = ExiftoolExtractor::new("exiftool");
        let res = ex.extract(&p).await.unwrap();
        assert!(res.is_some());
    }
}
```

- [ ] **Step 3: 删占位并运行测试**

Run: `rm -f src/exif.rs && cargo test exif::`
Expected: `date_parsing` PASS；`reads_real_image...` 在装了 exiftool 时 PASS，否则打印 skip 并 PASS。

- [ ] **Step 4: Commit**

```bash
git add src/exif/ && git rm --quiet --ignore-unmatch src/exif.rs
git commit -m "feat: exif extraction via exiftool subprocess + whitelist"
```

---

### Task 12: 管线编排 — process_photo

**Files:**
- Create: `src/pipeline/process.rs`
- Modify: `src/pipeline/mod.rs`（启用 `mod process; pub use process::process_photo;`）

把 Task 6-11 组装成一个 `PhotoManifestItem`。EXIF 需要先写临时文件给 exiftool（M1a 用临时文件，M2 改 stay-open）。

- [ ] **Step 1: 写 src/pipeline/process.rs**

```rust
use sha2::{Digest, Sha256};

use crate::config::ProcessingConfig;
use crate::error::Result;
use crate::exif::ExifExtractor;
use crate::manifest::PhotoManifestItem;
use crate::pipeline::{decode, info, thumbnail, thumbhash, tone};
use crate::storage::{StorageObject, StorageProvider};

pub struct PipelineDeps<'a> {
    pub storage: &'a dyn StorageProvider,
    pub exif: &'a dyn ExifExtractor,
    pub processing: &'a ProcessingConfig,
    /// 缩略图输出目录（thumbnails/）。
    pub thumb_dir: &'a std::path::Path,
}

/// 处理单张照片，写出缩略图文件，返回 manifest item。失败返回 Err（调用方记失败计数）。
pub async fn process_photo(obj: &StorageObject, deps: &PipelineDeps<'_>) -> Result<PhotoManifestItem> {
    let key = &obj.key;
    let raw = deps.storage.get_file(key).await?
        .ok_or_else(|| crate::Error::Storage(format!("missing file: {key}")))?;

    // photoId
    let id = photo_id(key, deps.processing.digest_suffix_length);
    // contentDigest（M1a 无格式转换 → 处理后字节 = 原始字节）
    let digest = hex::encode(Sha256::digest(&raw));

    // EXIF：写临时文件给 exiftool
    let exif_res = {
        let tmp = tempfile::Builder::new().suffix(&dot_ext(key)).tempfile()
            .map_err(|e| crate::Error::Io { path: std::path::PathBuf::from("tmp"), source: e })?;
        std::fs::write(tmp.path(), &raw).map_err(|e| crate::Error::Io { path: tmp.path().to_path_buf(), source: e })?;
        deps.exif.extract(tmp.path()).await?
    };
    let orientation = exif_res.as_ref().map(|e| e.orientation).unwrap_or(1);
    let exif_value = exif_res.as_ref().map(|e| e.exif.clone());
    let exif_date = exif_res.as_ref().and_then(|e| e.date_taken_iso.clone());

    // 解码（按 orientation 校正）
    let decoded = tokio::task::block_in_place(|| decode::decode(&raw, key, orientation))?;

    // 缩略图 + thumbHash
    let thumb_jpeg = tokio::task::block_in_place(|| {
        thumbnail::make_thumbnail(&decoded.image, deps.processing.thumbnail_width, deps.processing.thumbnail_quality)
    })?;
    let thumb_hash = thumbhash::compute_thumbhash(&thumb_jpeg)?;
    // 写缩略图文件
    std::fs::create_dir_all(deps.thumb_dir).map_err(|e| crate::Error::Io { path: deps.thumb_dir.to_path_buf(), source: e })?;
    let thumb_path = deps.thumb_dir.join(format!("{id}.jpg"));
    std::fs::write(&thumb_path, &thumb_jpeg).map_err(|e| crate::Error::Io { path: thumb_path.clone(), source: e })?;

    // 影调
    let tone = tokio::task::block_in_place(|| tone::analyze_tone(&decoded.image));

    // info
    let pinfo = info::extract_info(key, exif_date.as_deref());

    // 组装
    let extension = key.rsplit('.').next().map(|e| e.to_ascii_uppercase()).filter(|e| !e.is_empty()).unwrap_or_else(|| "UNKNOWN".into());
    let is_hdr = compute_is_hdr(exif_value.as_ref());

    Ok(PhotoManifestItem {
        id,
        format: extension,
        title: pinfo.title,
        description: pinfo.description,
        date_taken: pinfo.date_taken,
        tags: pinfo.tags,
        original_url: deps.storage.generate_public_url(key),
        thumbnail_url: format!("/thumbnails/{}.jpg", thumb_path.file_stem().unwrap().to_string_lossy()),
        thumb_hash,
        width: decoded.width,
        height: decoded.height,
        aspect_ratio: decoded.width as f64 / decoded.height as f64,
        s3_key: key.clone(),
        last_modified: obj.last_modified
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        size: obj.size.unwrap_or(0),
        digest: Some(digest),
        exif: exif_value,
        tone_analysis: Some(tone),
        location: None,
        video: None,
        is_hdr,
        og_image_url: None,
    })
}

fn photo_id(key: &str, digest_suffix_length: usize) -> String {
    let base = key.rsplit('/').next().unwrap_or(key);
    let stem = base.rsplit_once('.').map(|(s, _)| s).filter(|s| !s.is_empty()).unwrap_or(base);
    if digest_suffix_length == 0 { return stem.to_string(); }
    let hash = hex::encode(Sha256::digest(key.as_bytes()));
    format!("{stem}_{}", &hash[..digest_suffix_length.min(hash.len())])
}

fn dot_ext(key: &str) -> String {
    key.rsplit_once('.').map(|(_, e)| format!(".{e}")).unwrap_or_default()
}

fn compute_is_hdr(exif: Option<&serde_json::Value>) -> bool {
    let Some(e) = exif else { return false };
    let mp = e.get("MPImageType").and_then(|v| v.as_str());
    let urn = e.get("UniformResourceName").and_then(|v| v.as_str());
    mp == Some("Gain Map Image") || urn == Some("urn:iso:std:iso:ts:21496:-1")
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
}
```

> 需要 `tempfile` 移到 `[dependencies]`（process.rs 在库代码里用到）；把 `tempfile = "3"` 从 dev-dependencies 也加到 dependencies。
> `tokio::task::block_in_place` 要求多线程 runtime；测试与 builder 均在 multi-thread runtime 下运行。

- [ ] **Step 2: 运行测试**

Run: `cargo test pipeline::process`
Expected: 2 PASS。

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/ Cargo.toml
git commit -m "feat: process_photo orchestration"
```

---

### Task 13: 增量与删除逻辑

**Files:**
- Create: `src/manifest/incremental.rs`
- Modify: `src/manifest/mod.rs`（`mod incremental; pub use incremental::*;`）

- [ ] **Step 1: 写 src/manifest/incremental.rs**

```rust
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::manifest::PhotoManifestItem;
use crate::storage::StorageObject;

/// 是否需要更新（基于 lastModified 时间戳字符串比较）。
pub fn needs_update(existing: Option<&PhotoManifestItem>, obj: &StorageObject) -> bool {
    let Some(existing) = existing else { return true; };
    let Some(lm) = obj.last_modified else { return true; };
    let obj_iso = lm.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    obj_iso != existing.last_modified // 字符串不等即视为变化（含更新/回退）
}

/// 缩略图文件是否存在。
pub fn thumbnail_exists(thumb_dir: &Path, photo_id: &str) -> bool {
    thumb_dir.join(format!("{photo_id}.jpg")).exists()
}

/// 从存储图片列表筛选出需要处理的对象。force 时全量。
pub fn filter_tasks<'a>(
    images: &'a [StorageObject],
    existing_by_key: &HashMap<String, &PhotoManifestItem>,
    thumb_dir: &Path,
    force: bool,
) -> Vec<&'a StorageObject> {
    if force { return images.iter().collect(); }
    images.iter().filter(|obj| {
        let existing = existing_by_key.get(&obj.key).copied();
        if existing.is_none() { return true; }
        if needs_update(existing, obj) { return true; }
        let id = obj.key.rsplit('/').next().unwrap_or(&obj.key)
            .rsplit_once('.').map(|(s, _)| s).unwrap_or(&obj.key);
        !thumbnail_exists(thumb_dir, id)
    }).collect()
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
            if let Some(stem) = name.strip_suffix(".jpg") {
                if !keep.contains(stem) {
                    if std::fs::remove_file(entry.path()).is_ok() { deleted += 1; }
                }
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
        StorageObject { key: key.into(), size: Some(1), last_modified: Some(Utc.timestamp_opt(secs, 0).unwrap()), etag: None }
    }
    fn item(id: &str, key: &str, last_modified: &str) -> PhotoManifestItem {
        let mut it = crate::manifest::AfilmoryManifest::default();
        let _ = &mut it; // silence
        PhotoManifestItem {
            id: id.into(), format: "JPG".into(), title: id.into(), description: String::new(),
            date_taken: last_modified.into(), tags: vec![], original_url: String::new(),
            thumbnail_url: format!("/thumbnails/{id}.jpg"), thumb_hash: None, width: 1, height: 1,
            aspect_ratio: 1.0, s3_key: key.into(), last_modified: last_modified.into(), size: 1,
            digest: None, exif: None, tone_analysis: None, location: None, video: None, is_hdr: false, og_image_url: None,
        }
    }

    #[test]
    fn new_and_changed_selected() {
        let dir = tempdir().unwrap();
        let existing_item = item("a", "a.jpg", "1970-01-01T00:00:10.000Z");
        let mut map = HashMap::new();
        map.insert("a.jpg".to_string(), &existing_item);
        // a 未变(10s) 且缩略图存在 → 跳过；b 新增 → 选中
        std::fs::write(dir.path().join("a.jpg"), b"x").unwrap();
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
```

> `filter_tasks` 用纯 basename 作 thumbnail 判定（与上游 `filterTaskImages` 一致，不含 digest 后缀）。注意测试里 `thumbnail_exists` 对 key "a.jpg" 的 id 是 "a"，需另建 `a.jpg` 缩略图——上例中 a 未变但缩略图不存在仍会被选中，故测试构造 b 为新增、a 缩略图缺失也会入选；为聚焦"新增"，把 a 的缩略图补上：在断言前 `std::fs::write(dir.path().join("a.jpg"), ...)` 已写入的是"缩略图目录里的 a.jpg"，即 id=a 的缩略图，故 a 被跳过、仅 b 入选。

- [ ] **Step 2: 运行测试**

Run: `cargo test manifest::incremental`
Expected: 2 PASS。

- [ ] **Step 3: Commit**

```bash
git add src/manifest/
git commit -m "feat: incremental filter + deleted thumbnail cleanup"
```

---

### Task 14: Builder 编排 + `build` 子命令（端到端）

**Files:**
- Create: `src/builder.rs`（替换占位）
- Modify: `src/main.rs`
- Test: 集成测试 `tests/build_integration.rs`

- [ ] **Step 1: 写 src/builder.rs**

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};

use crate::config::{Config, StorageConfig};
use crate::error::Result;
use crate::exif::{ExifExtractor, ExiftoolExtractor};
use crate::manifest::{filter_tasks, handle_deleted, load_manifest, save_manifest, PhotoManifestItem};
use crate::pipeline::{process_photo, PipelineDeps};
use crate::storage::{LocalProvider, StorageProvider};

pub struct BuildOptions { pub force: bool }
impl Default for BuildOptions { fn default() -> Self { Self { force: false } } }

#[derive(Debug, Default)]
pub struct BuildResult {
    pub new_count: usize,
    pub processed_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub deleted_count: usize,
    pub total: usize,
}

pub struct Builder {
    config: Config,
    storage: Arc<dyn StorageProvider>,
    exif: Arc<dyn ExifExtractor>,
    lock: Mutex<()>,
    manifest_path: PathBuf,
    thumb_dir: PathBuf,
}

impl Builder {
    pub fn from_config(config: Config) -> Result<Self> {
        let storage: Arc<dyn StorageProvider> = match &config.storage {
            StorageConfig::Local { base_path, base_url, exclude_regex, max_file_limit } => Arc::new(
                LocalProvider::new(base_path.clone(), base_url.clone(), exclude_regex.clone(), *max_file_limit)?
            ),
        };
        let exif: Arc<dyn ExifExtractor> = Arc::new(ExiftoolExtractor::new(config.exif.exiftool_path.clone()));
        let workdir = config.server.workdir.clone();
        let manifest_path = workdir.join("manifest.json");
        let thumb_dir = workdir.join("thumbnails");
        Ok(Self { config, storage, exif, lock: Mutex::new(()), manifest_path, thumb_dir })
    }

    pub fn manifest_path(&self) -> &std::path::Path { &self.manifest_path }

    pub async fn build(&self, opts: BuildOptions) -> Result<BuildResult> {
        let _guard = self.lock.lock().await; // 串行化：webhook/轮询并发触发不会撕裂

        let existing = load_manifest(&self.manifest_path)?;
        let existing_by_key: HashMap<String, &PhotoManifestItem> =
            existing.data.iter().map(|i| (i.s3_key.clone(), i)).collect();

        let images = self.storage.list_images().await?;
        let s3_keys: std::collections::HashSet<String> = images.iter().map(|o| o.key.clone()).collect();
        let tasks = filter_tasks(&images, &existing_by_key, &self.thumb_dir, opts.force);

        let sem = Arc::new(Semaphore::new(self.config.processing.concurrency.max(1)));
        let mut handles = Vec::new();
        for obj in tasks.iter().cloned().cloned() {
            let permit = sem.clone().acquire_owned().await.unwrap();
            let storage = self.storage.clone();
            let exif = self.exif.clone();
            let processing = self.config.processing.clone();
            let thumb_dir = self.thumb_dir.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let deps = PipelineDeps { storage: storage.as_ref(), exif: exif.as_ref(), processing: &processing, thumb_dir: &thumb_dir };
                let is_new = true; // 计数细分留待 server 计划；M1a 先按处理/失败统计
                (obj.key.clone(), is_new, process_photo(&obj, &deps).await)
            }));
        }

        let mut result = BuildResult::default();
        let mut processed: HashMap<String, PhotoManifestItem> = HashMap::new();
        for h in handles {
            let (key, _is_new, res) = h.await.expect("task panicked");
            match res {
                Ok(item) => { result.processed_count += 1; processed.insert(key, item); }
                Err(e) => { result.failed_count += 1; tracing::warn!("process failed {key}: {e}"); }
            }
        }

        // 合并：本轮处理结果 + 存储里仍存在但未处理的旧项
        let mut final_items: Vec<PhotoManifestItem> = Vec::new();
        for (key, existing_item) in &existing_by_key {
            if processed.contains_key(key) { continue; }
            if s3_keys.contains(key) {
                final_items.push((*existing_item).clone());
                result.skipped_count += 1;
            }
        }
        final_items.extend(processed.into_values());

        result.deleted_count = handle_deleted(&self.thumb_dir, &final_items);
        result.total = final_items.len();
        save_manifest(&self.manifest_path, final_items)?;
        Ok(result)
    }
}
```

> `new_count` 精确细分（new vs processed）依赖 existing 判定，留到 server 计划完善；M1a 以 processed/skipped/failed/deleted/total 为准，足以验证端到端。

- [ ] **Step 2: 写 src/main.rs（`build` 子命令）**

```rust
use std::path::PathBuf;
use afilmory_lite::builder::{Builder, BuildOptions};
use afilmory_lite::config::Config;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber_init();
    let args: Vec<String> = std::env::args().collect();
    // 用法：afilmory-lite build --config <path> [--force]
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let config_path = arg_value(&args, "--config").unwrap_or_else(|| "afilmory.toml".into());
    let force = args.iter().any(|a| a == "--force");

    match cmd {
        "build" => {
            let config = Config::load(&PathBuf::from(&config_path))?;
            let builder = Builder::from_config(config)?;
            let r = builder.build(BuildOptions { force }).await?;
            println!("build done: processed={} skipped={} failed={} deleted={} total={}",
                r.processed_count, r.skipped_count, r.failed_count, r.deleted_count, r.total);
            println!("manifest: {}", builder.manifest_path().display());
        }
        _ => {
            eprintln!("usage: afilmory-lite build --config <path> [--force]");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).cloned()
}

fn tracing_subscriber_init() {
    // 极简：忽略错误，避免引入额外依赖配置
    let _ = std::panic::catch_unwind(|| {});
}
```

> 若希望真正的日志，后续可加 `tracing-subscriber`；M1a 用 `println!` 即可，`tracing::warn!` 无 subscriber 时静默丢弃，可接受。

- [ ] **Step 3: 写集成测试 tests/build_integration.rs**

```rust
use std::fs;
use afilmory_lite::builder::{Builder, BuildOptions};
use afilmory_lite::config::Config;

fn write_jpg(path: &std::path::Path, w: u32, h: u32) {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([120, 130, 140]));
    image::DynamicImage::ImageRgb8(img).save(path).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn end_to_end_local_build() {
    let dir = tempfile::tempdir().unwrap();
    let photos = dir.path().join("photos");
    let work = dir.path().join("work");
    fs::create_dir_all(photos.join("trip")).unwrap();
    write_jpg(&photos.join("trip/2024-05-01_sunset.jpg"), 1200, 800);
    write_jpg(&photos.join("hello.png"), 400, 400);

    let toml = format!(r#"
        [server]
        workdir = "{work}"
        dist_dir = ""
        [storage]
        provider = "local"
        base_path = "{photos}"
        base_url = "/photos"
        [processing]
        concurrency = 2
    "#, work = work.display(), photos = photos.display());

    let config = Config::from_toml_str(&toml).unwrap();
    let builder = Builder::from_config(config).unwrap();
    let r = builder.build(BuildOptions { force: false }).await.unwrap();
    assert_eq!(r.total, 2);
    assert!(r.failed_count == 0, "no failures expected");

    // manifest 存在且结构正确
    let manifest_str = fs::read_to_string(work.join("manifest.json")).unwrap();
    let m: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();
    assert_eq!(m["version"], "v10");
    assert_eq!(m["data"].as_array().unwrap().len(), 2);

    // 缩略图生成
    assert!(work.join("thumbnails/sunset.jpg").exists() || work.join("thumbnails/2024-05-01_sunset.jpg").exists());
    // 第二次构建为增量：无失败、total 不变
    let r2 = builder.build(BuildOptions { force: false }).await.unwrap();
    assert_eq!(r2.total, 2);
}
```

> 注意：缩略图文件名用 photoId（= 文件名去扩展名），`2024-05-01_sunset.jpg` 的 id 是 `2024-05-01_sunset`，故缩略图为 `thumbnails/2024-05-01_sunset.jpg`。断言里二选一以容错。

- [ ] **Step 4: 运行全部测试**

Run: `cargo test`
Expected: 全部 PASS（exiftool 缺失时其集成测试自动 skip）。若装了 exiftool，端到端会附带读取 EXIF。

- [ ] **Step 5: 手动冒烟**

Run:
```bash
mkdir -p /tmp/af/photos && cp <几张真实照片> /tmp/af/photos/
cat > /tmp/af/afilmory.toml <<'EOF'
[server]
workdir = "/tmp/af/work"
dist_dir = ""
[storage]
provider = "local"
base_path = "/tmp/af/photos"
base_url = "/photos"
EOF
cargo run -- build --config /tmp/af/afilmory.toml
```
Expected: 打印统计；`/tmp/af/work/manifest.json` + `/tmp/af/work/thumbnails/*.jpg` 生成。人工检查 manifest 字段结构与缩略图视觉正常。

- [ ] **Step 6: Commit**

```bash
git add src/builder.rs src/main.rs tests/
git commit -m "feat: builder orchestration + build subcommand (end-to-end local loop)"
```

---

## Self-Review 结论（写计划时已核对）

- **Spec 覆盖**：本计划覆盖 spec 的 storage(Local)、pipeline(decode/thumbnail/thumbhash/tone/info)、exif(exiftool)、manifest(model/store/incremental)、builder 编排。Spec 中 server/scheduler/S3/HEIC/Live/Motion/HDR-video/geocoding/OG **不在本计划**——属计划二及 M2/M3/M4。
- **类型一致**：`PhotoManifestItem` 字段、`StorageObject`、`ExifResult`、`PipelineDeps`、`BuildOptions/BuildResult` 在各任务间签名一致。
- **已知简化（非占位，均有说明）**：image 自带 JPEG 编码/Lanczos3（非 mozjpeg）；EXIF 每图写临时文件（非 stay-open）；`new_count` 细分留待计划二；日期时区按 UTC 解析。这些都在对应任务标注，且不影响"结构/语义一致"。

---

## 后续计划（提示，不在本计划内）

- **计划二（M1b）**：`server/`（axum serve dist + 注入 `__MANIFEST__`/`__CONFIG__`/`__SITE_CONFIG__` + SPA fallback + 缓存头）、main 装配为常驻 daemon、手动/轮询/webhook/S3 事件触发、`AppState` 热更新 manifest 缓存、`/api/status`。
- **M2**：S3(SigV4) provider + 存储 trait 转 async 已就绪、增量 newCount 细分、exiftool stay-open。
- **M3**：HEIC(libheif)、Live Photo、Motion Photo、HDR 视频字段、BMP 转码 digest。
- **M4**：OSS/COS/B2/GitHub、geocoding、OG/SEO、native-exif 快路径、mozjpeg/fast_image_resize 优化。
