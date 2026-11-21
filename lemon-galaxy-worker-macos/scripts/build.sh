#!/bin/bash
# 构建 LemonGalaxyWorker.app：arm64 与 x86_64 分别编译后 lipo 合成通用二进制，组装 .app 并签名。
# 用法：scripts/build.sh [--debug] [--dmg]
#   产物：build/LemonGalaxyWorker.noindex/LemonGalaxyWorker.app；
#         --dmg 另出 build/dist.noindex/LemonGalaxyWorker-<版本>-macOS.dmg（自动更新按这个文件名识别 worker / macos）
#   .noindex 目录不被聚焦索引，系统不会把构建目录里的副本登记成第二个 LemonGalaxyWorker。本脚本不安装、不打开应用。
#   版本：环境变量 LEMONGALAXY_VERSION（默认 0.1.0）与 LEMONGALAXY_BUILD（默认 年月日时分）
#   签名（决定系统授权在重新编译后能否保留），与 Mac 主控端相同的选择顺序：
#     1. 环境变量 LEMONGALAXY_SIGN_IDENTITY（证书名或 SHA-1；"-" 表示临时签名）
#     2. 本机文件 scripts/.sign-identity（不进仓库，内容同上）
#     3. 钥匙串里第一张 Developer ID Application
#     4. 证书主题含 LemonIT 的 Apple Development
#     5. 都没有（例如 CI）才临时签名。临时签名的应用每次重新编译，屏幕录制等授权都会失效。
#   Rust 内核：Vendor/LemonGalaxyCore.xcframework 与 Sources/Generated/galaxy_ffi.swift 都在时自动接入并默认使用
#   （仓库根 scripts/build-ffi-apple.sh 生成）；没有时只编译模拟内核。
#   共享代码：Sources/Shared/ 下是指向 lemon-galaxy-control-macos 源码的符号链接，构建前不需要额外步骤。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG=release
DMG=0
for a in "$@"; do
  case "$a" in
    --debug) CONFIG=debug ;;
    --dmg) DMG=1 ;;
    *) echo "未知参数：$a" >&2; exit 2 ;;
  esac
done
NAME=LemonGalaxyWorker
VERSION="${LEMONGALAXY_VERSION:-0.1.0}"
BUILD="${LEMONGALAXY_BUILD:-$(date +%Y%m%d%H%M)}"
SCRATCH="$ROOT/build/swiftpm.noindex"
APPDIR="$ROOT/build/$NAME.noindex"
APP="$APPDIR/$NAME.app"
OUT="$ROOT/build/dist.noindex"
ICON="$ROOT/Packaging/AppIcon.icns"
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
cd "$ROOT"

if [ -d Vendor/LemonGalaxyCore.xcframework ] && [ -f Sources/Generated/galaxy_ffi.swift ] && [ "${LEMONGALAXY_NO_CORE:-}" != 1 ]; then
  echo "内核：Rust 真内核"
else
  echo "内核：只有模拟内核（缺 Vendor/LemonGalaxyCore.xcframework 或 Sources/Generated/galaxy_ffi.swift）"
fi

BINS=()
for ARCH in arm64 x86_64; do
  echo "编译 ${ARCH}（${CONFIG}）…"
  LOG="$SCRATCH/build-$ARCH.log"
  mkdir -p "$SCRATCH"
  if ! swift build -c "$CONFIG" --arch "$ARCH" --scratch-path "$SCRATCH" >"$LOG" 2>&1; then
    grep -E "error" "$LOG" | head -40 >&2
    echo "编译失败：${ARCH}（完整日志 ${LOG}）" >&2
    exit 1
  fi
  tail -1 "$LOG"
  # 产物目录随 SwiftPM 版本与构建系统不同而不同，问它要；两个架构共用输出目录，先各自拷出来
  BINPATH="$(swift build -c "$CONFIG" --arch "$ARCH" --scratch-path "$SCRATCH" --show-bin-path)"
  BIN="$SCRATCH/$NAME-$ARCH"
  cp "$BINPATH/$NAME" "$BIN"
  [ "$(lipo -archs "$BIN")" = "$ARCH" ] || { echo "产物架构不对：$(lipo -archs "$BIN")，应为 $ARCH" >&2; exit 1; }
  BINS+=("$BIN")
done

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create "${BINS[@]}" -output "$APP/Contents/MacOS/$NAME"
if [ "$CONFIG" = release ]; then xcrun strip -x "$APP/Contents/MacOS/$NAME"; fi
sed -e "s/__VERSION__/$VERSION/" -e "s/__BUILD__/$BUILD/" Packaging/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
cp -R Packaging/zh-Hans.lproj Packaging/en.lproj "$APP/Contents/Resources/"
cp "$ICON" "$APP/Contents/Resources/AppIcon.icns"
# 语言包：运行时按 Contents/Resources 查找（App/AppSupport.swift 的 AppResources）
cp -R Resources/Locales "$APP/Contents/Resources/"
# 无人值守（docs/spec.md 3.0.1）：后台服务的 launchd 配置放在 SMAppService 要求的位置；会话代理模板放在旁边（后台服务据此生成全局配置）
mkdir -p "$APP/Contents/Library/LaunchDaemons" "$APP/Contents/Library/LaunchAgents"
cp Packaging/LaunchDaemons/*.plist "$APP/Contents/Library/LaunchDaemons/"
cp Packaging/LaunchAgents/*.plist "$APP/Contents/Library/LaunchAgents/"
plutil -lint -s "$APP"/Contents/Library/LaunchDaemons/*.plist "$APP"/Contents/Library/LaunchAgents/*.plist
cp ../LICENSE "$APP/Contents/Resources/LICENSE.txt" 2>/dev/null || true

# 签名身份
IDENTITY="${LEMONGALAXY_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ] && [ -f "$ROOT/scripts/.sign-identity" ]; then IDENTITY="$(head -1 "$ROOT/scripts/.sign-identity" | tr -d '[:space:]')"; fi
if [ -z "$IDENTITY" ]; then
  IDS="$(security find-identity -v -p codesigning 2>/dev/null || true)"
  IDENTITY="$(printf '%s\n' "$IDS" | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Developer ID Application:.*/\1/p' | head -1)"
  if [ -z "$IDENTITY" ]; then
    DEV="$(printf '%s\n' "$IDS" | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "\(Apple Development:.*\)"$/\1 \2/p')"
    while read -r HASH CERTNAME; do
      [ -z "$HASH" ] && continue
      if security find-certificate -c "$CERTNAME" -p 2>/dev/null | openssl x509 -noout -subject 2>/dev/null | grep -q "LemonIT"; then IDENTITY="$HASH"; break; fi
    done <<EOF_IDS
$DEV
EOF_IDS
  fi
fi
[ -z "$IDENTITY" ] && IDENTITY="-"
# 只有 Developer ID 才要安全时间戳（公证需要）；开发证书与临时签名不联网取时间戳
TS=--timestamp=none
if [ "$IDENTITY" != "-" ]; then
  case "$(security find-identity -v -p codesigning 2>/dev/null | grep -F "$IDENTITY" || true)" in *"Developer ID Application"*) TS=--timestamp ;; esac
fi
echo "签名：$( [ "$IDENTITY" = "-" ] && echo 临时签名 || security find-identity -v -p codesigning | grep -F "$IDENTITY" | sed 's/.*"\(.*\)"/\1/' | head -1)"
# Hardened Runtime（--options runtime）下的例外权限：一个都不需要，Packaging/LemonGalaxyWorker.entitlements 是空的
# （AMFI 不认 plist 里的注释，说明写在这里）：
#   - 不开 App Sandbox：被控端要用 ScreenCaptureKit 截全屏、CGEvent 向其他应用注入键鼠、读其他应用的辅助功能控件树，
#     沙箱里做不到；这些只靠「屏幕录制」「辅助功能」两项授权，由用户本人在系统设置里打开。
#   - 不需要 allow-jit / allow-unsigned-executable-memory：Rust 内核静态链接，没有 JIT。
#   - 不需要 disable-library-validation：不加载第三方动态库。
#   - 不声明 keychain-access-groups：Developer ID 分发没有描述文件，声明了无法启动；没有它时钥匙串自动退回传统钥匙串。
codesign --force --options runtime $TS --entitlements Packaging/LemonGalaxyWorker.entitlements --sign "$IDENTITY" "$APP"
codesign --verify --strict "$APP"
"$ROOT/scripts/check-bundle.sh" "$APP"
echo "$APP"
lipo -archs "$APP/Contents/MacOS/$NAME"
du -sh "$APP" | cat

if [ "$DMG" = 1 ]; then
  mkdir -p "$OUT"
  DMG_PATH="$OUT/$NAME-$VERSION-macOS.dmg"
  STAGE="$APPDIR/dmg-stage"
  rm -rf "$STAGE" "$DMG_PATH" && mkdir -p "$STAGE"
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -volname "$NAME" -srcfolder "$STAGE" -fs HFS+ -format UDZO -imagekey zlib-level=9 "$DMG_PATH" >/dev/null
  rm -rf "$STAGE"
  if [ "$IDENTITY" != "-" ]; then codesign --force $TS --sign "$IDENTITY" "$DMG_PATH"; fi
  echo "$DMG_PATH"
fi
# 构建目录里的副本只给打包和自测用，不登记
"$LSREGISTER" -u "$APP" >/dev/null 2>&1 || true
