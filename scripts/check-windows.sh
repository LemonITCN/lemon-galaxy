#!/usr/bin/env bash
# 本机交叉编译 / 检查 Windows 代码（llvm-mingw，gnullvm 目标）。正式包在 CI 上用 MSVC 编。
#   scripts/check-windows.sh -p galaxy-host                      检查
#   CMD=build TARGET=aarch64-pc-windows-gnullvm scripts/check-windows.sh -p lemon-galaxy-worker-windows
#   TARGET=i686-pc-windows-gnullvm scripts/check-windows.sh -p lemon-galaxy-worker-windows   32 位
set -euo pipefail
LLVM_MINGW="${LLVM_MINGW:-$(ls -d "$HOME"/.cache/lemon-galaxy/build-tools/llvm-mingw-*/ 2>/dev/null | tail -1)}"
[ -d "$LLVM_MINGW/bin" ] || { echo "找不到 llvm-mingw，请下载 https://github.com/mstorsjo/llvm-mingw/releases 解压到 ~/.cache/lemon-galaxy/build-tools/" >&2; exit 1; }
export PATH="$LLVM_MINGW/bin:$PATH"
for a in x86_64 aarch64 i686; do
  T="$a-pc-windows-gnullvm"
  U="$(echo "$T" | tr 'a-z-' 'A-Z_')"
  L="$(echo "$T" | tr '-' '_')"
  export "CARGO_TARGET_${U}_LINKER=$a-w64-mingw32-clang"
  export "CC_$L=$a-w64-mingw32-clang" "CXX_$L=$a-w64-mingw32-clang++" "AR_$L=llvm-ar"
done
TARGET="${TARGET:-x86_64-pc-windows-gnullvm}"
rustup target list --installed | grep -q "^$TARGET$" || rustup target add "$TARGET"
exec cargo "${CMD:-check}" --target "$TARGET" "$@"
