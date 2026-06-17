mod inject;
pub use inject::inject;

use std::path::{Path, PathBuf};

use axum::extract::{Path as AxPath, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};

use crate::scheduler::BuildCoordinator;
use crate::state::AppState;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

/// 构建 serve 路由：dist 静态资源、缩略图、触发 API、以及 SPA fallback（注入首页）。
pub fn build_router(state: AppState, coord: BuildCoordinator) -> Router {
    let mut router = Router::new()
        .route("/static/web/{*path}", get(serve_static))
        .route("/thumbnails/{*path}", get(serve_thumbnail))
        .route("/api/status", get(api_status))
        .route("/api/admin/build", post(api_admin_build))
        .route("/api/hooks/build", post(api_hook_build))
        .route("/api/hooks/s3", post(api_hook_s3));
    // 本地存储：按 base_url 前缀托管原图（S3 的 originalUrl 直连桶/CDN，无需本服务）
    if let Some((prefix, _dir)) = &state.originals {
        router = router.route(&format!("{prefix}/{{*path}}"), get(serve_original));
    }
    router
        .fallback(spa_fallback)
        .layer(Extension(coord))
        .with_state(state)
}

async fn serve_original(State(st): State<AppState>, AxPath(path): AxPath<String>) -> Response {
    match &st.originals {
        Some((_, dir)) => serve_file(dir, &path).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ---- 静态资源 / 首页注入 ----

async fn serve_static(State(st): State<AppState>, AxPath(path): AxPath<String>) -> Response {
    serve_file(&st.config.server.dist_dir, &path).await
}

async fn serve_thumbnail(State(st): State<AppState>, AxPath(path): AxPath<String>) -> Response {
    serve_file(&st.config.server.workdir.join("thumbnails"), &path).await
}

/// SPA history fallback：无扩展名路径返回注入后的 index.html；有扩展名但未命中静态路由 → 404。
async fn spa_fallback(State(st): State<AppState>, uri: Uri) -> Response {
    let last = uri.path().rsplit('/').next().unwrap_or("");
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
    let index_path = st.config.server.dist_dir.join("index.html");
    let Ok(html) = tokio::fs::read_to_string(&index_path).await else {
        return (StatusCode::NOT_FOUND, "index.html not found").into_response();
    };
    let manifest_json = {
        let m = st.manifest.read().await;
        serde_json::to_string(&*m).unwrap_or_else(|_| "{}".into())
    };
    let site_json = serde_json::to_string(&st.config.site).unwrap_or_else(|_| "{}".into());
    let (title, description) = site_title_desc(&st.config.site);
    // standalone：__CONFIG__ 默认全 false（即空对象，SPA 端 merge 默认值）
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
fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    if rel.split('/').any(|seg| seg == ".." || seg == ".") {
        return None;
    }
    Some(base.join(rel))
}

// ---- 触发 API ----

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
    match authorized(&st, &headers) {
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
    match authorized(&st, &headers) {
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
    if !st.config.triggers.enable_s3_event {
        return StatusCode::NOT_FOUND.into_response();
    }
    // M1b：忽略事件体细节，统一触发整体增量
    coord.trigger(false);
    StatusCode::ACCEPTED.into_response()
}

/// 鉴权：返回 None=端点禁用（未配置 token）；Some(true)=通过；Some(false)=拒绝。
fn authorized(st: &AppState, headers: &HeaderMap) -> Option<bool> {
    let token = st.config.triggers.webhook_token.as_deref()?;
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
            [storage]
            provider = "local"
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
        let state = AppState::new(config).unwrap();
        (dir, state)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn serves_static_thumbnail_spa_and_404() {
        let (_dir, state) = setup(None);
        let coord = BuildCoordinator::start(state.clone());
        let app = build_router(state, coord);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/static/web/assets/app.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("cache-control").unwrap(), IMMUTABLE);
        assert!(
            resp.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("javascript")
        );

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/thumbnails/x.jpg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/some/photo/id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("cache-control").unwrap(), NO_CACHE);
        let body = body_string(resp).await;
        assert!(body.contains("window.__MANIFEST__ = "));
        assert!(body.contains("<title>My Gallery</title>"));

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/static/web/assets/missing.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/missing.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn webhook_auth_and_status() {
        // 配置了 token 的情况
        let (_dir, state) = setup(Some("t"));
        let coord = BuildCoordinator::start(state.clone());
        let app = build_router(state, coord);

        // 无鉴权 → 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/hooks/build")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // 正确 Bearer → 202
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

        // status 可读
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("\"running\""));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn webhook_disabled_without_token() {
        let (_dir, state) = setup(None);
        let coord = BuildCoordinator::start(state.clone());
        let app = build_router(state, coord);
        // 未配置 token → 端点禁用 → 404
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/hooks/build")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn serves_local_originals() {
        // 本地存储 base_url="/photos" → 原图经 /photos/* 托管
        let (_dir, state) = setup(None);
        let coord = BuildCoordinator::start(state.clone());
        let app = build_router(state, coord);
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/photos/orig.jpg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/photos/missing.jpg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
