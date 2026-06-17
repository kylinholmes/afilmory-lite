# Afilmory Lite (Rust) — 设计规格

- 日期：2026-06-17
- 状态：草案（待用户评审）
- 配套参考：`docs/afilmory-feature-inventory.md`（上游功能穷尽盘点）
- 上游代码：`afilmory-main/`（pnpm monorepo，作为复刻基线，只读参考）

---

## 1. 背景与目标

上游 Afilmory 的痛点：每次 S3 里的照片变化，都要重新跑 build（`pnpm build:manifest` + 重建前端）才能更新画廊。整套 Node + pnpm + sharp + exiftool 工具链重、用起来麻烦。

**本项目目标**：用 Rust 写一个**单二进制常驻 daemon**，它：
1. **Serve** 一份预构建好的 Afilmory SPA 静态壳（运行时注入数据，永不重建前端）。
2. 在内部**复刻 Afilmory 的 build 管线**（从存储拉照片 → 处理 → 产出 manifest + 缩略图）。
3. 通过 **webhook / 定时轮询 / S3 事件通知 / 手动端点**触发增量更新，**无需重新 build Afilmory**。

### 1.1 一致性目标（已与用户确认）
**结构/语义一致**，非字节级一致：
- manifest 字段结构、序列化语义（"必出可 null" vs "无值省略键"）与上游 v10 完全一致。
- 预构建的 SPA 壳能正确渲染；缩略图视觉一致；EXIF 显示值一致；HDR/Live/Motion/tags/日期等行为一致。
- **不要求**与上游 TS builder 逐字节相同：`digest`(sha256)、缩略图 JPEG 字节、`thumbHash` 比特允许实现内自洽即可（Rust 是 manifest 唯一生成者，SPA 不依赖跨实现一致）。

### 1.2 非目标
- 不复刻上游的多进程 cluster 模型、TUI、`be/apps/core` 的 DB/多租户/dashboard、SSR(Next.js)。
- 不负责 build 前端 SPA（由外部 CI/Docker 完成，见 §10）。
- v1 不做：OSS/COS/B2/GitHub/Eagle provider、geocoding、远程缩略图存储、github-repo-sync、动态 OG PNG、旧 manifest 迁移导入（均列为后续可选）。

---

## 2. 已锁定的关键决策

| 项 | 决策 |
|---|---|
| 一致性 | 结构/语义一致，非字节级 |
| 实现语言 | 纯 Rust 单二进制 + 两个系统依赖（`exiftool`、`libheif`） |
| EXIF | `ExifExtractor` trait；默认 **`exiftool` CLI 子进程**（`-json`，stay-open 复用），可选纯 Rust 快路径 |
| 存储 v1 | **S3 及兼容**（手写 SigV4）+ **本地目录**；trait 化，后续加 OSS/COS/B2/GitHub |
| 格式 | 对齐 Afilmory：常规格式 + **HEIC**(libheif) + **Live Photo** + **Motion Photo**；HEIC 若 libheif 太难则降级（见 §6.4） |
| 触发 | 四种全做：定时轮询 / 鉴权 webhook / S3 事件通知 / 手动端点 |
| 并发 | tokio 任务 + 信号量限流，CPU 活走 `spawn_blocking`/`rayon`；**一次 build = 全局互斥锁串行任务** |
| SPA 交付 | 运行时指向 dist 目录（`BUILD_FOR_SERVER_SERVE=1 AFILMORY_EMBED_MANIFEST=false` 产物） |

---

## 3. 总体架构

单进程，三大子系统共享一份配置与状态目录：

```
                 ┌─────────────── afilmory-lite (单二进制) ───────────────┐
   浏览器  ──HTTP──▶  server/   静态 serve dist + 运行时注入 __MANIFEST__   │
                 │             + SPA fallback + 缓存头 + (可选 OG/SEO)      │
                 │                       ▲ 读 manifest                     │
   轮询/webhook ─▶  scheduler/  四类触发 → 去重/排队 → 加锁调用 builder     │
   /S3事件/手动     │                       │                              │
                 │  builder.rs  列举 → 增量筛选 → 并发处理 → 保存 manifest   │
                 │     │            │              │                        │
                 │  storage/     pipeline/      manifest/                  │
                 │  (S3/Local)   (单图处理)     (模型/读写/增量/删除)        │
                 │                  │                                       │
                 │               exif.rs (exiftool CLI)                    │
                 └──────────────────────────────────────────────────────────┘
                          工作目录:  manifest.json  +  thumbnails/<id>.jpg
```

**数据流**：触发 → `builder.build(opts)` 加锁执行 → 列举存储 → 与现有 manifest 比对筛选增量 → 并发跑 pipeline → 写 `manifest.json` + `thumbnails/` → server 下次请求即注入新 manifest。原图不经过本服务（`originalUrl` 直指 S3/CDN）。

---

## 4. 模块结构

```
src/
  main.rs            启动：解析 config → 初始化共享状态 → 起 axum server + scheduler，select! 常驻
  config.rs          TOML 配置解析与校验（见 §11）
  state.rs           AppState：config、当前 manifest 缓存(RwLock)、build 互斥锁(Mutex)、build 状态
  error.rs           统一 error 类型（thiserror），绝不 process::exit

  storage/
    mod.rs           StorageProvider trait + StorageObject + 工厂
    s3.rs            S3/兼容：手写 SigV4 + reqwest + quick-xml 列举
    sigv4.rs         SigV4 签名（ring/hmac/sha2）
    local.rs         本地目录递归扫描
    live_photo.rs    detect_live_photos：同名 .mov/.mp4 配对

  exif/
    mod.rs           ExifExtractor trait + PickedExif 裁剪（白名单）
    exiftool.rs      exiftool CLI stay-open 子进程（默认）
    native.rs        可选纯 Rust 快路径（nom-exif），feature-gated

  pipeline/
    mod.rs           process_photo：编排单图全步骤
    decode.rs        HEIC(libheif)/BMP/常规 解码 → RGBA/尺寸；orientation 处理
    thumbnail.rs     600px JPEG 缩略图（fast_image_resize + mozjpeg）
    thumbhash.rs     100px → thumbhash crate → hex
    tone.rs          直方图(BT.709) + toneAnalysis
    hdr.rs           GainMap 检测
    motion_photo.rs  图内嵌视频检测（ContainerDirectory / ftyp）
    info.rs          title/dateTaken/tags/description 抽取

  manifest/
    model.rs         serde 数据模型（AfilmoryManifest / PhotoManifestItem / ...）
    store.rs         读/写 manifest.json（2 空格缩进）、排序、cameras/lenses 聚合
    incremental.rs   needs_update / should_process / filter_tasks / handle_deleted

  builder.rs         build(opts: BuildOptions) -> BuildResult；全局锁；进度上报

  server/
    mod.rs           axum router + 启动
    static_files.rs  serve dist + history fallback + 缓存头
    inject.rs        index.html 注入 __MANIFEST__/__CONFIG__/__SITE_CONFIG__ + <title>
    og.rs            (可选) photo/homepage OG meta 字符串注入

  scheduler/
    mod.rs           触发统一抽象 → 单一 build 队列（去重/合并）
    poll.rs          定时轮询
    webhook.rs       鉴权 webhook 端点
    s3_event.rs      S3/SNS/SQS/EventBridge 事件解析端点
    manual.rs        手动触发端点（含 force 选项）
```

---

## 5. 存储层（storage/）

### 5.1 trait
```rust
#[async_trait]
trait StorageProvider: Send + Sync {
    async fn list_images(&self) -> Result<Vec<StorageObject>>;      // 按 10 个扩展名过滤
    async fn list_all_files(&self) -> Result<Vec<StorageObject>>;   // 全部（用于 live photo 配对）
    async fn get_file(&self, key: &str) -> Result<Option<Bytes>>;
    fn generate_public_url(&self, key: &str) -> String;
    fn detect_live_photos(&self, all: &[StorageObject]) -> HashMap<String, StorageObject>;
}
struct StorageObject { key: String, size: Option<u64>, last_modified: Option<DateTime<Utc>>, etag: Option<String> }
```
- 图片扩展名集合（小写、纯扩展名）：`jpg jpeg png webp bmp tiff tif heic heif hif`。
- 视频集合（Live Photo）：`mov mp4`；同目录同 basename 配对。

### 5.2 S3 及兼容（s3.rs + sigv4.rs）
- **手写 SigV4**（header-based），**不用 aws-sdk/rust-s3/object_store**（URL/endpoint/service-name 行为需逐条复刻，见盘点 §3）。
- 列举：`GET ?list-type=2`，`quick-xml` 解析 `ListBucketResult`；分页规则照搬（`maxFileLimit ≤ 1000` 只发一页；每页 `min(limit,1000)`）。
- `generate_public_url`：customDomain 优先（直拼 key 不 encode）；否则 virtual-host（AWS/OSS/COS 各自域名）或自定义 endpoint 的 path-style；`encodeS3Key` 逐段 encode。region 默认 `us-east-1`。
- 下载：`Semaphore(download_concurrency=16)`；重试 `max_attempts=3`，整请求超时 60s、读空闲超时 10s，退避 `min(4000,300*2^(n-1))`+抖动；失败返回 `None`。

### 5.3 本地目录（local.rs）
- 递归扫描 `base_path`；etag = `mtime-size`；`excludeRegex` 在扫描时对相对路径匹配；`maxFileLimit` 扫完后截断。
- `generate_public_url`：`base_url` 存在则 `base_url + key`，否则 `file://`。（本地通常配合把原图也 serve 出去，见 §9 注意点。）

---

## 6. 单图处理管线（pipeline/）

`process_photo(obj, existing, live_map, opts) -> Result<PhotoManifestItem>`，步骤与 manifest 字段严格对齐盘点 §2。CPU-bound 步骤在 `spawn_blocking` 中执行。

### 6.1 解码（decode.rs）
- 取原图 bytes（storage.get_file 或预取）。
- HEIC/HEIF/HIF → libheif 解码为 RGB(A)（替代 heic-convert）。
- BMP（魔数 `42 4D`）→ `image` 解码。
- 常规 → `image` 解码。
- 取 `width/height`；按 EXIF orientation∈{5,6,7,8} 交换宽高。
- `digest = sha256(规范化后字节).hex`（自洽用途：增量去重）。

### 6.2 缩略图 + thumbHash
- 缩略图：按 EXIF 旋转 → `fast_image_resize` 缩到宽 600（不放大，等比）→ `mozjpeg` quality 100 → 写 `thumbnails/<id>.jpg`，URL `/thumbnails/<id>.jpg`。
- thumbHash：缩略图 → resize 100×100(fit inside) → RGBA → `thumbhash` crate → `hex::encode`。

### 6.3 EXIF / HDR / Motion / Live / 影调 / info
- EXIF：见 §7。
- HDR：`ContainerDirectory` 含 `Semantic=GainMap && Length`，或 `MPImageType=='Gain Map Image'`、`UniformResourceName==urn:...21496:-1`。
- Motion Photo：用 **raw bytes**，从 `ContainerDirectory`/`MicroVideoOffset` 定位内嵌视频，校验 `ftyp`、≥8KB。
- Live Photo：从 `live_map` 取配对视频 → `generate_public_url`。
- Motion 与 Live 互斥 → 冲突则该图标记失败（跳过，不中断整批）。
- 影调：histogram(256×256, sRGB, BT.709 亮度) → brightness/contrast/shadow/highlight/toneType；异常回退 `{normal,50,50,0.33,0.33}`。
- info：tags=目录每级；dateTaken=EXIF DateTimeOriginal→文件名日期→当前时间；title=文件名清洗；description=`""`。

### 6.4 HEIC 退路
libheif 是唯一 C 依赖。若集成成本过高，降级顺序：① 启动时检测 libheif，缺失则对 HEIC 走 fallback；② fallback 候选：shell 调 `heif-convert`(libheif-tools) 或 `vips` CLI 转 JPEG 再处理；③ 最次：HEIC 跳过并在日志/状态里标注（用户体验上 HEIC 不显示，但其余正常）。默认目标是 libheif 直连。

---

## 7. EXIF（exif/）

```rust
#[async_trait]
trait ExifExtractor { async fn extract(&self, jpeg_or_path: &Path, raw_for_heic: Option<&[u8]>) -> Result<Option<PickedExif>>; }
```
- **默认实现 `exiftool.rs`**：以 `-stay_open` 模式常驻一个 `exiftool` 子进程，`-json -n?`（数值/文本策略需对齐上游 PrintConv 文本输出，默认不加 `-n` 以保留文本值如 `"Aperture-priority AE"`），按盘点 §1.3 的 `pickKeys` 白名单裁剪，日期转 ISO，`GPSAltitudeRef` 归一化 0/1。
- HEIC 时把 raw 原图喂给 exiftool（与上游一致）。
- daemon 下子进程常驻复用，优雅关闭时再退出（不注册每次 build 的关闭钩子）。
- 可选 `native.rs`（feature `native-exif`）：`nom-exif` 抽基础字段做快路径，缺失字段再回落 exiftool。v1 可不启用。

---

## 8. Manifest（manifest/）

### 8.1 数据模型（model.rs，serde）
严格按盘点 §1 定义；序列化关键：
- `thumb_hash/exif/tone_analysis/location`：`Option<T>`，**序列化为 null**（不 skip）。
- `digest/video/og_image_url`：`Option<T>` + `skip_serializing_if = "Option::is_none"`（无值省略键）。
- `is_hdr`：上游构造时**总被赋值**（true/false 恒出现），故定为始终输出的 `bool`，不用 Option。
- 字段顺序按上游构造字面量顺序排列 struct 字段（serde 按声明序输出）。
- 写出：`serde_json::to_string_pretty`（2 空格）；顶层 `version="v10", data, cameras, lenses`。
- `video` 用内部 tag 枚举 `#[serde(tag="type")]`：`live-photo` / `motion-photo`。

### 8.2 读写 / 聚合 / 排序（store.rs）
- `data` 按 `dateTaken` 降序；`cameras/lenses` 去重键 `displayName`、`localeCompare` 升序（Rust 用合适的排序，CJK/重音排序差异记为已知风险，§13）。
- manifest 路径、thumbnails 路径来自 config（默认 `<workdir>/manifest.json`、`<workdir>/thumbnails/`）。

### 8.3 增量 / 删除（incremental.rs）
- `needs_update`：无 existing→true；无 last_modified→true；否则 `obj.last_modified > existing.last_modified`。
- `should_process`：force / 新图 / lastModified 字符串不等或 force-manifest / 缩略图缺失或 force-thumbnails → 处理。
- 字段级复用：非 force 时复用 existing 的 thumbnail(有 thumbHash+文件)、exif、toneAnalysis、location。
- `handle_deleted`：扫 thumbnails 目录，文件名(去`.jpg`)不在 manifest id 集合 → 删；manifest 空 → 清空目录。仍存在于存储但本轮未处理的旧项补回 manifest。

---

## 9. Server（server/）

基于 axum + tower-http。职责（盘点 §7、§10）：
1. **静态 serve**：dist 目录挂 `/static/web/*`（与构建 base 对齐）；mime 猜测；防目录穿越。
2. **HTML 注入**（每次请求实时改写 index.html，结果不缓存）：
   - `<script id="manifest">` ← `window.__MANIFEST__ = <当前 manifest JSON>;`（**必须**；manifest 从 `AppState` 的 RwLock 缓存取，build 完成后热更新缓存）。
   - `<script id="config">` ← `window.__CONFIG__ = {...};window.__SITE_CONFIG__ = {...}`。standalone：`__CONFIG__={}`（默认全 false）；`__SITE_CONFIG__` 来自 config 的 site 段。
   - 覆盖 `<title>` 与 `meta[name=description]`（清理 server-serve 产物里残留的 `<%- title %>` 字面量）。
3. **SPA history fallback**：无扩展名路径且非已存在文件 → 注入后的 index.html(200)；有扩展名找不到 → 404。
4. **缓存头**：HTML `no-cache`（必须）；带扩展名静态资源 `public, max-age=31536000, immutable`。
5. **缩略图可达**：`/thumbnails/<id>.jpg` 由本服务静态 serve（来自工作目录的 thumbnails/）。`thumbnailUrl` 在 manifest 里写成同源相对路径。
6. **（可选）OG/SEO**：`/photos/:id` 与 `/` 运行时注入 og/twitter meta 字符串（零依赖）。动态 OG PNG 不做。

> 注意：`thumbnailUrl` 的 base 需与 serve 路由对齐。若 SPA base=`/static/web/`，缩略图既可放 dist 内随静态资源走，也可单独路由 `/thumbnails`（更适合 daemon 动态产出）——v1 采用**独立 `/thumbnails` 路由指向工作目录**，使更新缩略图无需触碰 dist。manifest 里 `thumbnailUrl` 写 `/thumbnails/<id>.jpg`，并在注入/serve 时保证该路径可达（必要时调整 SPA 期望的前缀）。

---

## 10. Scheduler（scheduler/）

统一抽象：所有触发源 → 投递一个 `BuildRequest{ force_flags, source }` 到单一 worker；worker 持 `Mutex` 串行执行 `builder.build`。**并发触发去重/合并**：执行中再来的请求合并为"下一轮跑一次"（coalesce），避免堆积。

四类触发：
- **poll.rs**：`tokio::time::interval(poll_interval)` → 普通增量（force 全 false）。间隔可配，默认建议 300s。
- **webhook.rs**：`POST /api/hooks/build`，校验 `Authorization: Bearer <token>`（config 配 token）→ 增量。可带 `?force=...`。
- **s3_event.rs**：`POST /api/hooks/s3`，解析 S3→SNS/SQS/EventBridge 通知体（也兼容 Cloudflare R2 等的 JSON）→ 增量（可按事件里的 key 做定向处理优化，v1 先整体增量）。
- **manual.rs**：`POST /api/admin/build`（鉴权）支持 `force/force-manifest/force-thumbnails`；可选 CLI 子命令 `afilmory-lite build --force`。

可观测：`GET /api/status` 返回最近 build 结果（newCount/processed/skipped/deleted/total、耗时、最后成功时间、是否正在跑）。

---

## 11. 配置（config.rs，TOML）

```toml
[server]
listen = "0.0.0.0:8080"
dist_dir = "/app/web/dist"          # 预构建 SPA 产物
workdir  = "/app/data"              # manifest.json + thumbnails/

[site]                              # 注入 __SITE_CONFIG__
name = "..."; title = "..."; description = "..."; url = "..."; accent_color = "#007bff"
# author/social/map/beian 等，对齐 SiteConfig

[storage]
provider = "s3"                     # 或 "local"
bucket = "..."; region = "..."; endpoint = "..."; prefix = ""
access_key_id = "..."; secret_access_key = "..."   # 支持环境变量覆盖
custom_domain = ""; exclude_regex = ""; max_file_limit = 1000; download_concurrency = 16
# provider="local": base_path / base_url

[processing]
concurrency = 10
enable_live_photo = true
digest_suffix_length = 0
thumbnail_width = 600
thumbnail_quality = 100

[exif]
mode = "exiftool"                   # 或 "native"
exiftool_path = "exiftool"

[triggers]
poll_interval_secs = 300            # 0 = 关闭轮询
webhook_token = "..."               # 空 = 关闭 webhook
enable_s3_event = true
```
敏感项（密钥/token）支持 `${ENV_VAR}` 展开或环境变量覆盖。

---

## 12. 部署与前端 build（外部）

- **前端**：由 CI（GitHub Action）或 Docker 内脚本执行
  `AFILMORY_EMBED_MANIFEST=false BUILD_FOR_SERVER_SERVE=1 pnpm --filter @afilmory/web build`
  产出 `apps/web/dist/`，作为构建产物放到运行环境的 `dist_dir`。本仓库提供该脚本/Dockerfile 模板（不在 Rust 二进制职责内）。
- **运行时系统依赖**：`exiftool`（`apt install libimage-exiftool-perl`，自带 Perl）、`libheif`（`apt install libheif1 libheif-dev`，HEIC）。提供包含这些依赖的运行镜像。
- **Rust 二进制**：读 config → serve `dist_dir` + 工作目录 → 起调度。

---

## 13. 测试与验证策略

- **黄金对比**：用一组样本图（含 jpg/png/webp/heic/live/motion），跑上游 `pnpm build:manifest` 得到基准 manifest；跑 Rust 得到 manifest；对比**结构与语义**——逐字段比对，但对 volatile 字段（`digest`、`thumbHash`、缩略图字节）只校验"存在性/类型/格式"而非具体值；对 EXIF 校验关键显示字段一致。
- **缩略图视觉**：抽样人工/感知哈希(pHash)比对，确认视觉等价。
- **SPA 冒烟**：把 Rust 产出的 manifest + thumbnails 喂给预构建 SPA 壳，确认渲染、详情、地图、Live/Motion 播放正常。
- **增量正确性**：改一张、删一张、加一张，验证 newCount/skipped/deleted 与缩略图清理正确。
- **并发安全**：并发打 webhook + 轮询，确认串行化、无 manifest 撕裂。
- 单元测试：SigV4 签名（对已知用例）、URL 拼接各分支、manifest 序列化（"必出 null" vs "省略键"）、增量判定、live photo 配对、tone 计算。

---

## 14. 里程碑（建议）

- **M1 端到端最小闭环**：Local provider + 常规格式（jpg/png/webp）+ exiftool + 缩略图/thumbhash/tone/info + manifest 读写 + server 注入/serve/fallback + 手动触发。能让 SPA 壳跑起来。
- **M2 S3 + 增量 + 调度**：S3(SigV4) provider + 增量/删除 + 轮询 + webhook + S3 事件 + 状态端点 + 锁/去重。
- **M3 特殊格式**：HEIC(libheif) + Live Photo + Motion Photo + HDR。
- **M4 增强**：OG/SEO meta、native-exif 快路径、OSS/COS/B2/GitHub provider、geocoding、远程缩略图存储（按需）。

---

## 15. 已知风险

1. **HEIC/libheif** 集成（C 依赖、跨平台构建）——有退路（§6.4）。
2. **EXIF 显示值对齐**：完全依赖 exiftool CLI；若改 native 快路径需注意字段语义差异。
3. **cameras/lenses 排序**：上游用 JS `localeCompare`（locale 相关），Rust 排序对 CJK/重音可能略有差异，但不影响功能。
4. **缩略图 base/路径前缀**：需确认 SPA 对 `thumbnailUrl` 前缀的期望（`/thumbnails` vs `/static/web/thumbnails`），M1 验证。
5. **S3 事件体格式多样**（SNS/SQS/EventBridge/R2），需逐个适配；v1 先整体增量、不做定向 key 优化。
6. `dateTaken` 缺失时用当前时间 → 非确定性；与上游一致，接受。
