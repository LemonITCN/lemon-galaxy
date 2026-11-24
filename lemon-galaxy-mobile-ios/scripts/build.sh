#!/usr/bin/env bash
# 不签名地编出 LemonGalaxy iOS 的模拟器版与真机版（CI 用）。
#   scripts/build.sh [--debug] [--sim-only|--device-only] [--no-core] [--test]
# 产物：build/Products/<配置>-iphonesimulator/LemonGalaxy.app、build/Products/<配置>-iphoneos/LemonGalaxy.app
#       以及 build/LemonGalaxy-<配置>-iphoneos-unsigned.ipa（未签名，装机前需要重新签名）。
# 有 Vendor/LemonGalaxyCore.xcframework 与 Sources/Generated/galaxy_ffi.swift 时接入真内核（仓库根 scripts/build-ffi-apple.sh --ios 生成），
# 没有或加 --no-core 时只编模拟内核（类型来自 Sources/CoreMirror/BridgeMirror.swift）。
# --test 先在 macOS 上跑 GalaxyKit 的单元测试（swift test）。
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
CONFIG=Release
SIM=1
DEVICE=1
TEST=0
for a in "$@"; do
  case "$a" in
    --debug) CONFIG=Debug ;;
    --sim-only) DEVICE=0 ;;
    --device-only) SIM=0 ;;
    --no-core) export LEMONGALAXY_NO_CORE=1 ;;
    --test) TEST=1 ;;
    *) echo "未知参数 $a" >&2; exit 2 ;;
  esac
done

if [ "${LEMONGALAXY_NO_CORE:-0}" != 1 ] && [ -f Vendor/LemonGalaxyCore.xcframework/Info.plist ] && [ -f Sources/Generated/galaxy_ffi.swift ]; then
  echo "内核：Rust 真内核（Vendor/LemonGalaxyCore.xcframework）"
else
  echo "内核：只有模拟内核（没有内核产物或指定了 --no-core）"
fi

if [ "$TEST" = 1 ]; then
  swift test
fi

# .noindex：不被聚焦索引，iPhone / 模拟器版的 LemonGalaxy.app 不会出现在 Mac 的应用列表里（显示成打叉的禁用图标）
DD="$ROOT/build.noindex"
COMMON=(-project LemonGalaxy.xcodeproj -scheme LemonGalaxy -configuration "$CONFIG" -derivedDataPath "$DD"
        CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY= -skipPackagePluginValidation)
# 包清单读环境变量（LEMONGALAXY_NO_CORE）：换内核模式后要让 Xcode 重新解析
xcodebuild -resolvePackageDependencies -project LemonGalaxy.xcodeproj -scheme LemonGalaxy -derivedDataPath "$DD" >/dev/null

if [ "$SIM" = 1 ]; then
  xcodebuild "${COMMON[@]}" -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' build
  echo "模拟器版：$DD/Build/Products/$CONFIG-iphonesimulator/LemonGalaxy.app"
fi
if [ "$DEVICE" = 1 ]; then
  xcodebuild "${COMMON[@]}" -sdk iphoneos -destination 'generic/platform=iOS' build
  APP="$DD/Build/Products/$CONFIG-iphoneos/LemonGalaxy.app"
  echo "真机版：$APP"
  # 未签名的 ipa（CI 归档用；装机或上传 TestFlight 前要用自己的证书重新签名）
  TMP="$(mktemp -d)"
  mkdir -p "$TMP/Payload"
  cp -R "$APP" "$TMP/Payload/"
  (cd "$TMP" && zip -qry "$DD/LemonGalaxy-$CONFIG-iphoneos-unsigned.ipa" Payload)
  rm -rf "$TMP"
  echo "未签名 ipa：$DD/LemonGalaxy-$CONFIG-iphoneos-unsigned.ipa"
fi
