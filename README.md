# afilmory-lite

一个用 Rust 写的、给 [**Afilmory**](https://github.com/Afilmory/Afilmory) 照片画廊用的**单二进制常驻服务**：
serve 预构建好的 Afilmory 前端，并在内部复刻它的 build 管线（从对象存储拉照片 → 处理 → 生成 `manifest.json` + 缩略图），
通过 **定时轮询 / webhook / S3 事件 / 手动端点** 触发**增量更新**，**不需要每次重新构建前端**。

> 上游项目：https://github.com/Afilmory/Afilmory

---

## 为什么有这个项目

[Afilmory](https://github.com/Afilmory/Afilmory) 是个很棒的自托管照片画廊，但它的更新方式比较重：**每次照片有变化，都要重新跑一整套 Node 构建**（拉 S3 → 生成 manifest/缩略图 → 重新打包前端）再部署。照片多、更新频繁时很麻烦。

afilmory-lite 把「**构建一次前端**」和「**持续更新数据**」解耦：

- 前端静态壳只构建一次（数据不再内嵌，改成运行时注入）；
- 一个常驻的 Rust 服务负责：从存储拉照片、处理、生成 manifest + 缩略图，并把数据注入到前端；
- 数据更新由 webhook / 轮询 / S3 事件 / 手动触发，**全程不碰前端构建**。

产物只有一个二进制（外加 `exiftool`、HEIC 还需 `libheif`），可直接跑，也可用我们打好的 Docker 镜像。

## 工作原理

```
afilmory-lite（单进程）
├─ server     serve 前端 dist + 运行时注入 __MANIFEST__/__SITE_CONFIG__ + SPA 路由回退 + 缓存头 + /thumbnails(+本地原图)
├─ scheduler  轮询 / webhook / S3 事件 / 手动 → 去重串行的构建协调器
├─ builder    列举存储 → 增量筛选 → 并发处理 → 写 manifest.json + thumbnails/
├─ storage    S3 / S3 兼容（手写 SigV4）、本地目录
├─ pipeline   解码(可选 HEIC) → 缩略图 → thumbHash → 影调 → HDR/Live/Motion Photo → 组装
└─ exif       exiftool 子进程（EXIF 保真）
```
原图不经过本服务（S3 模式下 `originalUrl` 直连桶/CDN；本地模式由本服务托管）。与上游 manifest 保持**结构/语义一致**（字段结构、缩略图视觉、EXIF 显示值一致），不追求字节级相同。

---

## 快速开始（Docker Compose，最简单）

镜像里已打包好程序 + 前端 + exiftool + libheif，开箱即用。

```bash
git clone https://github.com/kylinholmes/afilmory-lite
cd afilmory-lite

cp docker/afilmory.example.toml afilmory.toml   # 按需改 [storage] / [site] / [triggers]
mkdir -p data photos                            # data=持久化产物；photos=本地照片(用 S3 可不要)

docker compose up -d
docker compose logs -f
```
浏览器打开 `http://<host>:8080/`。镜像在 `ghcr.io/kylinholmes/afilmory-lite`（`:main` 为主干滚动版；打了 `v*` tag 后有 `:latest` / `:x.y.z`）。

> GHCR 包若是 private，先 `docker login ghcr.io`（用带 `read:packages` 的 PAT）。

---

## 配置

TOML，完整示例见 [`docker/afilmory.example.toml`](docker/afilmory.example.toml)。

| 段 | 字段 | 说明 |
|---|---|---|
| `[server]` | `listen` | 监听地址，默认 `0.0.0.0:8080` |
| | `workdir` | 存放 `manifest.json` + `thumbnails/`（需持久化） |
| | `dist_dir` | 前端静态壳目录（Docker 内置为 `/app/web/dist`） |
| | `admin_token` | 设了才启用 `/admin` 配置页 + 配置读写（Bearer 鉴权），运行时热重载 |
| `[site]` | `name`/`title`/`description`/`accentColor`/… | 注入 `window.__SITE_CONFIG__`（站点信息） |
| `[storage.local]` | `base_path` / `base_url` | 本地目录；`base_url` 为根路径（如 `/photos`）时由本服务托管原图 |
| `[storage.s3]` | `bucket`/`region`/`endpoint`/`access_key_id`/`secret_access_key`/`prefix`/`custom_domain`… | S3 / 兼容 AWS / MinIO / Cloudflare R2 / Wasabi |
| `[processing]` | `concurrency`/`thumbnail_width`(600)/`thumbnail_quality`(100)/`enable_live_photo` | 处理参数 |
| `[exif]` | `exiftool_path` | exiftool 可执行路径（默认 `exiftool`） |
| `[triggers]` | `poll_interval_secs` | >0 开启定时轮询 |
| | `webhook_token` | 设了才启用 `/api/hooks/build` 与 `/api/admin/build`（Bearer 鉴权） |
| | `enable_s3_event` | 启用 `/api/hooks/s3` |
| `[geocoding]` | `enabled`/`provider`/`mapbox_token`/`nominatim_base_url`/`language`/`cache_precision` | GPS→城市/国家 反查，写入 `location`；默认关。`provider=auto` 有 token 用 Mapbox 否则 Nominatim |

最小示例（本地照片）：
```toml
[server]
listen = "0.0.0.0:8080"
workdir = "./data"
dist_dir = "./web-dist"
[site]
title = "My Gallery"
[storage.local]
base_path = "./photos"
base_url = "/photos"
```

**在线改配置（/admin）**：`[server]` 配了 `admin_token` 后，浏览器打开 `http://host:8080/admin` 即可在线编辑配置并**热重载**——除 `listen`（端口，需重启）外其余即时生效。底层接口：`GET` / `PUT /api/admin/config`（Bearer admin token；含密钥故需鉴权）。

---

## 使用

CLI 两个子命令：
```bash
afilmory-lite build --config afilmory.toml          # 跑一次构建（增量；--force 全量）
afilmory-lite serve --config afilmory.toml          # 启动常驻服务（启动时也会跑一次增量）
```

更新数据（serve 模式）：
```bash
curl http://host:8080/api/status                                              # 查看构建状态
curl -X POST -H "Authorization: Bearer <token>" http://host:8080/api/hooks/build   # webhook 触发增量
curl -X POST -H "Authorization: Bearer <token>" \
     -H 'content-type: application/json' -d '{"force":true}' \
     http://host:8080/api/admin/build                                         # 手动触发（可全量）
```
日志默认 `info` 级（每张图打印分步耗时）；`RUST_LOG=debug` 更详细、`RUST_LOG=warn` 更安静。

---

## 部署

- **方式一：Docker Compose（推荐）** —— 见上面「快速开始」。更新镜像：`docker compose pull && docker compose up -d`。
- **方式二：Release tar 包** —— 从 [Releases](https://github.com/kylinholmes/afilmory-lite/releases) 下载 `afilmory-lite-<target>.tar.gz`，内含**程序 + `afilmory.example.toml` + `web/dist/`**。解压后把 `dist_dir` 指向解压出的 `web/dist`，改好配置即可 `./afilmory-lite serve --config afilmory.toml`（可自行配 systemd 常驻）。

产物：
- **Docker 镜像** → `ghcr.io/kylinholmes/afilmory-lite`（多架构 amd64/arm64）。
- **Tar 包** → 打 `v*` tag 时由 CI 生成并附到 GitHub Release。

---

## 状态

| 能力 | 状态 |
|---|---|
| Builder 核心（存储 → manifest + 缩略图） | ✅ |
| Server + 四类触发器（常驻 daemon） | ✅ |
| S3 / S3 兼容存储（手写 SigV4） | ✅ |
| 本地目录存储（含原图托管） | ✅ |
| HDR / Live Photo / Motion Photo | ✅ |
| HEIC（`heic` feature / libheif≥1.18） | ✅ amd64（CI/运行镜像经 strukturag PPA 启用）；arm64 暂不含 |

设计文档见 [`docs/`](docs/)（上游功能盘点 + 各阶段 spec/plan）。

---

## 与上游的关系 / 许可

- 前端静态壳在构建时从 **上游 Afilmory 仓库** 拉取并编译，本项目不修改其源码。
- 仓库内若有 `afilmory-main/`，是上游的本地只读拷贝（靠 `pull` 更新，不纳入版本控制）。
- Afilmory 的许可见其[上游仓库](https://github.com/Afilmory/Afilmory)。
