#!/usr/bin/env bash
# 编 XP 虚拟屏驱动（32 位 XPDM：小端口 lgxpmp.sys + 显示驱动 lgxpdd.dll），用 llvm-mingw，Mac / Linux / Windows 都能编。
#   lemon-galaxy-worker-windows/driver/xp/build.sh [输出目录]   （默认 build/xp）
# 没有 C 运行库、没有栈探测；内核映像要正确的 PE 校验和（编完用 pe-checksum.py 写进去）。
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-$HERE/../build/xp}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
LLVM_MINGW="${LLVM_MINGW:-$(ls -d "$HOME"/.cache/lemon-galaxy/build-tools/llvm-mingw-*/ 2>/dev/null | tail -1)}"
CC="${LLVM_MINGW:+$LLVM_MINGW/bin/}i686-w64-mingw32-clang"
command -v "$CC" >/dev/null || { echo "找不到 i686-w64-mingw32-clang（设 LLVM_MINGW 指到 llvm-mingw 目录）" >&2; exit 1; }
INC="$(dirname "$(command -v "$CC")")/../generic-w64-mingw32/include"
RT="$("$CC" -print-libgcc-file-name)"
CFLAGS=(-O2 -Wall -Wextra -Wno-unused-parameter -Wno-cast-function-type-mismatch -Werror -ffreestanding -fno-stack-protector -mno-stack-arg-probe -fno-asynchronous-unwind-tables -I"$HERE")
LDXP=(-nostdlib -Wl,--major-os-version,5 -Wl,--minor-os-version,1 -Wl,--major-subsystem-version,5 -Wl,--minor-subsystem-version,1 -Wl,--dynamicbase -Wl,--no-insert-timestamp)

# 小端口：只导入 videoprt.sys
"$CC" "${CFLAGS[@]}" -I"$INC/ddk" -c "$HERE/lgxpmp.c" -o "$OUT/lgxpmp.o"
"$CC" "${LDXP[@]}" -Wl,--subsystem,native -Wl,--entry,_DriverEntry@8 -Wl,--image-base,0x10000 -o "$OUT/lgxpmp.sys" "$OUT/lgxpmp.o" -lvideoprt "$RT"

# 显示驱动：只导入 win32k.sys，映像入口就是 DrvEnableDriver
"$CC" "${CFLAGS[@]}" -I"$HERE/compat" -c "$HERE/lgxpdd.c" -o "$OUT/lgxpdd.o"
"$CC" "${LDXP[@]}" -shared -Wl,--subsystem,native -Wl,--entry,_DrvEnableDriver@12 -o "$OUT/lgxpdd.dll" "$OUT/lgxpdd.o" -lwin32k "$RT"

rm -f "$OUT"/*.o "$OUT"/*.a
python3 -I "$HERE/pe-checksum.py" "$OUT/lgxpmp.sys" "$OUT/lgxpdd.dll"
cp "$HERE/LemonGalaxyDisplayXP.inf" "$OUT/"
ls -la "$OUT"
