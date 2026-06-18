# Afilmory 上游配置 vs afilmory-lite 差异清单

> 调研自 `afilmory-main`（上游源码）。用于评估 lite 还可以补哪些配置/功能。
> 上游分**两套互不相干**的配置：`builder.config.ts`（基础设施/构建管线）与 `site.config.ts` + `config.json`（站点展示/品牌）。lite 把两套合进一个 TOML。

来源文件：`packages/builder/src/types/config.ts`、`storage/interfaces.ts`、`config/defaults.ts`、`builder.config.default.ts`、`site.config.ts`、`config.example.json`。

---

## A. builder 配置（基础设施）

### A.1 storage —— 上游支持 8 种 provider

`'s3' | 'oss' | 'cos' | 'b2' | 'github' | 'local' | 'eagle' | 'managed'`。**lite 只有 `local` 与 `s3`。**

- **S3 系（s3/oss/cos）**：bucket/region/endpoint、accessKeyId/secretAccessKey/sessionToken、prefix、customDomain、excludeRegex、maxFileLimit(默认 1000)、downloadConcurrency(16)，外加网络调优 `keepAlive`/`maxSockets`/`*TimeoutMs`/`retryMode`/`maxAttempts`/`sigV4Service`。
  - **lite 状态**：核心字段全有；网络调优字段无（性能微调，普通用户用不上）。`s3` 经 endpoint 已覆盖 R2 / MinIO / OSS / COS。
- **local**：basePath(必填)、baseUrl、`distPath`(原图拷贝目标)、excludeRegex、maxFileLimit。
  - **lite 状态**：有 base_path/base_url/exclude_regex/max_file_limit；缺 `distPath`。
- **b2 / github / eagle / managed**：**lite 完全没有**。
  - b2（Backblaze）：applicationKeyId/applicationKey/bucketId/bucketName/…
  - github：owner/repo/branch/token/path/useRawUrl/customDomain（把图片仓库当存储）。
  - eagle（Eagle 桌面图库）：libraryPath/include/exclude(tag/folder 规则)/folderAsTag。
  - managed：多租户 tenantId/upstream（嵌套真实存储）。

### A.2 processing（`config/defaults.ts`）

| 字段 | 默认 | lite |
|---|---|---|
| `defaultConcurrency` | 10 | ✅ `processing.concurrency` |
| `enableLivePhotoDetection` | true | ✅ `enable_live_photo` |
| `digestSuffixLength` | 0 | ✅ `digest_suffix_length` |
| `supportedFormats` | 一组扩展名 | ❌（lite 用内置集合，不可配） |
| `limitInputPixels` | Sharp 默认 | ❌ |

### A.3 observability / performance —— lite 完全没有

- observability：showProgress、showDetailedStats、logging(level/outputToFile/logFilePath)。lite 用 `tracing` + `RUST_LOG` 近似。
- performance.worker：workerCount(cpus*2)、timeout、useClusterMode、workerConcurrency。lite 用 tokio 并发，`processing.concurrency` 近似覆盖。

### A.4 plugins —— lite 没有插件机制

内置：`githubRepoSyncPlugin`（结果同步回 git 仓库）、**`geocodingPlugin`（GPS 反查城市/国家名）**、`thumbnailStoragePlugin`、各 `*StoragePlugin`。

> 注：lite 无插件机制，但 **`geocodingPlugin` 已直接内建移植**为 `[geocoding]` 配置段（pipeline 内联，非插件）。其余插件未移植。

### A.5 缩略图 / HEIC / EXIF（上游硬编码常量）

- 缩略图：`THUMBNAIL_WIDTH=600`、`THUMBNAIL_QUALITY=100`，JPEG。**lite 把这俩做成了可配（更灵活）**，默认值一致。
- HEIC：按扩展名自动转 JPEG（q0.95），无开关。
- EXIF：`exiftool-vendored`，超时 30s，`EXIFTOOL_PATH` 可覆盖 → 对应 lite 的 `exif.exiftool_path`。

---

## B. SiteConfig（站点展示，lite 经 `[site]` 原样透传给 `window.__SITE_CONFIG__`）

`site.config.ts:5-18`：

| 字段 | 类型 | 含义 | 默认 |
|---|---|---|---|
| `name` | string | **站点名**（非作者） | `'New Afilmory'` |
| `title` | string | 首页/tab 标题 | 同上 |
| `description` | string | SEO 描述 | `'A modern photo gallery website.'` |
| `url` | string | 站点绝对 URL | `'https://afilmory.art'` |
| `accentColor` | string | 主题色 HEX | `'#007bff'` |
| `author` | `{name,url,avatar?}` | **作者**（作者名 = `author.name`） | name `'Afilmory'` |
| `social?` | `{twitter?,github?}` | 社媒（见下） | — |
| `feed?` | `{folo?:{challenge?:{feedId,userId}}}` | Folo 订阅验证（**不是 RSS 开关**） | — |
| `map?` | `'maplibre'[]` | 地图 provider | — |
| `mapStyle?` | string | 地图样式（`'builtin'` 或 URL） | — |
| `mapProjection?` | `'globe'\|'mercator'` | 地图投影 | — |
| `beian?` | `{icp?,police?}` | 中国 ICP/公安备案 | — |

> 上游 SiteConfig **没有** keywords / 独立 ogImage / analytics / gallery-layout / EXIF 显示开关。OG 图由 SSR 层按每张照片动态生成；RSS 是前端硬编码 `/feed.xml`。

### B（重点）社媒 social

- 字段路径：**顶层 `social`**（不是 `author.social`）；值是**字符串**（用户名或完整 URL）。
- **上游只支持 2 个 key：`twitter`（=X）、`github`。** 不支持 Instagram / Telegram / Bilibili / 小红书 / email。
- 前端 `resolveSocialUrl` 自动补前缀（github→github.com/，twitter→twitter.com/ 并去掉开头 `@`）。
- 渲染组件：`apps/web/src/modules/gallery/PageHeader/PageHeaderLeft.tsx`、`MasonryHeaderMasonryItem.tsx`、`PageHeaderRight.tsx`。RSS 按钮硬编码 `/feed.xml`，不读配置。
- **想支持更多平台 → 必须改前端 SPA**（上游 dist 没这逻辑）。

---

## C. 结论：差异大 / 用户可能想要的

1. **社媒只有 github + twitter(X)**——上游限制，非 lite 缺失。要更多平台得改 SPA。
2. ~~**geocoding 插件（GPS→城市名）**——lite 没有~~ → **已移植**（`[geocoding]` 段，见下）。lite 不走插件机制，直接内建在 pipeline 里：`src/pipeline/geocoding.rs`，Auto=有 `mapbox_token` 用 Mapbox 否则 Nominatim，单次构建内存缓存 + 每 provider 限速，结果写入 `PhotoManifestItem.location`。默认关闭。
3. **b2 / github / eagle 存储源**——lite 没有，但 s3 已覆盖绝大多数云存储。
4. **local `distPath`、`supportedFormats`、`limitInputPixels`**——小缺口。
5. 其余（worker/cluster、网络超时、日志、plugins 机制）属工程细节，精简掉合理。
6. lite 在**缩略图 width/quality 可配**上比上游更灵活。

### lite `/admin` 已支持配置的 site 字段
name / title / description / url / accentColor / `author{name,url,avatar}` / `social{github,twitter}`（其余 map / beian / feed 仍可手写进 `[site]` 透传，但表单未覆盖）。
