# Afilmory Lite — M2：S3 / 兼容存储（SigV4） 实现计划

> 由控制者内联实现，每步 `cargo test` 验证（subagent 无 Bash）。配合 `docs/afilmory-feature-inventory.md` §3、§6。

**Goal:** 实现手写 SigV4 的 S3（及 S3 兼容：MinIO / Cloudflare R2 / Wasabi 等，通过 endpoint）provider，让 daemon 能直接从真实桶拉照片；与上游 URL 拼接 / 列举 / 签名行为对齐。

**Architecture:** 新增 `storage/sigv4.rs`（纯 hmac/sha2 签名）+ `storage/s3.rs`（reqwest 异步请求 + quick-xml 列举 + 实现 `StorageProvider`）。`StorageConfig` 增加 `S3` 变体。`Builder::from_config` 按 provider 选实现。

**关键 parity 要求（盘点 §3/§6）：**
- **不用 aws-sdk/rust-s3/object_store**；手写 header-based SigV4 + reqwest。
- 列举 `GET ?list-type=2` → quick-xml 解析 `ListBucketResult`；分页：`max_file_limit ≤ 1000` 只发一页，每页 `min(limit,1000)`。
- `generate_public_url`：customDomain 优先（**直拼 key，不 encode**）；否则 baseUrl + `encode_s3_key`（逐段 encodeURIComponent，保留 `/`）。
- baseUrl：无 endpoint → AWS virtual-host `https://{bucket}.s3.{region}.amazonaws.com/`；有 endpoint → 含 `{bucket}` 占位则替换，否则 **path-style** `{endpoint}/{bucket}/`（覆盖 MinIO/R2）。region 默认 `us-east-1`。
- 下载并发 `Semaphore(download_concurrency ?? 16)`；get_file 重试 `max_attempts ?? 3`、总超时 60s；失败返回 `None`（不抛）。
- detect_live_photos：groupKey = `dirname + '/' + stem`，视频仅 `.mov/.mp4`，basename 大小写敏感。
- service name = `s3`（OSS/COS 的特例不在 M2 范围）。

**TLS/交叉编译注意：** reqwest 用 rustls（避免 openssl C 依赖）。Task 21 先验证 native + `x86_64-unknown-linux-musl` 能交叉编译该 TLS 后端；若默认 crypto 后端（aws-lc）交叉失败，切 ring。

**M2 测试策略：** 无法在沙箱连真实 S3 → 单测覆盖 parity 关键纯逻辑：SigV4 已知向量、URL 各分支、encode_s3_key、ListBucketResult XML 解析、分页阈值、live photo 配对。真实 get/list 由用户对自己桶验证。

---

### Task 21: 依赖 + S3 配置 + 交叉编译验证
- 加 `reqwest`(default-features=false, rustls-tls)、`quick-xml`(serialize)、`hmac`。
- `StorageConfig::S3 { bucket, region(默认us-east-1), endpoint?, access_key_id, secret_access_key, session_token?, prefix?, custom_domain?, exclude_regex?, max_file_limit?, download_concurrency(默认16) }`。
- 解析单测；`cargo build` native + `--target x86_64-unknown-linux-musl` 通过（验证 TLS 后端交叉干净）。

### Task 22: SigV4 签名器 `storage/sigv4.rs`
- `sign(method, url, region, service, access_key, secret_key, session_token, headers, payload_sha256_hex, amz_date_yyyymmddThhmmssZ) -> Authorization 头 + 需补的 headers`。
- canonical request / credential scope / string-to-sign / 签名密钥链（HMAC AWS4..→date→region→service→aws4_request）。
- **已知向量单测**：用 AWS SigV4 官方测试用例（get-vanilla 之类）核对最终 Authorization。

### Task 23: S3 URL 构建（`storage/s3.rs` 内）
- `encode_s3_key`（逐段 encodeURIComponent，保留 `/`）。
- `build_base_url(bucket, region, endpoint)`：覆盖无 endpoint / `{bucket}` 占位 / path-style。
- `generate_public_url`：custom_domain 优先（不 encode）否则 base_url + encode。各分支单测。

### Task 24: 列举 + XML 解析 + 分页
- `list_objects`：构造 `GET /?list-type=2&prefix=..&max-keys=..&continuation-token=..`，签名，发送，quick-xml 反序列化 `ListBucketResult { contents: [{key,size,last_modified,etag}], is_truncated, next_continuation_token }`。
- 分页规则照搬；`list_images` 按扩展名过滤 + exclude_regex；`list_all_files` 全量。
- 样例 XML 反序列化单测 + 分页阈值单测（纯函数化）。

### Task 25: get_file（重试/并发/超时）
- `Semaphore` 限流；reqwest GET 签名请求；`max_attempts` 重试 + 退避；总超时；非 2xx/错误 → 重试，耗尽 → `None`。

### Task 26: provider 装配 + live photo + 接线
- `impl StorageProvider for S3Provider`（list_images/list_all_files/get_file/generate_public_url）。
- `detect_live_photos`（移到 storage 公共函数或各自实现；与 local 共享分组逻辑）。
- `Builder::from_config` 增加 `StorageConfig::S3 => Arc::new(S3Provider::new(...)?)`。
- 全量 `cargo test` + clippy 干净；更新 `docker/afilmory.example.toml` 的 S3 段为可用示例。

---
## 后续（M3）
HEIC(libheif) 解码、Live Photo(已配对，补 video 字段)、Motion Photo(图内嵌视频)、HDR GainMap。
