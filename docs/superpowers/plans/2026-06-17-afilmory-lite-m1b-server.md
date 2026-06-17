# Afilmory Lite — Server + 触发器 实现计划（M1b）

> **For agentic workers:** 配合 superpowers:subagent-driven-development / executing-plans 逐任务实现。本环境 subagent 无法执行 Bash，故由控制者内联实现并以 `cargo test` 验证。步骤用 `- [ ]`。

**Goal:** 把 M1a 的 builder 核心装进一个常驻 daemon：axum serve 预构建 SPA `dist` + 运行时注入 `__MANIFEST__`/`__CONFIG__`/`__SITE_CONFIG__` + SPA history fallback + 缓存头 + `/thumbnails` 服务；并提供轮询/webhook/S3 事件/手动四类触发（去重串行）。

**Architecture:** `serve` 子命令启动 axum + 调度器；`AppState` 持有配置、`Builder`、以及 manifest 内存缓存（`RwLock`），每次 build 完成后热更新缓存。所有触发汇入一个串行 build 协调器（基于 Builder 内部互斥锁 + coalescing）。

**Tech Stack:** axum 0.8、tower（测试 oneshot）、mime_guess、tokio。复用 M1a 的 `Builder`/`config`/`manifest`。

**一致性边界：** 注入逻辑对齐 `be/apps/core` 的 `StaticWebService`（见 `docs/afilmory-feature-inventory.md` §7）。

**M1b 刻意简化：** OG/SEO meta 注入留到 M4（可选增强）；S3 事件体先只做"整体增量"（不按 key 定向）；coalescing 用"执行中再来的请求合并为下一轮一次"。

---

### Task 15: 配置扩展（listen + [triggers]）

**Files:** Modify `src/config.rs`

- [ ] **Step 1:** 给 `ServerConfig` 加 `listen`（默认 `"0.0.0.0:8080"`）。
- [ ] **Step 2:** 新增 `TriggersConfig { poll_interval_secs: u64=0, webhook_token: Option<String>, enable_s3_event: bool=false }`，并在 `Config` 加 `#[serde(default)] pub triggers: TriggersConfig`。
- [ ] **Step 3:** 测试：解析含 `[triggers] poll_interval_secs = 300` 的 TOML，断言 `listen` 默认值与 triggers 字段。
- [ ] **Step 4:** `cargo test config::`。

代码：
```rust
// ServerConfig 增加：
#[serde(default = "default_listen")]
pub listen: String,
// ...
fn default_listen() -> String { "0.0.0.0:8080".into() }

// 新增：
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TriggersConfig {
    #[serde(default)]
    pub poll_interval_secs: u64,
    #[serde(default)]
    pub webhook_token: Option<String>,
    #[serde(default)]
    pub enable_s3_event: bool,
}
// Config 增加：
#[serde(default)]
pub triggers: TriggersConfig,
```

---

### Task 16: HTML 注入（inject.rs）

**Files:** Create `src/server/mod.rs`（先挂 `mod inject;`）、`src/server/inject.rs`；Modify `src/lib.rs`（加 `pub mod server;` 与 `pub mod state;` 占位）、删除占位无关项。

注入规则（盘点 §7.1/7.2）：
- `<script id="manifest"></script>` → `<script id="manifest">window.__MANIFEST__ = <json>;</script>`
- `<script id="config">...</script>` 内容替换为 `window.__CONFIG__ = <cfg>;window.__SITE_CONFIG__ = <site>`
- `<title>...</title>` → 用 site.title 覆盖；`<meta name="description" content="...">` → 用 site.description 覆盖（清理 server-serve 残留的 `<%- ... %>`）。

- [ ] **Step 1:** 写 `inject.rs`：`pub fn inject(html: &str, manifest_json: &str, config_json: &str, site_config_json: &str, title: Option<&str>, description: Option<&str>) -> String`。用字符串替换实现（对 `<script id="manifest">` 用正则/分隔替换；`<title>` 用正则替换内容）。
- [ ] **Step 2:** 测试：给定含三个占位 + `<title>x</title>` 的样例 HTML，断言输出含 `window.__MANIFEST__ = {"version":"v10"...}`、`window.__CONFIG__`、`window.__SITE_CONFIG__`、`<title>` 被替换。
- [ ] **Step 3:** `cargo test server::inject`。

实现要点：
```rust
use regex::Regex;
pub fn inject(html: &str, manifest_json: &str, config_json: &str, site_config_json: &str,
              title: Option<&str>, description: Option<&str>) -> String {
    let mut out = html.to_string();
    // manifest
    out = replace_script_by_id(&out, "manifest", &format!("window.__MANIFEST__ = {manifest_json};"));
    // config + site config
    out = replace_script_by_id(&out, "config",
        &format!("window.__CONFIG__ = {config_json};window.__SITE_CONFIG__ = {site_config_json}"));
    // title / description
    if let Some(t) = title {
        out = Regex::new(r"(?s)<title>.*?</title>").unwrap().replace(&out, format!("<title>{}</title>", html_escape(t))).into_owned();
    }
    if let Some(d) = description {
        out = Regex::new(r#"(?s)(<meta\s+name="description"\s+content=")[^"]*(")"#).unwrap()
            .replace(&out, format!("${{1}}{}${{2}}", html_escape(d))).into_owned();
    }
    out
}
// replace_script_by_id：把 <script id="X" ...>...</script> 的内容替换为 body（保留属性）。
// 用正则 `(?s)(<script[^>]*id="X"[^>]*>).*?(</script>)` 替换中间。
```

---

### Task 17: AppState + manifest 缓存

**Files:** Create `src/state.rs`；Modify `src/lib.rs`（`pub mod state;`）

- [ ] **Step 1:** `AppState { config: Arc<Config>, builder: Arc<Builder>, manifest: Arc<RwLock<AfilmoryManifest>>, status: Arc<RwLock<BuildStatus>> }`（`tokio::sync::RwLock`）。
- [ ] **Step 2:** `AppState::new(config) -> Result<Self>`：构造 Builder，初始读 `load_manifest` 进缓存。
- [ ] **Step 3:** `async fn run_build(&self, opts) -> Result<BuildResult>`：调 `builder.build`，成功后 `*manifest.write() = load_manifest(path)`，更新 `status`。
- [ ] **Step 4:** 测试：用本地存储 + 临时 workdir，`AppState::new` 后 `run_build`，断言缓存 manifest 的 `data` 数量随之更新。
- [ ] **Step 5:** `cargo test state::`。

`BuildStatus { running: bool, last_result: Option<BuildResultSummary>, last_finished_iso: Option<String> }`（用于 `/api/status`）。

---

### Task 18: 静态服务 + 注入首页 + fallback + 缓存（server/mod.rs handlers）

**Files:** Modify `src/server/mod.rs`；新增依赖 `axum`、`mime_guess`，dev 依赖 `tower`。

路由（catch-all + 专用前缀）：
- `GET /thumbnails/{*path}` → 从 `workdir/thumbnails` serve（`immutable` 缓存）。
- `GET /static/web/{*path}` 及任意静态资源 → 从 `dist_dir` serve；带扩展名命中文件即返回（`immutable`），HTML `no-cache`。
- fallback（无扩展名、未命中文件、`/`、`/photos/:id`）→ 返回**注入后的 index.html**（`no-cache`）。

- [ ] **Step 1:** 写 `build_router(state: AppState) -> Router`：handler 用 `State<AppState>`。
- [ ] **Step 2:** `serve_index`：读 `dist_dir/index.html`，用 `inject::inject` 注入当前缓存 manifest（`serde_json::to_string`）+ `__CONFIG__`（standalone：`{}`）+ `__SITE_CONFIG__`（config.site）+ title/description（从 site 取），返回 `text/html; no-cache`。
- [ ] **Step 3:** `serve_file`（dist 与 thumbnails 共用）：防目录穿越（规范化、禁止 `..`），mime_guess 设 content-type，带扩展名静态资源 `Cache-Control: public, max-age=31536000, immutable`。
- [ ] **Step 4:** 测试（axum oneshot）：
  - 准备临时 `dist_dir`（含带占位的 index.html + `assets/app.js`）与 `workdir/thumbnails/x.jpg`。
  - 请求 `/assets/app.js` → 200 + immutable 缓存头 + 正确 content-type。
  - 请求 `/thumbnails/x.jpg` → 200。
  - 请求 `/some/spa/route`（无扩展名）→ 200 + body 含 `window.__MANIFEST__` + `no-cache`。
  - 请求 `/assets/missing.js` → 404。
- [ ] **Step 5:** `cargo test server::`。

---

### Task 19: 调度器（manual/webhook/s3-event/poll）+ 协调器 + status

**Files:** Create `src/scheduler/mod.rs`；Modify `src/server/mod.rs`（挂载 API 路由）、`src/lib.rs`。

- [ ] **Step 1:** `BuildCoordinator`：内部 `tokio::sync::Notify` + 标志，保证"执行中再来的触发合并为下一轮一次"（coalescing）。对外 `fn trigger(&self, force)`（非阻塞投递）+ 后台 worker 循环调 `state.run_build`。
- [ ] **Step 2:** API 路由：
  - `POST /api/admin/build`（body `{force?:bool}`，鉴权同 webhook token 或单独 admin token——M1b 复用 webhook_token）→ 触发。
  - `POST /api/hooks/build`（Bearer == `webhook_token`）→ 触发增量；无 token 配置则 404/禁用。
  - `POST /api/hooks/s3`（enable_s3_event）→ 解析通知体（容错：解析失败也触发整体增量）。
  - `GET /api/status` → 返回 `BuildStatus`。
- [ ] **Step 3:** 轮询：`poll_interval_secs > 0` 时后台 `interval` 循环 `trigger(false)`。
- [ ] **Step 4:** 测试：
  - 无 token 时 `POST /api/hooks/build` → 401/404；带正确 Bearer → 202。
  - `POST /api/admin/build` 触发后，轮询 `/api/status` 直到 `running=false` 且 `last_result` 有值（或直接 await 协调器测试钩子）。
- [ ] **Step 5:** `cargo test scheduler:: server::`。

---

### Task 20: `serve` 子命令 + 端到端

**Files:** Modify `src/main.rs`；Test `tests/serve_integration.rs`

- [ ] **Step 1:** clap 增加 `Serve { #[arg(long, default_value="afilmory.toml")] config: PathBuf }`。
- [ ] **Step 2:** `serve`：加载 config → `AppState::new` → 启动 `BuildCoordinator` worker + 轮询（若启用）→ `axum::serve` 绑定 `config.server.listen`。可选启动时跑一次增量 build。
- [ ] **Step 3:** 端到端测试：临时 dist + 本地 photos + workdir，构造 `AppState` + router，oneshot：
  - 先 `POST /api/admin/build`（带 token）→ 等完成。
  - `GET /` → body 含 `window.__MANIFEST__ = {...}` 且 manifest 的 `data` 含刚处理的照片。
  - `GET /thumbnails/<id>.jpg` → 200。
- [ ] **Step 4:** `cargo test`（全量）+ `cargo clippy` 干净。
- [ ] **Step 5:** 手动冒烟：`cargo run -- serve --config ...`，curl `/` 与 `/api/status`。

---

## 后续（不在本计划）
- M2：S3(SigV4) provider；存储 trait 已 async；newCount 细分；exiftool stay-open。
- M3：HEIC(libheif)、Live/Motion Photo、HDR。
- M4：OG/SEO meta 注入、OSS/COS/B2/GitHub、geocoding、远程缩略图、mozjpeg/fast_image_resize。
