#!/usr/bin/env bash
# 编 Windows 老系统兼容版被控端，并核对导入表（每个导入的函数目标系统都要有，否则程序在那里根本启动不了）。
# 在 Windows 上的 Git Bash 里跑（GitHub Actions 的 windows-2025），要 MSVC、Python、7z，以及 nightly + rust-src：
#   scripts/build-windows-legacy.sh win7-x64   Windows 7 SP1 / 8 / 8.1 / Server 2008 R2–2012 R2，x64（另出主控端兼容版用的 galaxy_ffi.dll）
#   scripts/build-windows-legacy.sh win7-x86   Windows Vista SP2 / 7 / 8 / 8.1 / 10，32 位
#   scripts/build-windows-legacy.sh xp         Windows XP SP3 / Server 2003 SP2 起，32 位（也能在更新的 32 位系统上跑）
# 产物在 dist/：lemon-galaxy-worker.exe、imports.txt（导入表），win7-x64 另有 galaxy_ffi.dll。
#
# 做法（原理与取舍见 docs/research/windows-xp-32bit.md）：
# - Rust 标准库要求 Windows 10，用 tier 3 的 *-win7-windows-msvc 目标自己编标准库（-Z build-std）；
# - 32 位链接 YY-Thunks（Vista 版 / XP 版）：Vista、XP 没有的系统接口由它补（运行时有真的用真的）；
# - XP 再链 VC-LTL5 的 XP 库（C 运行库改用系统自带的 msvcrt.dll），子系统版本设 5.01（XP 只认 5.x），
#   加 --cfg lemon_galaxy_xp：自动更新平台名 windowsxp，补上 YY-Thunks 没有的 WSASendMsg（lemon-galaxy-worker-windows/src/win/xp.rs）。
set -euo pipefail
FLAVOR="${1:?用法：$0 win7-x64|win7-x86|xp}"
NIGHTLY="${LEGACY_NIGHTLY:-nightly}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TOOLS="${LEGACY_TOOLS:-$ROOT/target/legacy-tools}"
DIST="${LEGACY_DIST:-$ROOT/dist}"
# 固定版本，SHA-256 取自 GitHub 发布页的文件摘要
YY_VER=1.2.3
YY_SHA=4dc68ec8784e0afed7284e4393aec672873608161c3a5ae52faf618875d95b81
LTL_VER=5.3.1
LTL_SHA=7a18799ed3aa84a225610a5447a56bc534c5c98ccb8dec05caba0e3f633431ad

case "$FLAVOR" in
  win7-x64) TARGET=x86_64-win7-windows-msvc; OS_EXPORTS=x64/6.1.7600; THUNKS= ;;
  win7-x86) TARGET=i686-win7-windows-msvc; OS_EXPORTS=x86/6.0.6000; THUNKS=x86/YY_Thunks_for_Vista.obj ;;
  xp) TARGET=i686-win7-windows-msvc; OS_EXPORTS=x86/5.1.2600; THUNKS=x86/YY_Thunks_for_WinXP.obj ;;
  *) echo "不认识的版本：$FLAVOR（win7-x64 / win7-x86 / xp）" >&2; exit 2 ;;
esac

winpath() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else echo "$1"; fi; }
fetch() { # 地址 SHA-256 保存到
  if [ -f "$3" ] && echo "$2  $3" | sha256sum -c --status; then return; fi
  curl -fsSL --retry 3 -o "$3.part" "$1"
  echo "$2  $3.part" | sha256sum -c -
  mv "$3.part" "$3"
}

mkdir -p "$TOOLS" "$DIST"
# YY-Thunks：XP / Vista 补接口用的目标文件，以及各版本系统 DLL 的导出表（核对导入表用）
YY="$TOOLS/YY-Thunks-$YY_VER"
if [ ! -d "$YY/objs" ]; then
  fetch "https://github.com/Chuyu-Team/YY-Thunks/releases/download/v$YY_VER/YY-Thunks-Objs.zip" "$YY_SHA" "$TOOLS/YY-Thunks-Objs-$YY_VER.zip"
  rm -rf "$YY" && mkdir -p "$YY"
  python -c "import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$(winpath "$TOOLS/YY-Thunks-Objs-$YY_VER.zip")" "$(winpath "$YY")"
fi

RF="-C target-feature=+crt-static"
if [ -n "$THUNKS" ]; then
  RF="$RF -C link-arg=$(winpath "$YY/objs/$THUNKS")"
fi
if [ "$FLAVOR" = xp ]; then
  RF="$RF --cfg lemon_galaxy_xp -C link-arg=/SUBSYSTEM:CONSOLE,5.01"
  if [ "${LEGACY_VCLTL:-1}" = 1 ]; then
    LTL="$TOOLS/VC-LTL-$LTL_VER"
    if [ ! -d "$LTL/TargetPlatform" ]; then
      fetch "https://github.com/Chuyu-Team/VC-LTL5/releases/download/v$LTL_VER/VC-LTL-Binary.7z" "$LTL_SHA" "$TOOLS/VC-LTL-Binary-$LTL_VER.7z"
      rm -rf "$LTL" && mkdir -p "$LTL"
      7z x -y -bd -o"$(winpath "$LTL")" "$(winpath "$TOOLS/VC-LTL-Binary-$LTL_VER.7z")" >/dev/null
    fi
    LIBDIR="$LTL/TargetPlatform/5.1.2600.0/lib/Win32"
    [ -d "$LIBDIR" ] || { echo "VC-LTL 包里没有 $LIBDIR" >&2; ls "$LTL/TargetPlatform" >&2; exit 1; }
    # 链接器先在这里找 libvcruntime / libucrt，用 VC-LTL 的（转到系统自带的 msvcrt.dll）
    RF="$RF -L native=$(winpath "$LIBDIR")"
    # C 依赖（openh264、opus、ring）也用 VC-LTL 的头文件（msvcrt 的区域设置结构与 UCRT 不同）
    INC="/I$(winpath "$LTL/TargetPlatform/header") /I$(winpath "$LTL/TargetPlatform/5.1.2600.0/header")"
    export CFLAGS_i686_win7_windows_msvc="$INC" CXXFLAGS_i686_win7_windows_msvc="$INC"
  fi
fi
if [ "$FLAVOR" != win7-x64 ]; then
  # 32 位要求映像带 SAFESEH 表（rustc 默认 /SAFESEH），openh264 的 NASM 汇编目标文件没有，不编汇编
  export OPENH264_NO_ASM=1
fi
echo "目标 $TARGET，RUSTFLAGS：$RF"

export CARGO_TARGET_DIR="$ROOT/target/legacy-$FLAVOR"
export RUSTFLAGS="$RF"
cd "$ROOT"
cargo "+$NIGHTLY" build -Z build-std=std,panic_abort --release --target "$TARGET" -p lemon-galaxy-worker-windows
OUT="$CARGO_TARGET_DIR/$TARGET/release"
cp "$OUT/lemon-galaxy-worker.exe" "$DIST/"
if [ "$FLAVOR" = win7-x64 ]; then
  # 主控端内核库也出一份 Win7 版（发布时换进主控端压缩包，另出兼容版）
  RUSTFLAGS="-C target-feature=+crt-static" cargo "+$NIGHTLY" build -Z build-std=std,panic_abort --release --target "$TARGET" -p galaxy-ffi --features decoder --lib
  cp "$OUT/galaxy_ffi.dll" "$DIST/"
fi
ls -l "$DIST"

# 核对导入表
DUMPBIN="$(ls /c/Program\ Files/Microsoft\ Visual\ Studio/*/*/VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe 2>/dev/null | tail -1)"
[ -n "$DUMPBIN" ] || { echo "找不到 dumpbin" >&2; exit 1; }
"$DUMPBIN" -nologo -headers "$DIST/lemon-galaxy-worker.exe" | grep -iE "machine|subsystem version|operating system version" || true
"$DUMPBIN" -nologo -imports "$DIST/lemon-galaxy-worker.exe" > "$DIST/imports.txt"
python "$ROOT/scripts/check-legacy-imports.py" "$DIST/imports.txt" "$YY/Config/$OS_EXPORTS.txt"
if [ -f "$DIST/galaxy_ffi.dll" ]; then
  "$DUMPBIN" -nologo -imports "$DIST/galaxy_ffi.dll" > "$DIST/ffi-imports.txt"
  python "$ROOT/scripts/check-legacy-imports.py" "$DIST/ffi-imports.txt" "$YY/Config/$OS_EXPORTS.txt" || echo "主控端兼容版内核库 galaxy_ffi.dll 导入了 Windows 7 没有的函数（见上），先不挡发布"
fi
