#!/usr/bin/env bash
# 生成 Windows 主控端用的 C# 绑定：编出本机的内核动态库（读 uniffi 元数据），再用 tools/uniffi-bindgen-cs 生成。
# 生成结果与平台无关，macOS / Linux / Windows 上都能跑。
#   scripts/gen-csharp-bindings.sh                 默认输出到 Windows 主控端工程
#   LIB=<galaxy_ffi.dll> scripts/gen-csharp-bindings.sh   用已编好的库（CI 交叉编译时）
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="${OUT:-$ROOT/lemon-galaxy-control-windows/LemonGalaxy/Core/Generated}"
if [ -z "${LIB:-}" ]; then
  cargo build -p galaxy-ffi --lib --release --features decoder
  case "$(uname -s)" in
    Darwin) LIB="$ROOT/target/release/libgalaxy_ffi.dylib" ;;
    MINGW* | MSYS* | CYGWIN*) LIB="$ROOT/target/release/galaxy_ffi.dll" ;;
    *) LIB="$ROOT/target/release/libgalaxy_ffi.so" ;;
  esac
fi
cargo build --release --manifest-path tools/uniffi-bindgen-cs/Cargo.toml
GEN="$ROOT/tools/uniffi-bindgen-cs/target/release/lemon-galaxy-uniffi-bindgen-cs"
[ -x "$GEN" ] || GEN="$GEN.exe"
mkdir -p "$OUT"
"$GEN" --library "$LIB" --no-format --config "$ROOT/lemon-galaxy-common-rust/galaxy-ffi/uniffi.toml" -o "$OUT"
echo "已生成 $OUT/galaxy_ffi.cs"
