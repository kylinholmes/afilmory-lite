mod inject;
pub use inject::inject;

use std::path::Path;

use axum::extract::{Path as AxPath, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};

use crate::config::{Config, StorageConfig};
use crate::scheduler::BuildCoordinator;
use crate::state::AppState;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

/// 构建 serve 路由：dist 静态资源、缩略图、触发/管理 API、/admin 页、以及 SPA fallback（注入首页 + 本地原图）。
pub fn build_router(state: AppState, coord: BuildCoordinator) -> Router {
    Router::new()
        .route("/static/web/{*path}", get(serve_static))
        .route("/thumbnails/{*path}", get(serve_thumbnail))
        .route("/api/status", get(api_status))
        .route("/api/admin/build", post(api_admin_build))
        .route("/api/admin/config", get(api_get_config).put(api_put_config))
        .route("/api/admin/test-storage", post(api_test_storage))
        .route("/api/hooks/build", post(api_hook_build))
        .route("/api/hooks/s3", post(api_hook_s3))
        .route("/admin", get(admin_page))
        .fallback(spa_fallback)
        .layer(Extension(coord))
        .with_state(state)
}

// ---- 静态资源 / 首页注入 / 本地原图 ----

async fn serve_static(State(st): State<AppState>, AxPath(path): AxPath<String>) -> Response {
    let cfg = st.config().await;
    serve_file(&cfg.server.dist_dir, &path).await
}

async fn serve_thumbnail(State(st): State<AppState>, AxPath(path): AxPath<String>) -> Response {
    let cfg = st.config().await;
    serve_file(&cfg.server.workdir.join("thumbnails"), &path).await
}

/// fallback：① 命中本地原图前缀 → serve 原图；② 无扩展名 → 注入后的 index.html；③ 有扩展名未命中 → 404。
async fn spa_fallback(State(st): State<AppState>, uri: Uri) -> Response {
    let path = uri.path();
    // 本地存储托管原图（前缀来自 base_url，热重载可变，故在此动态判断而非静态路由）
    if let Some((prefix, dir)) = st.originals().await
        && let Some(rel) = path.strip_prefix(&prefix).and_then(|r| r.strip_prefix('/'))
        && !rel.is_empty()
    {
        return serve_file(&dir, rel).await;
    }
    let last = path.rsplit('/').next().unwrap_or("");
    if last.contains('.') && !last.starts_with('.') {
        return StatusCode::NOT_FOUND.into_response();
    }
    render_index(&st).await
}

async fn serve_file(base: &Path, rel: &str) -> Response {
    let Some(full) = safe_join(base, rel) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&full).await {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&full).first_or_octet_stream();
            (
                [
                    (header::CONTENT_TYPE, mime.as_ref().to_owned()),
                    (header::CACHE_CONTROL, IMMUTABLE.to_owned()),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn render_index(st: &AppState) -> Response {
    let cfg = st.config().await;
    let index_path = cfg.server.dist_dir.join("index.html");
    let Ok(html) = tokio::fs::read_to_string(&index_path).await else {
        return (StatusCode::NOT_FOUND, "index.html not found").into_response();
    };
    let manifest_json = {
        let m = st.manifest.read().await;
        serde_json::to_string(&*m).unwrap_or_else(|_| "{}".into())
    };
    let site_json = serde_json::to_string(&cfg.site).unwrap_or_else(|_| "{}".into());
    let (title, description) = site_title_desc(&cfg.site);
    let injected = inject(
        &html,
        &manifest_json,
        "{}",
        &site_json,
        title.as_deref(),
        description.as_deref(),
    );
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_owned()),
            (header::CACHE_CONTROL, NO_CACHE.to_owned()),
        ],
        injected,
    )
        .into_response()
}

fn site_title_desc(site: &serde_json::Value) -> (Option<String>, Option<String>) {
    let t = site.get("title").and_then(|v| v.as_str()).map(String::from);
    let d = site
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);
    (t, d)
}

/// 拼接并防目录穿越：拒绝包含 `..` / `.` 段。
fn safe_join(base: &Path, rel: &str) -> Option<std::path::PathBuf> {
    if rel.split('/').any(|seg| seg == ".." || seg == ".") {
        return None;
    }
    Some(base.join(rel))
}

// ---- 管理后台页 ----

async fn admin_page() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, NO_CACHE),
        ],
        include_str!("admin_page.html"),
    )
        .into_response()
}

// ---- 触发 / 配置 API ----

#[derive(serde::Deserialize, Default)]
struct AdminBuildReq {
    #[serde(default)]
    force: bool,
}

async fn api_status(State(st): State<AppState>) -> Response {
    let status = st.status.read().await.clone();
    Json(status).into_response()
}

async fn api_admin_build(
    State(st): State<AppState>,
    Extension(coord): Extension<BuildCoordinator>,
    headers: HeaderMap,
    body: Option<Json<AdminBuildReq>>,
) -> Response {
    let cfg = st.config().await;
    match check_bearer(cfg.triggers.webhook_token.as_deref(), &headers) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(false) => StatusCode::UNAUTHORIZED.into_response(),
        Some(true) => {
            let force = body.map(|j| j.0.force).unwrap_or(false);
            coord.trigger(force);
            StatusCode::ACCEPTED.into_response()
        }
    }
}

async fn api_hook_build(
    State(st): State<AppState>,
    Extension(coord): Extension<BuildCoordinator>,
    headers: HeaderMap,
) -> Response {
    let cfg = st.config().await;
    match check_bearer(cfg.triggers.webhook_token.as_deref(), &headers) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(false) => StatusCode::UNAUTHORIZED.into_response(),
        Some(true) => {
            coord.trigger(false);
            StatusCode::ACCEPTED.into_response()
        }
    }
}

async fn api_hook_s3(
    State(st): State<AppState>,
    Extension(coord): Extension<BuildCoordinator>,
    _body: Option<Json<serde_json::Value>>,
) -> Response {
    if !st.config().await.triggers.enable_s3_event {
        return StatusCode::NOT_FOUND.into_response();
    }
    coord.trigger(false);
    StatusCode::ACCEPTED.into_response()
}

/// 读当前配置（含密钥，故 admin token 鉴权）。
async fn api_get_config(State(st): State<AppState>, headers: HeaderMap) -> Response {
    let cfg = st.config().await;
    match check_bearer(cfg.server.admin_token.as_deref(), &headers) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(false) => StatusCode::UNAUTHORIZED.into_response(),
        Some(true) => Json(&*cfg).into_response(),
    }
}

/// 写配置：校验 → 写回 TOML 文件 → 运行时热重载。
/// `[server]` 段不允许经管理端修改——总是用当前运行配置的 server 覆盖请求体。
async fn api_put_config(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let cur = st.config().await;
    match check_bearer(cur.server.admin_token.as_deref(), &headers) {
        None => return StatusCode::NOT_FOUND.into_response(),
        Some(false) => return StatusCode::UNAUTHORIZED.into_response(),
        Some(true) => {}
    }
    let Some(Json(mut v)) = body else {
        return (StatusCode::BAD_REQUEST, "missing or invalid config body").into_response();
    };
    // 强制保留当前 server（listen/workdir/dist_dir/admin_token 只能改文件并重启）
    let server_value = match serde_json::to_value(&cur.server) {
        Ok(sv) => sv,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("server: {e}")).into_response();
        }
    };
    match v.as_object_mut() {
        Some(obj) => {
            obj.insert("server".to_string(), server_value);
        }
        None => return (StatusCode::BAD_REQUEST, "config must be a JSON object").into_response(),
    }
    let new_config: Config = match serde_json::from_value(v) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid config: {e}")).into_response(),
    };
    let toml = match new_config.to_toml_string() {
        Ok(t) => t,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("serialize: {e}")).into_response(),
    };
    // 先热重载（会重建 storage/builder，能验出存储配置错误）
    if let Err(e) = st.reload(new_config).await {
        return (StatusCode::BAD_REQUEST, format!("reload failed: {e}")).into_response();
    }
    // 再持久化到文件
    if let Err(e) = std::fs::write(st.config_path(), &toml) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("applied but write failed: {e}"),
        )
            .into_response();
    }
    (StatusCode::OK, "reloaded").into_response()
}

/// 测试存储连接：用请求体里的（未保存的）storage 配置建 provider 并试列举一次，
/// 返回 `{ok:true,count}` 或 `{ok:false,error}`（连接失败也回 200，便于前端读取详情）。
async fn api_test_storage(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let cfg = st.config().await;
    match check_bearer(cfg.server.admin_token.as_deref(), &headers) {
        None => return StatusCode::NOT_FOUND.into_response(),
        Some(false) => return StatusCode::UNAUTHORIZED.into_response(),
        Some(true) => {}
    }
    let Some(Json(v)) = body else {
        return (StatusCode::BAD_REQUEST, "missing storage body").into_response();
    };
    let storage: StorageConfig = match serde_json::from_value(v) {
        Ok(s) => s,
        Err(e) => {
            return Json(serde_json::json!({"ok": false, "error": format!("配置无效：{e}")}))
                .into_response();
        }
    };
    let provider = match crate::storage::build_provider(&storage) {
        Ok(p) => p,
        Err(e) => {
            return Json(serde_json::json!({"ok": false, "error": format!("初始化失败：{e}")}))
                .into_response();
        }
    };
    match provider.list_images().await {
        Ok(objs) => {
            tracing::info!(count = objs.len(), "存储测试连接成功");
            Json(serde_json::json!({"ok": true, "count": objs.len()})).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "存储测试连接失败");
            Json(serde_json::json!({"ok": false, "error": format!("{e}")})).into_response()
        }
    }
}

/// Bearer 鉴权：None=端点禁用（未配置 token）；Some(true)=通过；Some(false)=拒绝。
fn check_bearer(expected: Option<&str>, headers: &HeaderMap) -> Option<bool> {
    let token = expected?;
    let got = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    Some(got == Some(format!("Bearer {token}").as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn body_string(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn setup(token: Option<&str>) -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let dist = dir.path().join("dist");
        let work = dir.path().join("work");
        let photos = dir.path().join("photos");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::create_dir_all(work.join("thumbnails")).unwrap();
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(
            dist.join("index.html"),
            r#"<html><head><title>x</title><script id="config">window.__CONFIG__ = {}</script><script id="manifest"></script></head><body></body></html>"#,
        )
        .unwrap();
        std::fs::write(dist.join("assets/app.js"), b"console.log(1)").unwrap();
        std::fs::write(work.join("thumbnails/x.jpg"), b"jpgdata").unwrap();
        std::fs::write(photos.join("orig.jpg"), b"jpegdata").unwrap();
        let triggers = match token {
            Some(t) => format!("[triggers]\nwebhook_token = \"{t}\"\nenable_s3_event = true\n"),
            None => String::new(),
        };
        let toml = format!(
            r#"
            [server]
            listen = "127.0.0.1:0"
            workdir = "{work}"
            dist_dir = "{dist}"
            [site]
            title = "My Gallery"
            description = "Desc"
            [storage.local]
            base_path = "{photos}"
            base_url = "/photos"
            {triggers}
        "#,
            work = work.display(),
            dist = dist.display(),
            photos = photos.display(),
            triggers = triggers
        );
        let config = Config::from_toml_str(&toml).unwrap();
        let state = AppState::new(config, dir.path().join("afilmory.toml")).unwrap();
        (dir, state)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn serves_static_thumbnail_spa_and_404() {
        let (_dir, state) = setup(None);
        let coord = BuildCoordinator::start(state.clone());
        let app = build_router(state, coord);

        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/static/web/assets/app.js").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("cache-control").unwrap(), IMMUTABLE);

        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/thumbnails/x.jpg").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/some/photo/id").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("cache-control").unwrap(), NO_CACHE);
        let body = body_string(resp).await;
        assert!(body.contains("window.__MANIFEST__ = "));
        assert!(body.contains("<title>My Gallery</title>"));

        let resp = app
            .oneshot(Request::builder().uri("/missing.css").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn serves_local_originals_via_fallback() {
        let (_dir, state) = setup(None);
        let coord = BuildCoordinator::start(state.clone());
        let app = build_router(state, coord);
        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/photos/orig.jpg").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = app
            .oneshot(Request::builder().uri("/photos/missing.jpg").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn webhook_auth_and_status() {
        let (_dir, state) = setup(Some("t"));
        let coord = BuildCoordinator::start(state.clone());
        let app = build_router(state, coord);

        let resp = app
            .clone()
            .oneshot(Request::builder().method("POST").uri("/api/hooks/build").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/hooks/build")
                    .header("authorization", "Bearer t")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let resp = app
            .oneshot(Request::builder().uri("/api/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_string(resp).await.contains("\"running\""));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn admin_config_disabled_without_token() {
        let (_dir, state) = setup(None); // 无 admin_token
        let coord = BuildCoordinator::start(state.clone());
        let app = build_router(state, coord);
        let resp = app
            .oneshot(Request::builder().uri("/api/admin/config").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn admin_config_get_put_reload() {
        // 带 admin_token 的独立配置
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        let photos = dir.path().join("photos");
        let dist = dir.path().join("dist");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::create_dir_all(&dist).unwrap();
        let cfg_path = dir.path().join("afilmory.toml");
        let toml = format!(
            "[server]\nworkdir = \"{w}\"\ndist_dir = \"{d}\"\nadmin_token = \"adm\"\n[storage.local]\nbase_path = \"{p}\"\n[processing]\nconcurrency = 3\n",
            w = work.display(),
            d = dist.display(),
            p = photos.display()
        );
        std::fs::write(&cfg_path, &toml).unwrap();
        let state = AppState::new(Config::from_toml_str(&toml).unwrap(), cfg_path.clone()).unwrap();
        let coord = BuildCoordinator::start(state.clone());
        let app = build_router(state.clone(), coord);

        // GET 需鉴权
        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/api/admin/config").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/api/admin/config").header("authorization", "Bearer adm").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cfg_json = body_string(resp).await;
        assert!(cfg_json.contains("\"concurrency\":3"));

        // PUT 修改 concurrency → reload + 写回
        let mut v: serde_json::Value = serde_json::from_str(&cfg_json).unwrap();
        v["processing"]["concurrency"] = serde_json::json!(7);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/admin/config")
                    .header("authorization", "Bearer adm")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&v).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // 运行时已生效
        assert_eq!(state.config().await.processing.concurrency, 7);
        // 文件已写回
        let written = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(written.contains("concurrency = 7"));
    }
}
