#!/bin/bash
# 构建 LemonGalaxy.app：arm64 与 x86_64 分别编译后 lipo 合成通用二进制，组装 .app 并签名。
# 用法：scripts/build.sh [--debug] [--dmg]
#   产物：build/LemonGalaxy.noindex/LemonGalaxy.app；--dmg 另出 build/dist.noindex/LemonGalaxy-<版本>.dmg
#   .noindex 目录不被聚焦索引，系统不会把构建目录里的副本登记成第二个 LemonGalaxy。本脚本不安装、不打开应用。
#   版本：环境变量 LEMONGALAXY_VERSION（默认 0.1.0）与 LEMONGALAXY_BUILD（默认 年月日时分）
#   签名（决定系统授权在重新编译后能否保留）：
#     1. 环境变量 LEMONGALAXY_SIGN_IDENTITY（证书名或 SHA-1；"-" 表示临时签名）
#     2. 本机文件 scripts/.sign-identity（不进仓库，内容同上）
#     3. 钥匙串里第一张 Developer ID Application
#     4. 证书主题含 LemonIT 的 Apple Development
#     5. 都没有才临时签名。临时签名的应用每次重新编译，屏幕录制等授权都会失效。
#   CI 发布包用 Secrets 里的 LemonGalaxy 专用自签名证书（系统不信任它，但签名、校验、加固运行时都正常；
#   系统授权认的「指定要求」= 标识 + 证书，每个版本都一样，所以升级后授权保留）。
#   Rust 内核：Vendor/LemonGalaxyCore.xcframework 与 Core/Generated/galaxy_ffi.swift 都在时自动接入并默认使用
#   （仓库根 scripts/build-ffi-apple.sh 生成）；没有时只编译模拟内核。见 Sources/LemonGalaxy/Core/Bridge/README.md。
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
VERSION="${LEMONGALAXY_VERSION:-0.1.0}"
BUILD="${LEMONGALAXY_BUILD:-$(date +%Y%m%d%H%M)}"
SCRATCH="$ROOT/build/swiftpm.noindex"
APPDIR="$ROOT/build/LemonGalaxy.noindex"
APP="$APPDIR/LemonGalaxy.app"
OUT="$ROOT/build/dist.noindex"
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
cd "$ROOT"

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
  BIN="$SCRATCH/LemonGalaxy-$ARCH"
  cp "$BINPATH/LemonGalaxy" "$BIN"
  [ "$(lipo -archs "$BIN")" = "$ARCH" ] || { echo "产物架构不对：$(lipo -archs "$BIN")，应为 $ARCH" >&2; exit 1; }
  RESDIR="$BINPATH"
  BINS+=("$BIN")
done

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create "${BINS[@]}" -output "$APP/Contents/MacOS/LemonGalaxy"
if [ "$CONFIG" = release ]; then xcrun strip -x "$APP/Contents/MacOS/LemonGalaxy"; fi
sed -e "s/__VERSION__/$VERSION/" -e "s/__BUILD__/$BUILD/" Packaging/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
cp -R Packaging/zh-Hans.lproj Packaging/en.lproj "$APP/Contents/Resources/"
cp Packaging/AppIcon.icns "$APP/Contents/Resources/"
# 语言包、终端配色、开源许可：运行时按 Contents/Resources 查找（App/Resources.swift）
cp -R Resources/Locales Resources/TerminalThemes Resources/Licenses "$APP/Contents/Resources/"
# 依赖如果带 SwiftPM 资源包，一并放进 Resources（SwiftTerm 1.2.x 没有）
for b in "$RESDIR"/*.bundle; do
  [ -e "$b" ] && cp -R "$b" "$APP/Contents/Resources/"
done

# 签名身份
IDENTITY="${LEMONGALAXY_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ] && [ -f "$ROOT/scripts/.sign-identity" ]; then IDENTITY="$(head -1 "$ROOT/scripts/.sign-identity" | tr -d '[:space:]')"; fi
if [ -z "$IDENTITY" ]; then
  IDS="$(security find-identity -v -p codesigning 2>/dev/null || true)"
  IDENTITY="$(printf '%s\n' "$IDS" | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Developer ID Application:.*/\1/p' | head -1)"
  if [ -z "$IDENTITY" ]; then
    DEV="$(printf '%s\n' "$IDS" | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "\(Apple Development:.*\)"$/\1 \2/p')"
    while read -r HASH NAME; do
      [ -z "$HASH" ] && continue
      if security find-certificate -c "$NAME" -p 2>/dev/null | openssl x509 -noout -subject 2>/dev/null | grep -q "LemonIT"; then IDENTITY="$HASH"; break; fi
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
# 显示名称不加 -v：自签名证书不受系统信任，-v 只列受信任的身份
echo "签名：$( [ "$IDENTITY" = "-" ] && echo 临时签名 || { security find-identity -p codesigning | grep -F "$IDENTITY" | sed 's/.*"\(.*\)".*/\1/' | head -1; } )"
# Hardened Runtime（--options runtime）下的例外权限：目前一个都不需要，Packaging/LemonGalaxy.entitlements 是空的
# （AMFI 不认 plist 里的注释，说明写在这里）：
#   - 不开 App Sandbox：被控端要用 ScreenCaptureKit 截全屏、CGEvent 向其他应用注入键鼠、读其他应用的辅助功能控件树，
#     沙箱里做不到；这些只靠「屏幕录制」「辅助功能」两项授权，由用户本人在系统设置里打开。
#   - 不需要 allow-jit / allow-unsigned-executable-memory：Rust 内核静态链接，没有 JIT。
#   - 不需要 disable-library-validation：不加载第三方动态库。
#   - 不声明 keychain-access-groups：Developer ID 分发没有描述文件，声明了无法启动；没有它时钥匙串自动退回传统钥匙串。
#   - 不需要 com.apple.security.device.audio-input：远端声音用 ScreenCaptureKit 采系统声音，靠「屏幕录制」授权；以后要采麦克风时才加。
codesign --force --options runtime $TS --entitlements Packaging/LemonGalaxy.entitlements --sign "$IDENTITY" "$APP"
codesign --verify --strict "$APP"
echo "$APP"
lipo -archs "$APP/Contents/MacOS/LemonGalaxy"
du -sh "$APP" | cat

if [ "$DMG" = 1 ]; then
  mkdir -p "$OUT"
  DMG_PATH="$OUT/LemonGalaxy-$VERSION.dmg"
  STAGE="$APPDIR/dmg-stage"
  rm -rf "$STAGE" "$DMG_PATH" && mkdir -p "$STAGE"
  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -volname "LemonGalaxy" -srcfolder "$STAGE" -fs HFS+ -format UDZO -imagekey zlib-level=9 "$DMG_PATH" >/dev/null
  rm -rf "$STAGE"
  if [ "$IDENTITY" != "-" ]; then codesign --force $TS --sign "$IDENTITY" "$DMG_PATH"; fi
  echo "$DMG_PATH"
fi
# 构建目录里的副本只给打包和自测用，不登记
"$LSREGISTER" -u "$APP" >/dev/null 2>&1 || true
