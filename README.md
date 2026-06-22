# afilmory-lite

afilmory-lite 是 [Afilmory](https://github.com/Afilmory/Afilmory) 照片画廊的单二进制常驻服务，以 Rust 实现。它托管预构建的 Afilmory 前端，并在进程内复刻其构建管线：从对象存储拉取照片，处理后生成 `manifest.json` 与缩略图。数据更新通过定时轮询、webhook、S3 事件或手动端点触发，以增量方式进行，无需重新构建前端。

## 背景

Afilmory 是自托管照片画廊。其默认更新流程在每次照片变更时需重新执行完整的 Node 构建（拉取 S3、生成 manifest 与缩略图、重新打包前端）并重新部署；在照片数量大、更新频繁的场景下成本较高。

afilmory-lite 将「前端构建」与「数据更新」解耦：

- 前端静态壳仅构建一次，数据改为运行时注入，不再内嵌；
- 常驻 Rust 服务负责从存储拉取照片、处理、生成 manifest 与缩略图，并注入前端；
- 数据更新由 webhook、轮询、S3 事件或手动端点触发，全程不涉及前端构建。

部署产物为单个二进制，运行依赖 `exiftool`（HEIC 另需 `libheif`）；同时提供预构建 Docker 镜像。

## 架构

```
afilmory-lite（单进程）
├─ server     serve 前端 dist + 运行时注入 __MANIFEST__/__SITE_CONFIG__ + SPA 路由回退 + 缓存头 + /thumbnails(+本地原图)
├─ scheduler  轮询 / webhook / S3 事件 / 手动 → 去重串行的构建协调器
├─ builder    列举存储 → 增量筛选 → 并发处理 → 写 manifest.json + thumbnails/
├─ storage    S3 / S3 兼容（手写 SigV4）、本地目录
├─ pipeline   解码(可选 HEIC) → 缩略图 → thumbHash → 影调 → HDR/Live/Motion Photo → 组装
└─ exif       exiftool 子进程（EXIF 保真）
```

原图不经过本服务：S3 模式下 `originalUrl` 直连存储桶或 CDN，本地模式由本服务托管。生成的 manifest 与上游保持结构与语义一致（字段结构、缩略图视觉、EXIF 显示值一致），不追求字节级相同。

## 快速开始（Docker Compose）

镜像包含程序、前端、`exiftool` 与 `libheif`。

```bash
git clone https://github.com/kylinholmes/afilmory-lite
cd afilmory-lite

cp docker/afilmory.example.toml afilmory.toml   # 按需修改 [storage] / [site] / [triggers], 也可以进入网页进行配置，建议更新配置后 关闭配置入口
mkdir -p data photos                            # data=持久化产物；photos=本地照片（使用 S3 时可省略）

docker compose up -d
docker compose logs -f
```

服务启动后通过 `http://<host>:8080/` 访问。镜像发布于 `ghcr.io/kylinholmes/afilmory-lite`：`:main` 为主干滚动构建，打 `v*` tag 后提供 `:latest` 与 `:x.y.z`。

## 配置

配置为 TOML 格式，完整示例见 [`docker/afilmory.example.toml`](docker/afilmory.example.toml)。

| 段 | 字段 | 说明 |
|---|---|---|
| `[server]` | `listen` | 监听地址，默认 `0.0.0.0:8080` |
| | `workdir` | 存放 `manifest.json` 与 `thumbnails/`（需持久化） |
| | `dist_dir` | 前端静态壳目录（Docker 内置为 `/app/web/dist`） |
| | `admin_token` | 设置后启用 `/admin` 配置页与配置读写接口（Bearer 鉴权），运行时热重载 |
| `[site]` | `name` / `title` / `description` / `accentColor` / … | 注入 `window.__SITE_CONFIG__`（站点信息） |
| `[storage.local]` | `base_path` / `base_url` | 本地目录；`base_url` 为根路径（如 `/photos`）时由本服务托管原图 |
| `[storage.s3]` | `bucket` / `region` / `endpoint` / `access_key_id` / `secret_access_key` / `prefix` / `custom_domain` … | S3 及兼容服务（AWS / MinIO / Cloudflare R2 / Wasabi）|
| `[processing]` | `concurrency` / `thumbnail_width`(600) / `thumbnail_quality`(100) / `enable_live_photo` | 处理参数 |
| `[exif]` | `exiftool_path` | exiftool 可执行路径（默认 `exiftool`） |
| `[triggers]` | `poll_interval_secs` | 大于 0 时启用定时轮询 |
| | `webhook_token` | 设置后启用 `/api/hooks/build` 与 `/api/admin/build`（Bearer 鉴权） |
| | `enable_s3_event` | 启用 `/api/hooks/s3` |
| `[geocoding]` | `enabled` / `provider` / `mapbox_token` / `nominatim_base_url` / `language` / `cache_precision` | GPS 反查城市与国家，写入 `location`；默认关闭。`provider=auto` 时有 token 用 Mapbox，否则用 Nominatim |

最小示例（本地照片）：

```toml
[server]
listen = "0.0.0.0:8080"
workdir = "./data"
dist_dir = "./web/dist"

# admin_token = "adm123" # Optional

[site]
title = "My Gallery"

[storage.local]
base_path = "./photos"
base_url = "/photos"
```

在线配置（`/admin`）：设置 `[server].admin_token` 后，通过 `http://<host>:8080/admin` 在线编辑配置并热重载——除 `listen`（需重启）外即时生效。底层接口为 `GET` / `PUT /api/admin/config`（Bearer admin token 鉴权，因配置含密钥）。

## 使用

CLI 提供两个子命令：

```bash
afilmory-lite build --config afilmory.toml          # 执行一次构建（增量；--force 全量）
afilmory-lite serve --config afilmory.toml          # 启动常驻服务（启动时执行一次增量构建）
```


## 部署

- **Docker Compose（推荐）**：见「快速开始」。更新镜像：`docker compose pull && docker compose up -d`。
- **Release tar 包**：从 [Releases](https://github.com/kylinholmes/afilmory-lite/releases) 下载 `afilmory-lite-<target>.tar.gz`，内含程序、`afilmory.example.toml` 与 `web/dist/`。解压后将 `dist_dir` 指向 `web/dist`，完成配置后执行 `./afilmory-lite serve --config afilmory.toml`（可结合 systemd 常驻）。

发布产物：

- Docker 镜像：`ghcr.io/kylinholmes/afilmory-lite`（amd64 / arm64 多架构）。
- Tar 包：打 `v*` tag 时由 CI 生成并附加至 GitHub Release。

## 功能状态

| 能力 | 状态 |
|---|---|
| Builder 核心（存储 → manifest + 缩略图） | ✅ |
| Server 与四类触发器（常驻 daemon） | ✅ |
| S3 及 S3 兼容存储（手写 SigV4） | ✅ |
| 本地目录存储（含原图托管） | ✅ |
| HDR / Live Photo / Motion Photo | ✅ |
| HEIC（`heic` feature / libheif ≥ 1.18） | amd64 ✅（CI 与运行镜像经 strukturag PPA 启用）；arm64 暂不支持 |

设计文档见 [`docs/`](docs/)（上游功能盘点与各阶段 spec/plan）。

## 上游与许可

- 前端静态壳在构建时从上游 Afilmory 仓库拉取并编译，本项目不修改其源码。
- 仓库内的 `afilmory-main/`（若存在）为上游只读拷贝，经 `pull` 更新，不纳入版本控制。
- Afilmory 的许可见其[上游仓库](https://github.com/Afilmory/Afilmory)。
