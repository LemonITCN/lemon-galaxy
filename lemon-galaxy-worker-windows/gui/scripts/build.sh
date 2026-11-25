#!/usr/bin/env bash
# 构建 Windows 被控端带界面的发布包（在 Windows 的 Git Bash 里运行，CI 与本机相同）。
#   lemon-galaxy-worker-windows/gui/scripts/build.sh [--worker 路径] [--arch x64|arm64] [--skip-build] [--selftest] [--live]
# 步骤：
#   1. dotnet build（.NET Framework 4.8，AnyCPU，x64 与 ARM64 共用同一个界面程序）
#   2. 组装 dist/LemonGalaxyWorker-<版本>-windows-<架构>/：LemonGalaxyWorker.exe、Resources\、lemon-galaxy-worker.exe（被控服务）、LICENSE.txt
#   3. 打包 dist/LemonGalaxyWorker-<版本>-windows-<架构>.zip（压缩包里是同名文件夹）
#   --worker      被控服务程序（默认 target/<Rust 目标>/release/lemon-galaxy-worker.exe）
#   --skip-build  不重新编界面（打第二个架构的包时用）
#   --selftest    界面自测：协议、离屏渲染各页面到 lemon-galaxy-worker-windows/gui/selftest/<架构>/、语言包
#   --live        自测时再起一个真的被控服务（前台 run，临时数据目录与临时管道），界面连上去走一遍
# 环境变量：LEMONGALAXY_VERSION（默认取仓库根 Cargo.toml 的版本）、LEMONGALAXY_BUILD（构建号，默认 dev）
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
GUI="$ROOT/lemon-galaxy-worker-windows/gui"
ARCH=x64
WORKER=""
SKIP_BUILD=0
SELFTEST=0
LIVE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --worker) WORKER="$2"; shift ;;
    --arch) ARCH="$2"; shift ;;
    --skip-build) SKIP_BUILD=1 ;;
    --selftest) SELFTEST=1 ;;
    --live) LIVE=1 ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
  shift
done
cd "$ROOT"
VERSION="${LEMONGALAXY_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
BUILD="${LEMONGALAXY_BUILD:-dev}"
case "$ARCH" in x64) T=x86_64 ;; arm64) T=aarch64 ;; *) echo "架构只能是 x64 或 arm64" >&2; exit 2 ;; esac
WORKER="${WORKER:-$ROOT/target/$T-pc-windows-msvc/release/lemon-galaxy-worker.exe}"
[ -f "$WORKER" ] || { echo "缺少被控服务程序 $WORKER" >&2; exit 1; }
NAME="LemonGalaxyWorker-$VERSION-windows-$ARCH"
DIST="$GUI/dist/$NAME"
winpath() { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else echo "$1"; fi; }
echo "LemonGalaxy Windows 被控端（带界面）${VERSION}（构建 ${BUILD}，${ARCH}）"

if [ "$SKIP_BUILD" = 0 ]; then
  dotnet build "$(winpath "$GUI/LemonGalaxyWorker.csproj")" -c Release -nologo \
    "-p:Version=$VERSION" "-p:InformationalVersion=$VERSION+$BUILD" "-p:ContinuousIntegrationBuild=true"
fi
OUT="$GUI/bin/Release/net48"
[ -f "$OUT/LemonGalaxyWorker.exe" ] || { echo "没有生成 LemonGalaxyWorker.exe" >&2; exit 1; }

rm -rf "$DIST" "$GUI/dist/$NAME.zip"
mkdir -p "$DIST"
cp "$OUT/LemonGalaxyWorker.exe" "$OUT/LemonGalaxyWorker.exe.config" "$DIST/"
cp -R "$OUT/Resources" "$DIST/Resources"
cp "$WORKER" "$DIST/lemon-galaxy-worker.exe"
cp "$ROOT/LICENSE" "$DIST/LICENSE.txt"
ls -l "$DIST"
powershell.exe -NoProfile -NonInteractive -Command \
  "Compress-Archive -Path '$(winpath "$DIST")' -DestinationPath '$(winpath "$GUI/dist/$NAME.zip")' -Force"
echo "已生成 $GUI/dist/$NAME.zip"
if command -v sha256sum >/dev/null 2>&1; then (cd "$GUI/dist" && sha256sum "$NAME.zip"); fi

[ "$SELFTEST" = 1 ] || exit 0
SDIR="$GUI/selftest/$ARCH"
rm -rf "$SDIR"
mkdir -p "$SDIR"
LIVE_ARGS=()
WPID=""
if [ "$LIVE" = 1 ]; then
  # 前台跑一个真的被控服务：临时数据目录、临时管道，只开本机控制接口（没有登录不连服务器）
  DATA="$(mktemp -d)"
  export LEMON_GALAXY_CONTROL_SOCKET="\\\\.\\pipe\\LemonGalaxyWorker.ci.$$"
  "$DIST/lemon-galaxy-worker.exe" --data-dir "$(winpath "$DATA")" run > "$SDIR/worker.log" 2>&1 &
  WPID=$!
  sleep 3
  LIVE_ARGS=(--live)
fi
set +e
"$DIST/LemonGalaxyWorker.exe" --selftest "$(winpath "$SDIR")" "${LIVE_ARGS[@]}"
CODE=$?
set -e
if [ -n "$WPID" ]; then
  # 只结束自己起的那个（不能按程序名杀：本机装着的被控服务也叫这个名字）
  WINPID="$(cat "/proc/$WPID/winpid" 2>/dev/null || true)"
  [ -n "$WINPID" ] && taskkill //F //T //PID "$WINPID" >/dev/null 2>&1 || true
  kill "$WPID" 2>/dev/null || true
  echo "--- 被控服务日志"
  tail -20 "$SDIR/worker.log" || true
fi
cat "$SDIR/report.txt" 2>/dev/null || true
echo "自测共 $(ls "$SDIR"/*.png 2>/dev/null | wc -l) 张截图，退出码 $CODE"
exit $CODE
