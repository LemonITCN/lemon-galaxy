#!/usr/bin/env bash
# 把 Rust 内核（galaxy-ffi）打成 LemonGalaxyCore.xcframework，并生成 Swift 绑定。
#   scripts/build-ffi-apple.sh [--debug] [--ios]
# 产物：
#   lemon-galaxy-control-macos/Vendor/LemonGalaxyCore.xcframework   macOS 通用（arm64 + x86_64），--ios 时加 iOS 真机与模拟器切片
#   lemon-galaxy-control-macos/Sources/LemonGalaxy/Core/Generated/galaxy_ffi.swift
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
PROFILE=release
CARGO_FLAG=--release
IOS=0
for a in "$@"; do
  case "$a" in
    --debug) PROFILE=debug; CARGO_FLAG= ;;
    --ios) IOS=1 ;;
  esac
done
export MACOSX_DEPLOYMENT_TARGET=13.0
export IPHONEOS_DEPLOYMENT_TARGET=16.0

MAC_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
for t in "${MAC_TARGETS[@]}"; do
  rustup target list --installed | grep -q "^$t$" || rustup target add "$t"
  cargo build -p galaxy-ffi --lib $CARGO_FLAG --target "$t"
done

OUT="$ROOT/target/apple-ffi"
rm -rf "$OUT" && mkdir -p "$OUT/macos" "$OUT/headers" "$OUT/swift"
lipo -create "${MAC_TARGETS[@]/#/$ROOT/target/}" -output /dev/null 2>/dev/null || true
lipo -create \
  "$ROOT/target/aarch64-apple-darwin/$PROFILE/libgalaxy_ffi.a" \
  "$ROOT/target/x86_64-apple-darwin/$PROFILE/libgalaxy_ffi.a" \
  -output "$OUT/macos/libgalaxy_ffi.a"

# 生成绑定（用主机架构的动态库读取元数据）
cargo build -p galaxy-ffi --features bindgen --bin uniffi-bindgen $CARGO_FLAG
HOST_DYLIB="$ROOT/target/$PROFILE/libgalaxy_ffi.dylib"
[ -f "$HOST_DYLIB" ] || cargo build -p galaxy-ffi --lib $CARGO_FLAG
"$ROOT/target/$PROFILE/uniffi-bindgen" generate --library "$HOST_DYLIB" --language swift --out-dir "$OUT/swift"
cp "$OUT/swift/galaxy_ffiFFI.h" "$OUT/headers/"
cp "$OUT/swift/galaxy_ffiFFI.modulemap" "$OUT/headers/module.modulemap"

ARGS=(-library "$OUT/macos/libgalaxy_ffi.a" -headers "$OUT/headers")
if [ "$IOS" = 1 ]; then
  for t in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
    rustup target list --installed | grep -q "^$t$" || rustup target add "$t"
    cargo build -p galaxy-ffi --lib $CARGO_FLAG --target "$t"
  done
  mkdir -p "$OUT/ios" "$OUT/ios-sim"
  cp "$ROOT/target/aarch64-apple-ios/$PROFILE/libgalaxy_ffi.a" "$OUT/ios/"
  lipo -create "$ROOT/target/aarch64-apple-ios-sim/$PROFILE/libgalaxy_ffi.a" "$ROOT/target/x86_64-apple-ios/$PROFILE/libgalaxy_ffi.a" -output "$OUT/ios-sim/libgalaxy_ffi.a"
  ARGS+=(-library "$OUT/ios/libgalaxy_ffi.a" -headers "$OUT/headers" -library "$OUT/ios-sim/libgalaxy_ffi.a" -headers "$OUT/headers")
fi

DEST="$ROOT/lemon-galaxy-control-macos/Vendor/LemonGalaxyCore.xcframework"
rm -rf "$DEST"
mkdir -p "$(dirname "$DEST")"
xcodebuild -create-xcframework "${ARGS[@]}" -output "$DEST" >/dev/null
GEN="$ROOT/lemon-galaxy-control-macos/Sources/LemonGalaxy/Core/Generated"
mkdir -p "$GEN"
cp "$OUT/swift/galaxy_ffi.swift" "$GEN/galaxy_ffi.swift"
# iOS 工程（若存在）也用同一份
if [ -d "$ROOT/lemon-galaxy-mobile-ios" ]; then
  mkdir -p "$ROOT/lemon-galaxy-mobile-ios/Vendor" "$ROOT/lemon-galaxy-mobile-ios/Sources/Generated"
  rm -rf "$ROOT/lemon-galaxy-mobile-ios/Vendor/LemonGalaxyCore.xcframework"
  cp -R "$DEST" "$ROOT/lemon-galaxy-mobile-ios/Vendor/"
  cp "$OUT/swift/galaxy_ffi.swift" "$ROOT/lemon-galaxy-mobile-ios/Sources/Generated/galaxy_ffi.swift"
fi
echo "已生成 $DEST"
echo "已生成 $GEN/galaxy_ffi.swift"
