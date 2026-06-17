use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};

use crate::config::{Config, StorageConfig};
use crate::error::Result;
use crate::exif::{ExifExtractor, ExiftoolExtractor};
use crate::manifest::{
    PhotoManifestItem, filter_tasks, handle_deleted, load_manifest, save_manifest,
};
use crate::pipeline::{PipelineDeps, process_photo};
use crate::storage::{LocalProvider, S3Provider, StorageProvider, detect_live_photos};

#[derive(Default)]
pub struct BuildOptions {
    pub force: bool,
}

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
        let exif: Arc<dyn ExifExtractor> =
            Arc::new(ExiftoolExtractor::new(config.exif.exiftool_path.clone()));
        let workdir = config.server.workdir.clone();
        let manifest_path = workdir.join("manifest.json");
        let thumb_dir = workdir.join("thumbnails");
        Ok(Self {
            config,
            storage,
            exif,
            lock: Mutex::new(()),
            manifest_path,
            thumb_dir,
        })
    }

    pub fn manifest_path(&self) -> &std::path::Path {
        &self.manifest_path
    }

    pub async fn build(&self, opts: BuildOptions) -> Result<BuildResult> {
        let _guard = self.lock.lock().await; // 串行化：webhook/轮询并发触发不会撕裂

        let existing = load_manifest(&self.manifest_path)?;
        let existing_by_key: HashMap<String, &PhotoManifestItem> = existing
            .data
            .iter()
            .map(|i| (i.s3_key.clone(), i))
            .collect();

        let images = self.storage.list_images().await?;
        let s3_keys: HashSet<String> = images.iter().map(|o| o.key.clone()).collect();
        let tasks = filter_tasks(&images, &existing_by_key, &self.thumb_dir, opts.force);

        // Live Photo 配对（需要全量文件列表；按配置开关）
        let live_map = Arc::new(if self.config.processing.enable_live_photo {
            detect_live_photos(&self.storage.list_all_files().await?)
        } else {
            HashMap::new()
        });

        let sem = Arc::new(Semaphore::new(self.config.processing.concurrency.max(1)));
        let mut handles = Vec::new();
        for obj in tasks.iter().cloned().cloned() {
            let permit = sem.clone().acquire_owned().await.unwrap();
            let storage = self.storage.clone();
            let exif = self.exif.clone();
            let processing = self.config.processing.clone();
            let thumb_dir = self.thumb_dir.clone();
            let live_map = live_map.clone();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let deps = PipelineDeps {
                    storage: storage.as_ref(),
                    exif: exif.as_ref(),
                    processing: &processing,
                    thumb_dir: &thumb_dir,
                    live_map: live_map.as_ref(),
                };
                (obj.key.clone(), process_photo(&obj, &deps).await)
            }));
        }

        let mut result = BuildResult::default();
        let mut processed: HashMap<String, PhotoManifestItem> = HashMap::new();
        for h in handles {
            let (key, res) = h.await.expect("pipeline task panicked");
            match res {
                Ok(item) => {
                    result.processed_count += 1;
                    if !existing_by_key.contains_key(&key) {
                        result.new_count += 1;
                    }
                    processed.insert(key, item);
                }
                Err(e) => {
                    result.failed_count += 1;
                    tracing::warn!("process failed {key}: {e}");
                }
            }
        }

        // 合并：存储里仍存在但本轮未处理的旧项
        let mut final_items: Vec<PhotoManifestItem> = Vec::new();
        for (key, existing_item) in &existing_by_key {
            if processed.contains_key(key) {
                continue;
            }
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
