use std::fs;

use afilmory_lite::builder::{BuildOptions, Builder};
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

    let toml = format!(
        r#"
        [server]
        workdir = "{work}"
        dist_dir = ""
        [storage]
        provider = "local"
        base_path = "{photos}"
        base_url = "/photos"
        [processing]
        concurrency = 2
    "#,
        work = work.display(),
        photos = photos.display()
    );

    let config = Config::from_toml_str(&toml).unwrap();
    let builder = Builder::from_config(config).unwrap();
    let r = builder.build(BuildOptions { force: false }).await.unwrap();
    assert_eq!(r.total, 2);
    assert_eq!(r.failed_count, 0, "no failures expected");

    // manifest 存在且结构正确
    let manifest_str = fs::read_to_string(work.join("manifest.json")).unwrap();
    let m: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();
    assert_eq!(m["version"], "v10");
    assert_eq!(m["data"].as_array().unwrap().len(), 2);

    // 缩略图生成（id = 文件名去扩展名）
    assert!(work.join("thumbnails/2024-05-01_sunset.jpg").exists());
    assert!(work.join("thumbnails/hello.jpg").exists());

    // 第二次构建为增量：无失败、total 不变、本轮不再处理
    let r2 = builder.build(BuildOptions { force: false }).await.unwrap();
    assert_eq!(r2.total, 2);
    assert_eq!(
        r2.processed_count, 0,
        "second run should be fully incremental"
    );
    assert_eq!(r2.skipped_count, 2);
}
