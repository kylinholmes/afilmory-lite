#!/usr/bin/env bash
#
# 在容器内编译 Afilmory 前端「server-serve 壳」，把静态文件导出到本地目录。
# Rust 不在此编译（由 GitHub Action 跨平台构建）。
#
# 用法:
#   scripts/build-web.sh [-o 输出目录] [--ref Afilmory分支/标签] [--repo git地址]
# 默认: -o web-dist  --ref main
# 环境变量 OUT / AFILMORY_REF / AFILMORY_REPO 亦可覆盖。

set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${OUT:-web-dist}"
AFILMORY_REPO="${AFILMORY_REPO:-https://github.com/Afilmory/Afilmory.git}"
AFILMORY_REF="${AFILMORY_REF:-main}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    -o|--out) OUT="$2"; shift 2 ;;
    --ref) AFILMORY_REF="$2"; shift 2 ;;
    --repo) AFILMORY_REPO="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0 ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

echo ">> 在 Docker 内编译前端壳 (Afilmory ref=$AFILMORY_REF) → $OUT/"
rm -rf "$OUT"
DOCKER_BUILDKIT=1 docker build \
  -f Dockerfile.web \
  --target export \
  --build-arg "AFILMORY_REPO=$AFILMORY_REPO" \
  --build-arg "AFILMORY_REF=$AFILMORY_REF" \
  --output "type=local,dest=$OUT" \
  .

echo ">> 完成。SPA 静态壳在 $OUT/（index.html 占位 + assets/，base=/static/web/）"
echo "   运行服务时把配置里的 dist_dir 指向它："
echo "     dist_dir = \"$(pwd)/$OUT\""
