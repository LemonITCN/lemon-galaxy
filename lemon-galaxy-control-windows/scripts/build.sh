#!/usr/bin/env bash
# 构建 LemonGalaxy Windows 主控端发布包（在 Windows 的 Git Bash 里运行，CI 与本机相同）。
#   lemon-galaxy-control-windows/scripts/build.sh [--skip-rust] [--selftest]
# 步骤：
#   1. 编 Rust：galaxy_ffi.dll（--features decoder，带 H.264 软解）与 lemon-galaxy-worker.exe（Windows 被控端）
#   2. 用编好的 galaxy_ffi.dll 生成 C# 绑定（scripts/gen-csharp-bindings.sh）
#   3. dotnet build（.NET Framework 4.8，x64）
#   4. 组装 dist/LemonGalaxy-<版本>-windows-x64/：LemonGalaxy.exe、galaxy_ffi.dll、lemon-galaxy-worker.exe、Resources\、LICENSE
#   5. 打包 dist/LemonGalaxy-<版本>-windows-x64.zip（压缩包里是同名文件夹）
#   --selftest  打包后用发布目录里的 exe 跑界面自测（模拟内核离屏渲染 PNG 到 lemon-galaxy-control-windows/selftest/），
#               再跑在线更新端到端（真的辅助程序换文件、启动新程序报到、坏包换回，在 selftest/update-e2e/ 里，不碰本机更新目录）
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
echo "LemonGalaxy Windows 主控端 ${VERSION}（构建 ${BUILD}，Rust 目标 ${TARGET}）"

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
  [ "$CODE" = 0 ] || exit $CODE

  # 7. 在线更新端到端（真的进程、真的文件）：发布目录复制一份当「装好的旧程序」，再做两个更新包，
  #    用辅助程序（LemonGalaxy.exe --apply-update）装：换文件 → 启动新程序 → 新程序加载内核后报到（--update-probe，不开窗口）→ 确认。
  #    第二个包的内核库是坏的：新程序报告加载失败 → 换回旧文件 → 重新打开旧程序。更新目录用 LEMONGALAXY_UPDATES_DIR 指到这里，不碰本机真的。
  #    工作目录放在 dist/ 下（不进自测产物），日志复制到 selftest/update-e2e.log。
  E2E="$WIN/dist/update-e2e"
  rm -rf "$E2E"
  mkdir -p "$E2E/helper" "$E2E/updates"
  cp -R "$DIST" "$E2E/app"
  cp "$DIST/LemonGalaxy.exe" "$DIST/LemonGalaxy.exe.config" "$E2E/helper/"
  jpath() { winpath "$1" | sed 's/\\/\\\\/g'; }
  apply_update() { # $1 = 更新包目录，$2 = 令牌；返回辅助程序的退出码
    cat > "$E2E/plan.json" <<EOF
{"Pid":0,"PidStart":0,"OldExe":"$(jpath "$E2E/app/LemonGalaxy.exe")","OldVersion":"$VERSION","NewVersion":"$VERSION","Source":"$(jpath "$1")","Target":"$(jpath "$E2E/app")","Migrate":false,"Token":"$2","Sha256":"","ZipPath":"","ExtraArgs":["--update-probe"]}
EOF
    LEMONGALAXY_UPDATES_DIR="$(winpath "$E2E/updates")" "$E2E/helper/LemonGalaxy.exe" --apply-update "$(winpath "$E2E/plan.json")"
  }
  FAIL=0
  cp -R "$DIST" "$E2E/pkg-good"
  echo good > "$E2E/pkg-good/Resources/update-e2e.txt"
  set +e
  apply_update "$E2E/pkg-good" "e2egood$RANDOM"
  UCODE=$?
  set -e
  if [ "$UCODE" != 0 ] || [ ! -f "$E2E/app/Resources/update-e2e.txt" ] || [ ! -f "$E2E/app/.lemongalaxy-files" ] || ls -d "$E2E/app"/.lg-update-* >/dev/null 2>&1; then
    echo "在线更新端到端：正常更新没装好（退出码 $UCODE）" >&2
    FAIL=1
  fi
  cp -R "$DIST" "$E2E/pkg-bad"
  echo bad > "$E2E/pkg-bad/Resources/update-e2e-bad.txt"
  # 文件头像程序（过得了更新包核对）但加载不了的内核库
  { printf 'MZ'; head -c 4096 /dev/zero; } > "$E2E/pkg-bad/galaxy_ffi.dll"
  set +e
  apply_update "$E2E/pkg-bad" "e2ebad$RANDOM"
  UCODE=$?
  set -e
  if [ "$UCODE" != 1 ] || [ -f "$E2E/app/Resources/update-e2e-bad.txt" ] || ! cmp -s "$DIST/galaxy_ffi.dll" "$E2E/app/galaxy_ffi.dll" \
     || ! grep -q '"Code":"startup"' "$E2E/updates/result.json"; then
    echo "在线更新端到端：新版本打不开时没换回旧版本（退出码 $UCODE）" >&2
    FAIL=1
  fi
  # 换回后辅助程序重新打开了旧程序（也带 --update-probe，加载内核后自己退出）；等它写完日志
  sleep 3
  echo "---- 在线更新日志 ----"
  cat "$E2E/updates/update.log" 2>/dev/null || true
  cp "$E2E/updates/update.log" "$WIN/selftest/update-e2e.log" 2>/dev/null || true
  echo "在线更新端到端：$([ "$FAIL" = 0 ] && echo 通过 || echo 失败)"
  rm -rf "$E2E" 2>/dev/null || true
  exit $FAIL
fi
