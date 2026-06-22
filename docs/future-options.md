# 后续可选技术方向（调研留档）

> 这里记录「现在不做、但将来可能想做」的技术选项与调研结论，便于后续快速回顾、想起来。

---

## 1. 纯 Rust HEIC 解码（去掉 libheif 这个唯一的 C 依赖）

- **调研日期**：2026-06
- **现状**：HEIC 走 `heic` cargo feature → `libheif-rs` / `libheif-sys`（libheif C 库，内部依赖 libde265）。这是本项目**唯一的 C 依赖**，也是 arm64 暂不支持 HEIC、CI 要装 strukturag PPA 的根因。
- **结论**：生态里**已经出现纯 Rust 的 H.265 / HEIC 解码方案**，理论上可去掉 libheif：
  - **`rust_h265`**（crates.io）—— 完整的纯 Rust H.265/HEVC 解码器：Main / Main10（8-bit、10-bit 4:2:0），CTU 16/32/64、I/P/B slice、WPP、tiles、SAO、deblocking、scaling list 等；自述对 FFmpeg **byte-exact**（含 1080p Big Buck Bunny x265 各 preset）。
  - **`imazen/heic`**（GitHub）—— 纯 Rust HEIC/HEIF **图像**解码器，无 C/C++ 依赖，内置带 DPB 的 HEVC 解码器（直接面向 HEIC 图片场景，比裸 H.265 解码器更贴合）。
  - **`scuffle-h265`** —— 仅 H.265 头 / SPS 解析（不做完整解码），可作辅助。
- **难点拆解**：HEIC = HEIF 容器(ISOBMFF) + HEVC 编码数据。容器/元数据解析很容易；难点一直在 **HEVC 帧内解码**——而上面这些 crate 正是把这块补上了。
- **落地前需验证**：grid/tile 网格拼接、EXIF 旋转、色彩（ICC / nclx）、10-bit→8-bit 下采样、与 libheif 输出的**视觉一致性**；以及这些 crate 的成熟度 / 维护活跃度 / 许可证；HEVC 解码端专利风险（通常较低，但留意）。
- **取舍**：当前 libheif 路线（feature-gated，amd64 已启用、运行镜像已带 libheif1）**可用且稳定**。换纯 Rust 的收益是「**单二进制零 C 依赖** + arm64 也能开 HEIC + CI 不再依赖 PPA」。属可选优化，非紧急。
- **链接**：
  - https://crates.io/crates/rust_h265
  - https://github.com/imazen/heic
  - https://docs.rs/scuffle-h265
