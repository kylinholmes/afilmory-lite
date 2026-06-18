use std::fs;

use afilmory_lite::builder::BuildOptions;
use afilmory_lite::config::Config;
use afilmory_lite::scheduler::BuildCoordinator;
use afilmory_lite::server::build_router;
use afilmory_lite::state::AppState;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn write_jpg(path: &std::path::Path, w: u32, h: u32) {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([90, 100, 110]));
    image::DynamicImage::ImageRgb8(img).save(path).unwrap();
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_injects_built_manifest_and_thumbnail() {
    let dir = tempfile::tempdir().unwrap();
    let dist = dir.path().join("dist");
    let work = dir.path().join("work");
    let photos = dir.path().join("photos");
    fs::create_dir_all(&dist).unwrap();
    fs::create_dir_all(&photos).unwrap();
    fs::write(
        dist.join("index.html"),
        r#"<html><head><title>x</title><script id="config">window.__CONFIG__ = {}</script><script id="manifest"></script></head><body></body></html>"#,
    )
    .unwrap();
    write_jpg(&photos.join("sunset.jpg"), 800, 600);

    let toml = format!(
        r#"
        [server]
        workdir = "{work}"
        dist_dir = "{dist}"
        [site]
        title = "Gallery"
        [storage.local]
        base_path = "{photos}"
    "#,
        work = work.display(),
        dist = dist.display(),
        photos = photos.display()
    );
    let state =
        AppState::new(Config::from_toml_str(&toml).unwrap(), dir.path().join("afilmory.toml")).unwrap();

    // 构建（直接调用，确定性）
    let r = state
        .run_build(BuildOptions { force: false })
        .await
        .unwrap();
    assert_eq!(r.total, 1);

    let coord = BuildCoordinator::start(state.clone());
    let app = build_router(state, coord);

    // GET / → 注入的 manifest 含该照片
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp).await;
    assert!(body.contains("window.__MANIFEST__"));
    assert!(body.contains("\"sunset\"")); // photo id 出现在 manifest 中
    assert!(body.contains("<title>Gallery</title>"));

    // 缩略图可达
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/thumbnails/sunset.jpg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
