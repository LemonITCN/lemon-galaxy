#!/usr/bin/env bash
# 构建 LemonGalaxy Windows 主控端发布包（在 Windows 的 Git Bash 里运行，CI 与本机相同）。
#   lemon-galaxy-control-windows/scripts/build.sh [--skip-rust] [--selftest]
# 步骤：
#   1. 编 Rust：galaxy_ffi.dll（--features decoder，带 H.264 软解）与 lemon-galaxy-worker.exe（Windows 被控端）
#   2. 用编好的 galaxy_ffi.dll 生成 C# 绑定（scripts/gen-csharp-bindings.sh）
#   3. dotnet build（.NET Framework 4.8，x64）
#   4. 组装 dist/LemonGalaxy-<版本>-windows-x64/：LemonGalaxy.exe、galaxy_ffi.dll、lemon-galaxy-worker.exe、Resources\、LICENSE
#   5. 打包 dist/LemonGalaxy-<版本>-windows-x64.zip（压缩包里是同名文件夹）
#   --selftest  打包后用发布目录里的 exe 跑界面自测（模拟内核离屏渲染 PNG 到 lemon-galaxy-control-windows/selftest/）
#   --skip-rust 不编 Rust（target 里已有产物时加快迭代）
# 环境变量：
#   LEMONGALAXY_VERSION  版本（默认取仓库根 Cargo.toml 的 workspace 版本）
#   LEMONGALAXY_BUILD    构建号（默认 dev），写进程序集信息版本「版本+构建号」，关于页显示
#   TARGET               Rust 目标（默认 x86_64-pc-windows-msvc）
#   RUSTFLAGS            CI 设为 -C target-feature=+crt-static（静态链接 C 运行库，目标机器不用装 VC++ 运行库）
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WIN="$ROOT/lemon-galaxy-control-windows"
PROJ="$WIN/LemonGalaxy/LemonGalaxy.csproj"
SKIP_RUST=0
SELFTEST=0
for a in "$@"; do
  case "$a" in
    --skip-rust) SKIP_RUST=1 ;;
    --selftest) SELFTEST=1 ;;
    *) echo "未知参数：$a" >&2; exit 2 ;;
  esac
done
cd "$ROOT"
VERSION="${LEMONGALAXY_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
VERSION="${VERSION:-0.1.0}"
BUILD="${LEMONGALAXY_BUILD:-dev}"
TARGET="${TARGET:-x86_64-pc-windows-msvc}"
REL="$ROOT/target/$TARGET/release"
NAME="LemonGalaxy-$VERSION-windows-x64"
DIST="$WIN/dist/$NAME"
echo "LemonGalaxy Windows 主控端 $VERSION（构建 $BUILD，Rust 目标 $TARGET）"

# Windows 路径（PowerShell、dotnet 参数用）
winpath() { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else echo "$1"; fi; }

# 1. Rust 内核与被控端
if [ "$SKIP_RUST" = 0 ]; then
  rustup target list --installed | grep -q "^$TARGET$" || rustup target add "$TARGET"
  echo "编译 galaxy_ffi.dll …"
  cargo build --release --target "$TARGET" -p galaxy-ffi --features decoder
  echo "编译 lemon-galaxy-worker.exe …"
  cargo build --release --target "$TARGET" -p lemon-galaxy-worker-windows
fi
for f in galaxy_ffi.dll lemon-galaxy-worker.exe; do
  [ -f "$REL/$f" ] || { echo "缺少 $REL/$f" >&2; exit 1; }
done

# 2. C# 绑定（与 dll 同一份元数据）
LIB="$REL/galaxy_ffi.dll" bash "$ROOT/scripts/gen-csharp-bindings.sh"

# 3. 主控端
echo "dotnet build …"
dotnet build "$(winpath "$PROJ")" -c Release -nologo \
  "-p:Version=$VERSION" "-p:InformationalVersion=$VERSION+$BUILD" "-p:ContinuousIntegrationBuild=true"
OUT="$WIN/LemonGalaxy/bin/Release/net48"
[ -f "$OUT/LemonGalaxy.exe" ] || { echo "没有生成 LemonGalaxy.exe" >&2; exit 1; }

# 4. 组装发布目录
rm -rf "$DIST" "$WIN/dist/$NAME.zip"
mkdir -p "$DIST"
cp "$OUT/LemonGalaxy.exe" "$OUT/LemonGalaxy.exe.config" "$DIST/"
cp -R "$OUT/Resources" "$DIST/Resources"
cp "$REL/galaxy_ffi.dll" "$REL/lemon-galaxy-worker.exe" "$DIST/"
cp "$ROOT/LICENSE" "$DIST/LICENSE.txt"
ls -l "$DIST"

# 5. 压缩包（Git Bash 不一定带 zip，用 PowerShell）
powershell.exe -NoProfile -NonInteractive -Command \
  "Compress-Archive -Path '$(winpath "$DIST")' -DestinationPath '$(winpath "$WIN/dist/$NAME.zip")' -Force"
echo "已生成 $WIN/dist/$NAME.zip"
if command -v sha256sum >/dev/null 2>&1; then (cd "$WIN/dist" && sha256sum "$NAME.zip"); fi

# 6. 界面自测（模拟内核，不连网络）
if [ "$SELFTEST" = 1 ]; then
  rm -rf "$WIN/selftest"
  mkdir -p "$WIN/selftest"
  set +e
  "$DIST/LemonGalaxy.exe" --selftest "$(winpath "$WIN/selftest")"
  CODE=$?
  set -e
  cat "$WIN/selftest/report.txt" 2>/dev/null || true
  ls "$WIN/selftest" | head -20
  echo "自测共 $(ls "$WIN/selftest"/*.png 2>/dev/null | wc -l) 张截图，退出码 $CODE"
  exit $CODE
fi
