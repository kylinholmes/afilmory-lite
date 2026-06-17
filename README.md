# afilmory-lite

用 Rust 写的 [Afilmory](https://github.com/Afilmory/Afilmory) 「lite」版：一个**单二进制常驻服务**，
serve 预构建好的 Afilmory 前端壳，并在内部复刻其 build 管线（从存储拉照片 → 处理 → 生成 manifest + 缩略图）。
通过 **webhook / 定时轮询 / S3 事件 / 手动端点** 触发增量更新，**无需每次重新 build 前端**。

## 它解决什么

上游 Afilmory 每次照片变化都要重跑整套 Node 构建。afilmory-lite 把「构建一次前端壳」与「持续更新数据」解耦：
前端壳构建一次，数据由这个 Rust 服务在运行时注入并按需增量更新。

## 架构

```
afilmory-lite (单二进制)
├─ server     serve dist/ + 运行时注入 __MANIFEST__/__CONFIG__/__SITE_CONFIG__ + SPA fallback + 缓存头 + /thumbnails
├─ scheduler  轮询 / webhook / S3 事件 / 手动 → coalescing 串行构建协调器
├─ builder    列举 → 增量筛选 → 并发处理 → 写 manifest.json + thumbnails/
├─ storage    StorageProvider：S3 / S3 兼容（手写 SigV4）、本地目录
├─ pipeline   解码(+HEIC) → 缩略图 → thumbHash → 影调 → HDR/Live/Motion → 组装
└─ exif       exiftool 子进程（EXIF 保真）
```

一致性目标：与上游 manifest **结构/语义一致**（字段结构、缩略图视觉、EXIF 显示值一致），不追求字节级相同。

## 构建与测试

```bash
cargo build --release          # 二进制（默认不含 HEIC）
cargo build --release --features heic   # 含 HEIC（需系统 libheif：apt install libheif-dev）
cargo test                     # 单元 + 集成测试
```

系统依赖：`exiftool`（EXIF 保真）；HEIC 还需 `libheif`。

## 运行

```bash
# 1) 构建前端壳（容器内编译 Afilmory 前端，导出静态文件）
scripts/build-web.sh --ref main -o web-dist

# 2) 写配置（见 docker/afilmory.example.toml），dist_dir 指向上一步产物
# 3) 构建一次 / 启动服务
cargo run --release -- build  --config afilmory.toml          # 增量构建（--force 全量）
cargo run --release -- serve  --config afilmory.toml          # 常驻服务
```

触发更新（serve 模式，配置 `[triggers].webhook_token` 后）：

```bash
curl -X POST -H "Authorization: Bearer <token>" http://host:8080/api/hooks/build   # webhook
curl http://host:8080/api/status                                                    # 构建状态
```

## 配置

TOML，见 [`docker/afilmory.example.toml`](docker/afilmory.example.toml)。要点：
- `[storage]` `provider = "local" | "s3"`（S3 兼容 AWS / MinIO / Cloudflare R2 / Wasabi，通过 `endpoint`）。
- `[server]` `listen` / `workdir`（manifest + thumbnails）/ `dist_dir`（前端壳）。
- `[triggers]` `poll_interval_secs` / `webhook_token` / `enable_s3_event`。

## Docker / CI

- **Docker 只编译前端**：`Dockerfile.web`（导出 dist）。
- **Rust 由 GitHub Action 跨平台编译**：`.github/workflows/release.yml`（linux x86_64/aarch64、macOS、Windows）。
- **运行时镜像** `Dockerfile.runtime` 只组装：预编译二进制 + dist + exiftool/libheif（多架构 → GHCR）。

## 状态

| 模块 | 状态 |
|---|---|
| Builder 核心（本地 → manifest + 缩略图） | ✅ |
| Server + 四类触发器（常驻 daemon） | ✅ |
| S3 / S3 兼容存储（手写 SigV4） | ✅（网络对真实桶验证） |
| HDR / Live Photo / Motion Photo | ✅ |
| HEIC（libheif，`heic` feature） | ✅ amd64：CI 与运行镜像经 strukturag PPA 取 libheif≥1.18 启用；arm64 暂不含（交叉编译无 arm64 PPA 库） |

设计文档见 [`docs/`](docs/)（功能盘点 + 各阶段 spec/plan）。

## 与上游的关系

`afilmory-main/`（如存在）是上游全量拷贝，仅作只读参考，靠 `pull` 更新，不纳入版本控制。
前端壳在构建时从上游仓库拉取并编译。
