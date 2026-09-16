#!/bin/bash
# 构建三平台产物并上传到 Gitea Generic Package Registry
# Build all-platform artifacts and upload them to the Gitea generic package registry.
#
# Usage:
#   GITEA_USER=<user> GITEA_PASS=<pass> ./deploy/package-upload.sh [version]
#   （version 缺省取自 Cargo.toml 的 workspace version）
#
# 说明 / Notes:
#   - Gitea 地址从 git remote origin 自动提取，脚本不硬编码任何地址
#   - 同一代码库编译三平台（macOS Apple Silicon / Linux x86_64 / Linux aarch64），无需代码分支
#   - 覆盖上传同一 version+filename 会自动替换（Gitea generic 包特性）
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:-$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')}"
OWNER="OM"          # 组织/用户名，可用环境变量覆盖
OWNER="${ZBP_PKG_OWNER:-$OWNER}"

# 从 origin 提取 Gitea 基地址（支持 http(s) remote；ssh remote 请设 GITEA_URL 覆盖）
if [ -n "${GITEA_URL:-}" ]; then
  BASE="$GITEA_URL"
else
  R=$(git remote get-url origin)
  BASE=$(echo "$R" | sed -E 's#(https?://)[^@/]+@?#\1#; s#\.git$##; s#/OM/zbxpatrol$##')
fi
: "${GITEA_USER:?need GITEA_USER env}"
: "${GITEA_PASS:?need GITEA_PASS env}"

echo "==> 构建 artifacts (version=$VERSION)"
mkdir -p dist
cargo build --release                                        # macOS arm64（本机）
cp target/release/zbxpatrol "dist/zbxpatrol-${VERSION}-macos-arm64"
command -v cargo-zigbuild >/dev/null || cargo install cargo-zigbuild
cargo zigbuild --release --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/zbxpatrol "dist/zbxpatrol-${VERSION}-linux-x86_64"
cargo zigbuild --release --target aarch64-unknown-linux-musl
cp target/aarch64-unknown-linux-musl/release/zbxpatrol "dist/zbxpatrol-${VERSION}-linux-aarch64"

echo "==> 上传到 $BASE/api/packages/$OWNER/generic/zbxpatrol/$VERSION"
for f in "dist/zbxpatrol-${VERSION}-macos-arm64" \
         "dist/zbxpatrol-${VERSION}-linux-x86_64" \
         "dist/zbxpatrol-${VERSION}-linux-aarch64"; do
  curl -sf -u "$GITEA_USER:$GITEA_PASS" \
    --upload-file "$f" \
    "$BASE/api/packages/$OWNER/generic/zbxpatrol/$VERSION/$(basename "$f")" \
    && echo "  uploaded: $(basename "$f") ($(wc -c <"$f" | tr -d ' ') bytes)"
done

echo "==> 完成。下载地址形如："
echo "  $BASE/api/packages/$OWNER/generic/zbxpatrol/$VERSION/zbxpatrol-${VERSION}-linux-x86_64"
