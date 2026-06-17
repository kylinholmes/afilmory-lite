# Afilmory 功能清单（Rust lite 复刻基线）

> 目的：在用 Rust 重写之前，把上游 Afilmory（`afilmory-main/`）"build 管线 + serve 注入层"的全部现有行为穷尽式记录下来。
> 一致性目标：**结构/语义一致**（manifest 字段结构相同、预构建 SPA 能正确渲染、缩略图视觉一致、EXIF 显示值一致），**不追求与 TS builder 字节级相同**——Rust 将是 manifest 的唯一生成者，比特级对齐既做不到也无必要。
> 盘点日期：2026-06-17。行号引用相对 `afilmory-main/`。

---

## 0. 总体架构

Afilmory 是「一条 build 管线 + 同一个 SPA 的三种 serving 模式」。三种模式跑**完全相同的 `apps/web` SPA**，唯一区别是**谁注入 `window.__MANIFEST__`**：

```
packages/builder (CLI 管线)
  storage sync (S3/OSS/COS/B2/GitHub/Local/Eagle)
    → 每张图: 解码 → 缩略图 → thumbHash → EXIF → HDR/Live/Motion 检测 → 影调 → 反向地理编码
      → photos-manifest.json (+ public/thumbnails/<id>.jpg)
                │
   ┌────────────┼─────────────────────────────┐
 apps/ssr     apps/web 独立构建            be/apps/core (Hono)
 (Next 注入)   (Vite 构建期内联)            (运行时从 DB 注入)
```

**Rust lite 要做的 = 取代「builder 管线」+「be/apps/core 的 serve 注入职责」，并把一次性 CLI 改成常驻 daemon（webhook + 轮询触发增量更新）。**

数据契约（manifest）是整个系统的中枢——只要 Rust 产出的 manifest 结构正确、缩略图就位，预构建的 SPA 壳就能正常工作。

---

## 1. 数据契约：Manifest

### 1.1 顶层 `AfilmoryManifest`
`packages/typing/src/manifest.ts:17-22`
```ts
type AfilmoryManifest = {
  version: ManifestVersion   // `v${number}`，当前 "v10"
  data: PhotoManifestItem[]  // 按 dateTaken 降序
  cameras: CameraInfo[]      // 去重相机，按 displayName 升序
  lenses: LensInfo[]         // 去重镜头，按 displayName 升序
}
```
- 当前版本常量 `CURRENT_MANIFEST_VERSION = 'v10'`（`manifest/version.ts:3`）。保存时**总是写常量**，覆盖内存里的旧版本。
- 写出路径：`apps/web/src/data/photos-manifest.json`（`manifest/manager.ts:12`，经 `workdir`）。
- 序列化：`JSON.stringify(obj, null, 2)`（**2 空格缩进、无尾换行**），顶层键顺序固定 `version, data, cameras, lenses`。
- **SPA 承重要求**（`packages/data/src/index.ts:16-22`）：`data / cameras / lenses` 三键必须存在（无条件读取，缺则 SPA 白屏）；空相册合法 = 四键齐全、数组为空。item 级别 PhotoLoader 直接读 `id`（建 map）和 `tags`（`getAllTags`，必须是数组）。

### 1.2 `PhotoManifestItem`（逐字段，JSON 顺序以构造字面量为准）
构造点 `photo/image-pipeline.ts:215-256`；类型 `packages/typing/src/photo.ts:52-72`（extends `PhotoInfo`）。

| # | 字段 | 类型 | 序列化语义 | 来源 |
|---|------|------|-----------|------|
| 1 | `id` | `string` | 必出 | `generatePhotoId(s3Key)`，默认 = 去目录去扩展名的 basename |
| 2 | `format` | `string` | 必出 | 扩展名去点大写，空→`"UNKNOWN"` |
| 3 | `title` | `string` | 必出 | `extractPhotoInfo` |
| 4 | `description` | `string` | 必出 | 恒为 `''`（硬编码，未实现） |
| 5 | `dateTaken` | `string` | 必出 | ISO；EXIF `DateTimeOriginal` → 文件名日期 → 当前时间 |
| 6 | `tags` | `string[]` | 必出 | 目录路径每一级 = 一个 tag |
| 7 | `originalUrl` | `string` | 必出 | `storageManager.generatePublicUrl(s3Key)` |
| 8 | `thumbnailUrl` | `string` | 必出 | `/thumbnails/<id>.jpg` |
| 9 | `thumbHash` | `string \| null` | **必出，可为 null** | thumbhash 二进制的小写 hex 串 |
| 10 | `width` | `number` | 必出 | sharp metadata（orientation∈{5,6,7,8} 时与 height 交换） |
| 11 | `height` | `number` | 必出 | 同上 |
| 12 | `aspectRatio` | `number` | 必出 | `width / height`（浮点） |
| 13 | `s3Key` | `string` | 必出 | 存储 key（增量/去重主键） |
| 14 | `lastModified` | `string` | 必出 | `obj.LastModified.toISOString()`（带毫秒+Z），缺省=当前时间 |
| 15 | `size` | `number` | 必出 | `obj.Size \|\| 0` |
| 16 | `digest` | `string?` | **无值省略键** | `sha256(处理后 imageBuffer).hex` |
| 17 | `exif` | `PickedExif \| null` | **必出，可为 null** | 见 1.3 |
| 18 | `toneAnalysis` | `ToneAnalysis \| null` | **必出，可为 null** | 见 1.4 |
| 19 | `location` | `LocationInfo \| null` | **必出，可为 null** | 反向地理编码（默认继承 existing） |
| 20 | `video` | `VideoSource?` | **无值省略键** | 见 1.4 |
| 21 | `isHDR` | `boolean?` | 实际总被赋值（true/false） | 见下 |
| (+) | `ogImageUrl` | `string \| null` | 默认不输出，仅 og 插件启用时追加在末尾 | — |

**序列化关键区分（Rust serde）**：
- `thumbHash / exif / toneAnalysis / location` → `Option<T>` 但 **必须序列化成 `null`**（不要 `skip_serializing_if`）。
- `digest / video / isHDR / ogImageUrl` → `Option<T>` + `#[serde(skip_serializing_if = "Option::is_none")]`（无值省略键）。

**算法细节**：
- `id` 默认 = `basename(s3Key, ext)`；若 `digestSuffixLength > 0` 则 `<baseName>_<sha256(s3Key 字符串).hex[0..N]>`（默认 N=0 不加后缀）。
- `digest` = `sha256(处理后 imageBuffer 全字节).hex`（HEIC/BMP 转码后的 buffer）。注意 `id` 哈希的是 **key 字符串**，`digest` 哈希的是 **图片字节**。
- `thumbHash` = `compressUint8Array(thumbhash)` = 每字节 2 位小写 hex 拼接（**非 base64**，Rust `hex::encode` 等价）。
- `isHDR` = `MPImageType === 'Gain Map Image' || UniformResourceName === 'urn:iso:std:iso:ts:21496:-1' || hasGainMap`。

### 1.3 `PickedExif`
`packages/typing/src/photo.ts:86-185`。EXIF 实际抽取在 `image/exif.ts`，用 **exiftool-vendored**（Perl exiftool 子进程）。流程：写临时文件（HEIC 写 raw 原图）→ `exiftool.read()` → **白名单裁剪**（只保留 `pickKeys` 列出的字段）→ 删 warnings/errors。

- 存的是 **exiftool 已格式化的文本值**（如 `Flash: "Off, Did not fire"`、`WhiteBalance: "Auto"`、`ExposureProgram: "Aperture-priority AE"`），不是原始枚举数字。**这是「走 exiftool CLI」的根本原因**——纯 Rust crate 只能拿到原始数值。
- 日期字段经 `formatExifDate` → `new Date(...).toISOString()`。
- `pickKeys` 白名单（完整）：`tz, tzSource, Orientation, Make, Model, Software, Artist, Copyright, ExposureTime, FNumber, ExposureProgram, ISO, OffsetTime, OffsetTimeOriginal, OffsetTimeDigitized, ShutterSpeedValue, ApertureValue, BrightnessValue, ExposureCompensationSet/Mode/Setting, ExposureCompensation, MaxApertureValue, LightSource, Flash, FocalLength, ColorSpace, ExposureMode, FocalLengthIn35mmFormat, SceneCaptureType, LensMake, LensModel, MeteringMode, WhiteBalance, WBShiftAB, WBShiftGM, WhiteBalanceBias, FlashMeteringMode, SensingMethod, FocalPlaneXResolution, FocalPlaneYResolution, Aperture, ScaleFactor35efl, ShutterSpeed, LightValue, Rating, GPSAltitude, GPSCoordinates, GPSAltitudeRef, GPSLatitude, GPSLatitudeRef, GPSLongitude, GPSLongitudeRef, MPImageType, UniformResourceName, MotionPhoto, MotionPhotoVersion, MotionPhotoPresentationTimestampUs, ContainerDirectory, MicroVideo, MicroVideoVersion, MicroVideoOffset, MicroVideoPresentationTimestampUs`。
- 额外合并：`ImageWidth/ImageHeight`（来自 `ExifImageWidth/Height`）、`FujiRecipe`（当存在 `FilmMode`）、`SonyRecipe`（当存在 `CreativeStyle`）。
- 下游真正消费的字段：`DateTimeOriginal`（→dateTaken）、`GPS*`（→location）、`Make/Model/LensMake/LensModel`（→cameras/lenses）、`MPImageType/UniformResourceName/ContainerDirectory`（→HDR）、`MotionPhoto/MicroVideo/...`（→Motion Photo）。
- v9→v10 迁移把 `GPSAltitudeRef` 归一化为数字 0/1（`'Below Sea Level'`→1，其它→0）。

### 1.4 `ToneAnalysis` / `VideoSource` / `LocationInfo` / `CameraInfo` / `LensInfo`
```ts
ToneAnalysis = { toneType: 'low-key'|'high-key'|'normal'|'high-contrast', brightness:0-100, contrast:0-100, shadowRatio:0-1, highlightRatio:0-1 }
VideoSource =
  | { type:'live-photo',  videoUrl:string, s3Key:string }
  | { type:'motion-photo', offset:number, size?:number, presentationTimestamp?:number }
LocationInfo = { latitude, longitude, country?, city?, locationName? }
CameraInfo = { make:string, model:string, displayName:string }   // displayName = `${make} ${model}`
LensInfo   = { make?:string, model:string, displayName:string }  // make 无值省略键
```
- cameras/lenses 由 manifest 聚合：去重键 = `displayName`，Make/Model/LensModel 先 `.trim()`，排序 `displayName.localeCompare()` 升序（**Rust 需注意 locale 排序细节**）。
- video 三态：motion-photo 优先 → live-photo → 省略。两者互斥，同时出现会抛错。

---

## 2. 单张照片处理管线
入口 `executePhotoProcessingPipeline`（`photo/image-pipeline.ts:151-264`）。依赖：`sharp@0.35.1`、`thumbhash@0.1.1`、`heic-convert@2.1.0`、`exiftool-vendored@36.0.0`、`@vingle/bmp-js`。（`blurhash` 已安装但**完全未使用**。）

有序步骤：
1. **生成 photoId**（见 1.2）。
2. **预处理 / 取原图**：`prefetchedBuffers` 或 `storageManager.getFile(key)` → `preprocessImageBuffer`：HEIC/HEIF/HIF（扩展名判定）→ `heic-convert({format:'JPEG', quality:0.95})`；其它原样。
3. **建 Sharp 实例 + BMP 转换 + 元数据**：`sharp(buf, {limitInputPixels})`；BMP（魔数 `0x42 0x4d`）→ `@vingle/bmp-js` 解码 + `sharp().jpeg()` 重编码；取 `{width,height}`（orientation∈{5,6,7,8} 交换宽高）。
4. **contentDigest** = `sha256(处理后 buffer).hex`。
5. **缩略图 + thumbHash**：
   - 缩略图：`sharp(buf).rotate().resize(600, null, {withoutEnlargement:true}).jpeg({quality:100})` → 写 `public/thumbnails/<id>.jpg`，URL `/thumbnails/<id>.jpg`。（`.rotate()` 按 EXIF 自动旋转；宽 600 等比；不放大。）
   - thumbHash：函数名叫 `generateBlurhash` 但**实际用 thumbhash 库**：`sharp(缩略图).resize(100,100,{fit:'inside'}).raw().ensureAlpha()` → `rgbaToThumbHash(w,h,data)` → `Uint8Array` → hex 串。
6. **EXIF**（见 1.3）。
7. **GainMap/HDR 检测**：`ContainerDirectory` 中存在 `Item.Semantic==='GainMap' && Item.Length`。
8. **Motion Photo 检测**（用 **raw buffer**）：flag = `MotionPhoto||MicroVideo`；标准格式从 `ContainerDirectory` 找 `Semantic==='MotionPhoto'`，`offset = rawLen - Item.Length`，校验 `ftyp` 框（前 32 字节）且视频 ≥8KB；legacy 回退用 `MicroVideoOffset`。
9. **Live Photo 处理**：从外部 `livePhotoMap.get(key)` 取配对视频 → `generatePublicUrl`。
10. **冲突校验**：Motion + Live 同时存在 → 抛错（该图处理失败）。
11. **影调分析**：`histogram` = `sharp.clone().toColourspace('srgb').resize(256,256,{fit:'inside'}).raw()`；亮度 = `0.2126R+0.7152G+0.0722B`（BT.709）；算 brightness/contrast(stdDev/127.5)/shadowRatio(lum[0..85])/highlightRatio(lum[170..255]) → toneType。catch 回退 `{normal,50,50,0.33,0.33}`。
12. **info 抽取**：tags = 目录每级；dateTaken = EXIF→文件名 `\d{4}-\d{2}-\d{2}`→当前时间；title = 文件名去日期/views/分隔符；description = `''`。
13. **组装 manifest item**（见 1.2）。

地理位置不在同步管线内——由 geocoding 插件在 `afterPhotoProcess` 异步补 `location`（默认禁用）。

---

## 3. 存储 Provider
接口 `storage/interfaces.ts:41-103`，真实实现在 `storage/providers/`（`plugins/storage/*` 只是注册薄包装）。Builder 只调：`listAllFiles()` / `detectLivePhotos()` / `listImages()` / `getFile()` / `generatePublicUrl()`。内置 7 个：`s3`/`oss`/`cos`（同一 `S3StorageProvider`）、`b2`、`github`、`local`、`eagle`。

- **图片扩展名集合**（`SUPPORTED_FORMATS`，小写、纯扩展名判定、不读 MIME）：`.jpg .jpeg .png .webp .bmp .tiff .tif .heic .heif .hif`。
- **Live Photo 配对**：视频集合仅 `.mov .mp4`；同目录、同 basename（去扩展名）配一对。S3/GitHub/B2 basename 大小写敏感，Local 不敏感。Eagle 未实现（返空）。
- **S3 客户端**：**不用 aws-sdk**，手写 **SigV4（header-based）+ `fetch`**（`s3/client.ts`）。`region` 默认 `us-east-1`；`sigV4Service`：oss→`oss`，**cos→`s3`**（默认）。列举走 `GET ?list-type=2` + XML 解析。
  - 分页：`maxFileLimit ≤ 1000` 时**只发一页**；每页 `min(maxFileLimit, 1000)`。
  - 下载并发 `Semaphore(downloadConcurrency ?? 16)`；getFile 重试 `maxAttempts ?? 3`，整请求超时 `totalTimeoutMs ?? 60_000`，读流空闲超时 `idleTimeoutMs ?? 10_000`，退避 `min(4000, 300*2^(n-1))` + 抖动。失败返回 `null`（不抛）。
  - 未接线的配置字段（可忽略）：`keepAlive/maxSockets/connectionTimeoutMs/socketTimeoutMs/requestTimeoutMs/retryMode`；`sessionToken` 也未用于签名。
- **generatePublicUrl 拼接规则**（S3/OSS/COS）：customDomain 优先（直拼 key**不 encode**）；否则 baseUrl + `encodeS3Key`（逐段 encodeURIComponent）。baseUrl：无 endpoint 时 virtual-host（AWS `{bucket}.s3.{region}.amazonaws.com`、OSS `{bucket}.{region}.aliyuncs.com`、COS `{bucket}.cos.{region}.myqcloud.com`）；有 endpoint 时支持 `{bucket}` 占位 / 阿里腾讯子域名注入 / 其它走 **path-style** `{endpoint}/{bucket}/`。
- 各 provider 配置字段见 `storage/interfaces.ts`（S3/OSS/COS 共享 `BaseS3LikeConfig`；B2/GitHub/Local/Eagle 各自）。

---

## 4. 增量 / 缓存 / 删除

两层筛选 + 字段级复用：
- **任务筛选** `filterTaskImages`（`builder.ts:712`）：force/force-manifest → 全量；否则 = 新图 ∪ `needsUpdate` ∪（缩略图缺失或 force-thumbnails）。
- `needsUpdate`（`manager.ts:47`）：无 existing → true；无 LastModified → true；否则 `s3.LastModified > new Date(item.lastModified)`。**纯比时间戳，不比 etag/size/digest。**
- **照片级** `shouldProcessPhoto`（`cache-manager.ts:20`）：force → 处理；新图 → 处理；`item.lastModified !== obj.LastModified.toISOString()` 或 force-manifest → 处理；缩略图缺失或 force-thumbnails → 处理；否则跳过复用 existing。
- **字段级复用**（非 force 时）：缩略图（有 thumbHash+文件存在）、EXIF（有 exif）、影调（有 toneAnalysis）、location（继承 existing）。
- **删除检测** `handleDeletedPhotos`（`manager.ts:86`）：扫 `public/thumbnails`，文件名（去 `.jpg`）不在当前 manifest 的 `id` 集合 → 删该缩略图；manifest 为空 → 清空整个目录。存储里仍存在但本轮未处理的旧项会被加回 manifest。
- daemon 友好点：增量状态**全部来自磁盘**（manifest 文件 + 缩略图目录存在性），无内存状态，重启自动恢复。

---

## 5. Manifest 版本与迁移
链式迁移 `manifest/migrations/`，当前 `v10`：
- **v1→v6**：视为无效，**丢弃全部数据**。
- **v6→v7**：`thumbnailUrl` 的 `.webp` → `.jpg`。
- **v7→v8**：扁平 Live/Motion 字段 → `video: VideoSource`。
- **v8→v9**：补 `format`。
- **v9→v10**：`exif.GPSAltitudeRef` 归一化为数字 0/1。

Rust lite 作为唯一生成者，可直接以 v10 结构产出；迁移链仅在「导入旧 manifest」时才需要，初期可不实现。

---

## 6. 插件系统
`PluginManager` 顺序分发生命周期事件。完整事件（按流程）：`onInit, beforeBuild, afterManifestLoad, afterAllFilesListed, afterLivePhotoDetection, afterImagesListed, afterTasksPrepared, beforeProcessTasks, beforePhotoProcess, afterPhotoProcess, photoProcessError, beforeAddManifestItem, afterProcessTasks, afterCleanup, beforeSaveManifest, afterSaveManifest, afterBuild, onError`。

内置插件：
- **storage:\<provider\>**：注册存储 provider（按 `storage.provider` 自动注入）。
- **geocoding**（默认禁用）：EXIF GPS → 反向地理编码（Mapbox / Nominatim / auto），写 `location`，按 `lat,lng`（精度 4）缓存。
- **github-repo-sync**（默认配置里禁用）：`beforeBuild` clone/pull 一个资产仓库并 symlink thumbnails/manifest；`afterBuild` 在 `hasUpdates` 时 commit/push。
- **thumbnail-storage**：把缩略图上传到远程存储并改写 `thumbnailUrl`。

Rust lite：这些都是「钩子式扩展」，核心管线不依赖。geocoding/远程缩略图可作为可选 feature，github-repo-sync 与 daemon 的"运行时更新"目标重叠/冲突，可不实现。

---

## 7. Serve / 注入层（Rust 服务器要复刻的）
权威实现：`be/apps/core/.../static-web/`。

### 7.1 index.html 占位点（`apps/web/index.html`）
- `<script id="manifest"></script>`（空）→ 填 `window.__MANIFEST__ = <整个 AfilmoryManifest JSON>;`。**唯一缺了必崩的注入。**
- `<script id="config">`（默认 `window.__CONFIG__={}` / `window.__SITE_CONFIG__={}`）→ 替换为 `window.__CONFIG__ = {...};window.__SITE_CONFIG__ = {...}`。
- `<title><%- title %></title>` / `<meta name="description" content="<%- description %>">` / splash 文案：server-serve 构建里 EJS 字面量**残留**，需运行时改 `<title>` 和 `meta[description]` 覆盖（否则页面标题显示 `<%- title %>` 字面量）。

### 7.2 注入内容
- `__MANIFEST__` = 整个 manifest（四键齐全）。standalone 直接读本地 `photos-manifest.json`。
- `__CONFIG__`：`{ useApi:bool, useCloud:bool }`（SPA 默认全 false）。**standalone 等价**：注 `{}` 或 `{useApi:false,useCloud:false}`。`useCloud:true` 才开评论/登录/社交等云端增强（缺了不崩，只少功能）。
- `__SITE_CONFIG__`：`Partial<SiteConfig>`（name/title/description/url/accentColor/author/social/map/beian…），来自 `config.json` + `site.config.ts` 合并。

### 7.3 静态 serve / fallback / 缓存
- server-serve 构建 base = `/static/web/`；产物在 `apps/web/dist/`（`index.html` + `assets/*`）。
- SPA history fallback：无扩展名路径 → `index.html`（200）；有扩展名找不到 → 404。
- 缓存头：HTML `no-cache`（**必须**，否则注入的 manifest 被缓存）；带扩展名静态资源 `public, max-age=31536000, immutable`。
- `/thumbnails/*` **不由 serve 端生成**——URL 写在 manifest `thumbnailUrl` 里；若是同源相对路径则走静态 serve（缩略图文件需与 SPA 一起部署），若是绝对 URL/CDN 则无需管。

### 7.4 OG / SEO（可选增强，SPA 不读、缺了不崩）
- photo page `/photos/:id` 与 homepage `/`：运行时删旧 `og:*`/`twitter:*`、改 `<title>`、注 `og:title/description/image/url`。纯字符串/DOM 操作，零依赖，**性价比最高**。
- `/og`、`/og/:id` 动态 PNG：satori → resvg，1200×628。重依赖，**最低优先级**，可不做。

### 7.5 构建 server-serve 壳的命令
```
AFILMORY_EMBED_MANIFEST=false BUILD_FOR_SERVER_SERVE=1 pnpm --filter @afilmory/web build
```
产出「空壳 SPA」（base=`/static/web/`、三注入点空/残留），正是 Rust serve 端要消费的产物。`BUILD_FOR_SERVER_SERVE=1` 的作用：改 base、单入口、排除一组构建期插件（manifest 内联 / site-config 注入 / PWA / OG / sitemap / html 模板）。

---

## 8. CLI / 配置 / 并发 / 环境变量
- **CLI 标志**：`--force`（全量）/`--force-manifest`（重生成 manifest，缩略图可复用）/`--force-thumbnails`（重生成缩略图）/`--config`（打印配置）/`--no-ui`/`--help`。无标志 = **增量更新**。
- **配置**：`builder.config.ts`（c12 加载，infra：storage + system.processing + system.observability + plugins）与 `config.json`+`site.config.ts`（presentation）分离。
  - `system.processing`：`defaultConcurrency=10`、`enableLivePhotoDetection=true`、`digestSuffixLength=0`、`limitInputPixels?`。
  - `system.observability.performance.worker`：`workerCount=cpus*2`、`useClusterMode=true`、`workerConcurrency=2`。
- **并发模型**：`useClusterMode && tasks ≥ concurrency*2` 用多进程 cluster（fork CLI 自身），否则进程内 async 线程池。
- **环境变量**（`env.ts`，zod 校验）：`S3_REGION/ACCESS_KEY_ID/SECRET_ACCESS_KEY/ENDPOINT/BUCKET_NAME/PREFIX/CUSTOM_DOMAIN/EXCLUDE_REGEX`、`PG_CONNECTION_STRING`、`GIT_TOKEN`；运行时还读 `EXIFTOOL_PATH`、`DEBUG`、cluster 相关 `CLUSTER_WORKER/WORKER_ID/...`。
- **运行时依赖**：构建前 `execSync('perl -v')`，**Perl 缺失 exit(1)**（exiftool-vendored 在 Linux 需系统 Perl）；另需 `git`（github-repo-sync）、写 `/tmp/image_process`（exif 临时文件）。

---

## 9. Rust 复刻：难点与策略

### 9.1 做不到字节级一致的项（已确认接受「结构/语义一致」）
| 产物 | 原因 | lite 策略 |
|---|---|---|
| `digest`（sha256） | 哈希的是 sharp/heic-convert 转码后字节 | 用 Rust 自己转码后的字节算 sha256，**自洽即可**（仅作增量比对/去重，SPA 不依赖跨实现一致） |
| 缩略图 JPEG 字节 | libvips→mozjpeg 编码器差异 | 视觉一致即可（image/libvips + jpeg 编码） |
| `thumbHash` 比特 | 依赖 resize 像素逐位一致 | thumbhash 算法用 Rust crate；像素近似即可（占位模糊图，肉眼无差） |
| `exif` 文本值 | exiftool 的 PrintConv 文本映射 | **走 `exiftool` CLI 子进程**（`-json`），保证显示值一致 |

### 9.2 推荐 crate / 方案
| 步骤 | 难度 | 方案 |
|---|---|---|
| SHA-256 / hex | 完全可对齐 | `sha2` / `hex` |
| thumbHash 算法 | 可对齐（需相同像素） | `thumbhash` crate |
| EXIF | 极难纯 Rust | **`exiftool` CLI 子进程 `-json`**（已定）；可选 `nom-exif` 抽基础字段做快路径 |
| HEIC 解码 | 像素可对齐 | `libheif-rs`（C 依赖 libheif） |
| resize/colorspace/rotate | 难逐位 | `image` + `fast_image_resize`(Lanczos3)，或 FFI `libvips` |
| JPEG 编码 | 字节不可对齐 | `mozjpeg` / `jpeg-encoder`（视觉一致） |
| BMP 解码 | 可对齐 | `image` |
| 直方图/影调 | 算法可对齐 | 纯 Rust 手写 |
| S3 | — | **手写 SigV4**（`reqwest`+`ring/hmac/sha2`），**不用 `rust-s3`/`object_store`**（URL/endpoint 行为对不上） |
| XML 列举解析 | — | `quick-xml` |
| 反向地理编码 | 可对齐 | `reqwest` |

### 9.3 一次性 CLI → 常驻 daemon 的改造要点
1. 去掉散布的 `process.exit`，build 失败抛错由调度层捕获，绝不拖垮进程。
2. 放弃 cluster「fork CLI 自身」模型，改用 **tokio 任务 / rayon 线程池**（结构一致不需要多进程）。
3. 放弃 TUI，走 `BuildProgressListener` 等价的进度上报（日志 / SSE）。
4. **并发触发要加互斥锁**：webhook + 轮询可能并发，绝不能并行写同一 manifest/thumbnails。建议「一次 build = 带锁的串行任务」，重复触发合并或排队。
5. 工作目录可配置（manifest + thumbnails 路径），与 serve 目录对齐。
6. exiftool 子进程在 daemon 下应**常驻复用**（stay-open），而非每次 build 重建。
7. 唯一构建入口语义 = `buildManifest({ isForceMode, isForceManifest, isForceThumbnails })`；轮询常规触发 = 三者全 false（增量）。

---

## 10. Rust lite 服务器最小职责清单
**必须**：① serve `dist`（`/static/web/*`）；② 每次返回 HTML 前注入 `__MANIFEST__`（必须）+ `__CONFIG__`/`__SITE_CONFIG__` + 覆盖 `<title>`/`description`；③ SPA history fallback；④ 缓存头（HTML no-cache、静态 immutable）；⑤ 缩略图可达。
**daemon**：⑥ webhook 端点 + 定时轮询 → 触发带锁的 `buildManifest`（增量）；⑦ 进度/状态可观测。
**可选增强**：OG/SEO meta 注入、动态 OG PNG、geocoding、远程缩略图存储、旧 manifest 迁移导入。
