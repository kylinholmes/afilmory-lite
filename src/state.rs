use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;

use crate::builder::{BuildOptions, BuildResult, Builder};
use crate::config::{Config, StorageConfig};
use crate::error::Result;
use crate::manifest::{AfilmoryManifest, load_manifest};

/// 最近一次构建的状态（供 `/api/status`）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct BuildStatus {
    pub running: bool,
    pub last_result: Option<BuildSummary>,
    pub last_finished_iso: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildSummary {
    pub new_count: usize,
    pub processed_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub deleted_count: usize,
    pub total: usize,
}

impl From<&BuildResult> for BuildSummary {
    fn from(r: &BuildResult) -> Self {
        Self {
            new_count: r.new_count,
            processed_count: r.processed_count,
            skipped_count: r.skipped_count,
            failed_count: r.failed_count,
            deleted_count: r.deleted_count,
            total: r.total,
        }
    }
}

/// 随配置变化、可热重载的部分（config + 由它派生的 builder/originals）。
struct Runtime {
    config: Arc<Config>,
    builder: Arc<Builder>,
    originals: Option<(String, PathBuf)>,
}

impl Runtime {
    fn build(config: Config) -> Result<Self> {
        let builder = Builder::from_config(config.clone())?;
        let originals = compute_originals(&config);
        Ok(Self {
            config: Arc::new(config),
            builder: Arc::new(builder),
            originals,
        })
    }
}

/// 本地存储且 base_url 为根路径（如 "/photos"）时，由本服务托管原图。
fn compute_originals(config: &Config) -> Option<(String, PathBuf)> {
    match &config.storage {
        StorageConfig::Local {
            base_path,
            base_url: Some(u),
            ..
        } if u.starts_with('/') => Some((u.trim_end_matches('/').to_string(), base_path.clone())),
        _ => None,
    }
}

/// 服务共享状态。`Clone` 廉价（全是 Arc）。可热重载部分在 `runtime` 里。
#[derive(Clone)]
pub struct AppState {
    runtime: Arc<RwLock<Runtime>>,
    pub manifest: Arc<RwLock<AfilmoryManifest>>,
    pub status: Arc<RwLock<BuildStatus>>,
    config_path: Arc<PathBuf>,
}

impl AppState {
    pub fn new(config: Config, config_path: PathBuf) -> Result<Self> {
        let runtime = Runtime::build(config)?;
        let manifest = load_manifest(runtime.builder.manifest_path())?;
        Ok(Self {
            runtime: Arc::new(RwLock::new(runtime)),
            manifest: Arc::new(RwLock::new(manifest)),
            status: Arc::new(RwLock::new(BuildStatus::default())),
            config_path: Arc::new(config_path),
        })
    }

    /// 当前配置（克隆 Arc，廉价）。
    pub async fn config(&self) -> Arc<Config> {
        self.runtime.read().await.config.clone()
    }

    /// 本地原图托管：(URL 前缀, 原图根目录)；远程存储为 None。
    pub async fn originals(&self) -> Option<(String, PathBuf)> {
        self.runtime.read().await.originals.clone()
    }

    /// 配置文件路径（写回用）。
    pub fn config_path(&self) -> &std::path::Path {
        &self.config_path
    }

    /// 当前 manifest.json 路径。
    pub async fn manifest_path(&self) -> PathBuf {
        self.runtime.read().await.builder.manifest_path().to_path_buf()
    }

    /// 执行一次构建（Builder 内部已串行化），完成后热更新 manifest 缓存与状态。
    pub async fn run_build(&self, opts: BuildOptions) -> Result<BuildResult> {
        self.status.write().await.running = true;
        let builder = self.runtime.read().await.builder.clone();
        let result = builder.build(opts).await;

        // 无论成败都尝试重载缓存（成功时反映新数据）
        if let Ok(m) = load_manifest(builder.manifest_path()) {
            *self.manifest.write().await = m;
        }

        let mut st = self.status.write().await;
        st.running = false;
        if let Ok(r) = &result {
            st.last_result = Some(BuildSummary::from(r));
            st.last_finished_iso =
                Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        }
        result
    }

    /// 热重载：用新配置重建 storage/builder/originals 并整体换掉。
    /// `listen`（监听端口）不在此处理——socket 已绑定，改它需重启。
    pub async fn reload(&self, new_config: Config) -> Result<()> {
        let runtime = Runtime::build(new_config)?;
        if let Ok(m) = load_manifest(runtime.builder.manifest_path()) {
            *self.manifest.write().await = m;
        }
        *self.runtime.write().await = runtime;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::BuildOptions;

    fn write_jpg(path: &std::path::Path, w: u32, h: u32) {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([10, 20, 30]));
        image::DynamicImage::ImageRgb8(img).save(path).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_build_updates_manifest_cache() {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&photos).unwrap();
        write_jpg(&photos.join("a.jpg"), 100, 80);

        let toml = format!(
            r#"
            [server]
            workdir = "{work}"
            [storage.local]
            base_path = "{photos}"
        "#,
            work = work.display(),
            photos = photos.display()
        );
        let config = Config::from_toml_str(&toml).unwrap();
        let state = AppState::new(config, dir.path().join("afilmory.toml")).unwrap();
        assert_eq!(state.manifest.read().await.data.len(), 0);

        let r = state.run_build(BuildOptions::default()).await.unwrap();
        assert_eq!(r.total, 1);
        assert_eq!(state.manifest.read().await.data.len(), 1);
        let st = state.status.read().await;
        assert!(!st.running);
        assert!(st.last_result.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reload_swaps_config_and_originals() {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        let mk = |base_url: &str| {
            let url_line = if base_url.is_empty() {
                String::new()
            } else {
                format!("base_url = \"{base_url}\"")
            };
            format!(
                "[server]\nworkdir = \"{w}\"\n[storage.local]\nbase_path = \"{p}\"\n{url_line}\n",
                w = dir.path().join("work").display(),
                p = photos.display(),
            )
        };
        // 初始无 base_url → 不托管原图
        let state =
            AppState::new(Config::from_toml_str(&mk("")).unwrap(), dir.path().join("c.toml")).unwrap();
        assert!(state.originals().await.is_none());

        // reload 成带根路径 base_url → 托管原图
        state.reload(Config::from_toml_str(&mk("/photos")).unwrap()).await.unwrap();
        assert_eq!(state.originals().await.unwrap().0, "/photos");
    }
}
