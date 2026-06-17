use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::storage::{StorageObject, StorageProvider, is_image_key};

pub struct LocalProvider {
    base_path: PathBuf,
    base_url: Option<String>,
    exclude: Option<regex::Regex>,
    max_file_limit: Option<usize>,
}

impl LocalProvider {
    pub fn new(
        base_path: PathBuf,
        base_url: Option<String>,
        exclude_regex: Option<String>,
        max_file_limit: Option<usize>,
    ) -> Result<Self> {
        let exclude = match exclude_regex {
            Some(r) => Some(
                regex::Regex::new(&r)
                    .map_err(|e| Error::Config(format!("bad exclude_regex: {e}")))?,
            ),
            None => None,
        };
        Ok(Self {
            base_path,
            base_url,
            exclude,
            max_file_limit,
        })
    }

    fn scan(&self) -> Result<Vec<StorageObject>> {
        let mut out = Vec::new();
        for entry in WalkDir::new(&self.base_path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = match entry.path().strip_prefix(&self.base_path) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if let Some(re) = &self.exclude
                && re.is_match(&rel)
            {
                continue;
            }
            let meta = entry
                .metadata()
                .map_err(|e| Error::Storage(e.to_string()))?;
            let modified: Option<DateTime<Utc>> = meta.modified().ok().map(DateTime::<Utc>::from);
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            out.push(StorageObject {
                key: rel,
                size: Some(meta.len()),
                last_modified: modified,
                etag: Some(format!("{}-{}", mtime, meta.len())),
            });
        }
        if let Some(limit) = self.max_file_limit {
            out.truncate(limit);
        }
        Ok(out)
    }

    fn abs(&self, key: &str) -> PathBuf {
        self.base_path.join(key)
    }
}

#[async_trait]
impl StorageProvider for LocalProvider {
    async fn list_images(&self) -> Result<Vec<StorageObject>> {
        Ok(self
            .scan()?
            .into_iter()
            .filter(|o| is_image_key(&o.key))
            .collect())
    }
    async fn list_all_files(&self) -> Result<Vec<StorageObject>> {
        self.scan()
    }
    async fn get_file(&self, key: &str) -> Result<Option<Bytes>> {
        match tokio::fs::read(self.abs(key)).await {
            Ok(b) => Ok(Some(Bytes::from(b))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io {
                path: self.abs(key),
                source: e,
            }),
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

        let p = LocalProvider::new(dir.path().to_path_buf(), Some("/photos".into()), None, None)
            .unwrap();
        let mut imgs: Vec<String> = p
            .list_images()
            .await
            .unwrap()
            .into_iter()
            .map(|o| o.key)
            .collect();
        imgs.sort();
        assert_eq!(imgs, vec!["c.png".to_string(), "trip/a.JPG".to_string()]);
        assert_eq!(p.generate_public_url("trip/a.JPG"), "/photos/trip/a.JPG");
    }
}
