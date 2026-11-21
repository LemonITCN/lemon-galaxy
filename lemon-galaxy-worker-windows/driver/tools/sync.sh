#!/bin/bash
# 把驱动工程拷到 Windows 开发机（tar 走 SSH），可选地顺手编译。
#   LG_WIN_HOST=user@host lemon-galaxy-worker-windows/driver/tools/sync.sh [build Release x64]
# LG_WIN_DIR 是那台机器上的目录（默认 D:/dev/lemon-galaxy-driver）；LG_WIN_KEY 是 SSH 私钥（可选）。
set -euo pipefail
host=${LG_WIN_HOST:?set LG_WIN_HOST to user@host of the Windows dev machine}
dir=${LG_WIN_DIR:-D:/dev/lemon-galaxy-driver}
key=()
[ -n "${LG_WIN_KEY:-}" ] && key=(-i "$LG_WIN_KEY")
root=$(cd "$(dirname "$0")/.." && pwd)
ssh "${key[@]}" -o BatchMode=yes "$host" "if not exist \"${dir//\//\\}\" mkdir \"${dir//\//\\}\""
# COPYFILE_DISABLE keeps macOS tar from adding ._ files with extended attributes.
COPYFILE_DISABLE=1 tar -C "$root" --exclude './build' --exclude '.vs' --exclude '*.user' --exclude '._*' -czf - . |
    ssh "${key[@]}" -o BatchMode=yes "$host" "tar -xzf - -C $dir"
if [ "${1:-}" = build ]; then
    shift
    ssh "${key[@]}" -o BatchMode=yes "$host" "${dir//\//\\}\\tools\\build.cmd $*"
fi
