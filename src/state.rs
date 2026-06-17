use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;

use crate::builder::{BuildOptions, BuildResult, Builder};
use crate::config::Config;
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

/// 服务共享状态。`Clone` 廉价（全是 Arc）。
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub builder: Arc<Builder>,
    pub manifest: Arc<RwLock<AfilmoryManifest>>,
    pub status: Arc<RwLock<BuildStatus>>,
}

impl AppState {
    pub fn new(config: Config) -> Result<Self> {
        let builder = Builder::from_config(config.clone())?;
        let manifest = load_manifest(builder.manifest_path())?;
        Ok(Self {
            config: Arc::new(config),
            builder: Arc::new(builder),
            manifest: Arc::new(RwLock::new(manifest)),
            status: Arc::new(RwLock::new(BuildStatus::default())),
        })
    }

    /// 执行一次构建（Builder 内部已串行化），完成后热更新 manifest 缓存与状态。
    pub async fn run_build(&self, opts: BuildOptions) -> Result<BuildResult> {
        self.status.write().await.running = true;
        let result = self.builder.build(opts).await;

        // 无论成败都尝试重载缓存（成功时反映新数据）
        if let Ok(m) = load_manifest(self.builder.manifest_path()) {
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
            [storage]
            provider = "local"
            base_path = "{photos}"
        "#,
            work = work.display(),
            photos = photos.display()
        );
        let config = Config::from_toml_str(&toml).unwrap();
        let state = AppState::new(config).unwrap();
        assert_eq!(state.manifest.read().await.data.len(), 0);

        let r = state.run_build(BuildOptions::default()).await.unwrap();
        assert_eq!(r.total, 1);
        assert_eq!(state.manifest.read().await.data.len(), 1);
        let st = state.status.read().await;
        assert!(!st.running);
        assert!(st.last_result.is_some());
    }
}
