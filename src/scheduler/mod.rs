use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

use crate::builder::BuildOptions;
use crate::state::AppState;

/// 串行化 + coalescing 的构建协调器。
/// 所有触发源（轮询/webhook/S3 事件/手动）都调用 `trigger`；构建按顺序执行，
/// 执行期间到来的多次触发合并为「之后再跑一次」。
#[derive(Clone)]
pub struct BuildCoordinator {
    inner: Arc<Inner>,
}

struct Inner {
    notify: Notify,
    pending: AtomicBool,
    force: AtomicBool,
}

impl BuildCoordinator {
    /// 启动后台 worker。
    pub fn start(state: AppState) -> Self {
        let coord = Self {
            inner: Arc::new(Inner {
                notify: Notify::new(),
                pending: AtomicBool::new(false),
                force: AtomicBool::new(false),
            }),
        };
        let worker = coord.clone();
        tokio::spawn(async move {
            loop {
                worker.inner.notify.notified().await;
                while worker.inner.pending.swap(false, Ordering::SeqCst) {
                    let force = worker.inner.force.swap(false, Ordering::SeqCst);
                    if let Err(e) = state.run_build(BuildOptions { force }).await {
                        tracing::error!("scheduled build failed: {e}");
                    }
                }
            }
        });
        coord
    }

    /// 投递一次构建请求（非阻塞）。`force` 为粘性：本轮合并窗口内任一触发要求 force 即生效。
    pub fn trigger(&self, force: bool) {
        if force {
            self.inner.force.store(true, Ordering::SeqCst);
        }
        self.inner.pending.store(true, Ordering::SeqCst);
        self.inner.notify.notify_one();
    }
}

/// 启动定时轮询（interval_secs > 0 时）。
pub fn spawn_poll(coord: BuildCoordinator, interval_secs: u64) {
    if interval_secs == 0 {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(interval_secs));
        tick.tick().await; // interval 的首个 tick 立即返回，跳过
        loop {
            tick.tick().await;
            coord.trigger(false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tokio::time::{Duration, sleep, timeout};

    fn write_jpg(path: &std::path::Path) {
        let img = image::RgbImage::from_pixel(60, 40, image::Rgb([1, 2, 3]));
        image::DynamicImage::ImageRgb8(img).save(path).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn coordinator_runs_build() {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("photos");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&photos).unwrap();
        write_jpg(&photos.join("a.jpg"));
        let toml = format!(
            r#"
            [server]
            workdir = "{w}"
            [storage]
            provider = "local"
            base_path = "{p}"
        "#,
            w = work.display(),
            p = photos.display()
        );
        let state = AppState::new(Config::from_toml_str(&toml).unwrap()).unwrap();
        let coord = BuildCoordinator::start(state.clone());
        coord.trigger(false);

        let done = timeout(Duration::from_secs(10), async {
            loop {
                if state.status.read().await.last_result.is_some() {
                    break;
                }
                sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(done.is_ok(), "build did not complete in time");
        assert_eq!(state.manifest.read().await.data.len(), 1);
    }
}
