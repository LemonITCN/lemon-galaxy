#!/usr/bin/env bash
# GNOME Wayland 后端自检：起无头 mutter（虚拟显示器）+ PipeWire，用 Mutter 的远程桌面接口截屏、编码。
# 以普通用户运行（mutter 不让 root 跑），需要 mutter、pipewire、wireplumber、dbus。
#   scripts/test-mutter-headless.sh <lemon-galaxy-worker 路径>
# root 运行时用 LG_TEST_USER（默认 alice）的身份重新执行自己。
set -euo pipefail
WORKER="$(cd "$(dirname "${1:-target/debug/lemon-galaxy-worker}")" && pwd)/$(basename "${1:-target/debug/lemon-galaxy-worker}")"
if [ "$(id -u)" = 0 ]; then
  U="${LG_TEST_USER:-alice}"
  cp "$0" /tmp/lg-test-mutter.sh && chmod 755 /tmp/lg-test-mutter.sh
  exec su "$U" -c "dbus-run-session -- bash /tmp/lg-test-mutter.sh '$WORKER'"
fi
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
  exec dbus-run-session -- bash "$0" "$WORKER"
fi
export XDG_RUNTIME_DIR="/tmp/lg-rt-$(id -u)"
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
LOG="$(mktemp -d)"
cleanup() { kill $(jobs -p) 2>/dev/null || true; }
trap cleanup EXIT
pipewire >"$LOG/pw.log" 2>&1 &
sleep 1
wireplumber >"$LOG/wp.log" 2>&1 &
sleep 1
mutter --headless --wayland --no-x11 --virtual-monitor 1280x720 >"$LOG/mutter.log" 2>&1 &
sleep 4
OUT="$(LG_BACKEND=mutter XDG_CURRENT_DESKTOP=GNOME "$WORKER" __lg-selftest 2 2>&1 || true)"
echo "$OUT"
if ! echo "$OUT" | grep -q "截屏：mutter-pipewire，1280x720"; then
  echo "== mutter"; tail -20 "$LOG/mutter.log"; echo "== pipewire"; tail -20 "$LOG/pw.log"
  echo "失败：Mutter 后端没有截到画面" >&2
  exit 1
fi
echo "通过"
