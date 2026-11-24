#!/usr/bin/env bash
# Linux 图形被控端到端测试（docs/linux-desktop.md）：以 root 运行被控端，账号绑定普通用户；
# 本机没有图形会话时主控打开屏幕列表 → 自动给该用户开虚拟桌面（Xvfb + 桌面环境）→ 看画面、截图、剪贴板、键鼠。
# 要求：root 运行（或免密 sudo）、装了 Xvfb 与一个桌面环境（如 xfce4）、xdotool、有普通用户（默认 alice，可用 LG_TEST_USER 指定）。
#   scripts/e2e-desktop.sh <二进制目录>
set -euo pipefail
BIN="$(cd "${1:-target/debug}" && pwd)"
TUSER="${LG_TEST_USER:-alice}"
WORK="$(mktemp -d)"
chmod 755 "$WORK"
PORT=$((20000 + RANDOM % 20000))
PIDS=()
SUDO=""
[ "$(id -u)" = 0 ] || SUDO="sudo -n"
cleanup() {
  for p in "${PIDS[@]:-}"; do $SUDO kill "$p" 2>/dev/null || true; done
  $SUDO "$BIN/lemon-galaxy-worker" desktop stop "$TUSER" >/dev/null 2>&1 || true
  $SUDO rm -rf "$WORK"
}
trap cleanup EXIT
fail() { echo "失败：$*" >&2; for f in "$WORK"/*.log /run/lemon-galaxy-worker/vd-*/*.log; do [ -f "$f" ] && { echo "== $f"; $SUDO tail -40 "$f"; }; done; exit 1; }
step() { echo; echo "==== $*"; }
dev() { timeout 90 "$BIN/galaxy-dev" --data "$WORK/ctl" "$@" 2>&1 | grep -v -E 'INFO|DEBUG|WARN' || true; }

id "$TUSER" >/dev/null 2>&1 || fail "没有用户 $TUSER"
pgrep -x Xvfb >/dev/null && fail "已经有 Xvfb 在运行，测试要求本机没有图形会话"

step "启动服务器（端口 ${PORT}）"
"$BIN/lemon-galaxy-server" --data-dir "$WORK/srv" --port "$PORT" user create alice init-pass-123 --display-name Alice >/dev/null
"$BIN/lemon-galaxy-server" --data-dir "$WORK/srv" --port "$PORT" >"$WORK/server.log" 2>&1 &
PIDS+=($!)
for i in $(seq 1 50); do grep -q "客户端填写的服务器地址" "$WORK/server.log" 2>/dev/null && break; sleep 0.2; done
ADDR="$(grep -o '客户端填写的服务器地址：.*' "$WORK/server.log" | sed 's/.*：//' | sed 's/\x1b\[[0-9;]*m//g' | tr -d '\r')"
[ -n "$ADDR" ] || fail "服务器没有启动"
"$BIN/galaxy-dev" --data "$WORK/ctl" login "$ADDR" alice init-pass-123 alice-new-pass-456 | tail -1

step "被控端（root）登录，绑定系统用户 $TUSER"
$SUDO env LEMON_GALAXY_PASSWORD=alice-new-pass-456 "$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker" login --server "$ADDR" --user alice --system-user "$TUSER" >"$WORK/login.log" 2>&1 || fail "被控端登录失败"
$SUDO env RUST_LOG=info "$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker" run >"$WORK/worker.log" 2>&1 &
PIDS+=($!)
sleep 2

step "能力列表里有画面"
CAPS="$(dev caps "")"
echo "$CAPS" | grep "^能力"
echo "$CAPS" | grep "^能力" | grep -q "display" || fail "没有画面能力：$CAPS"

step "打开屏幕列表：自动开虚拟桌面"
DISP="$(dev displays "")"
echo "$DISP"
echo "$DISP" | grep -q "虚拟 true" || fail "没有虚拟桌面：$DISP"
$SUDO "$BIN/lemon-galaxy-worker" desktop | tee "$WORK/desktop.txt"
grep -q "虚拟桌面：.*$TUSER" "$WORK/desktop.txt" || fail "被控端没有记录虚拟桌面"
VD="$(ls -d /run/lemon-galaxy-worker/vd-* | head -1)"
VDISPLAY="$($SUDO cat "$VD/state.json" | sed -E 's/.*"display":"([^"]+)".*/\1/')"
echo "虚拟桌面显示号 $VDISPLAY"

step "看画面 4 秒"
VIEW="$(dev view "" 0 4)"
echo "$VIEW" | tail -2
N="$(echo "$VIEW" | tail -1 | sed -E 's/.*累计 ([0-9]+) 帧.*/\1/')"
[ "${N:-0}" -ge 2 ] || fail "画面帧数太少：$VIEW"

step "截图"
dev shot "" 0 "$WORK/shot.jpg"
SIZE="$(wc -c <"$WORK/shot.jpg")"
[ "$SIZE" -gt 5000 ] || fail "截图太小（$SIZE 字节）"

step "剪贴板文字往返"
dev clip-text "" set "hello-剪贴板-e2e"
sleep 0.5
CLIP="$(dev clip-text "")"
echo "$CLIP"
echo "$CLIP" | grep -q "hello-剪贴板-e2e" || fail "剪贴板读回不对：$CLIP"

step "远程键鼠：点 (321,234) 后输入文字，核对光标位置"
dev type "" 0 "abc" 321 234 | tail -1
LOC="$($SUDO env DISPLAY="$VDISPLAY" XAUTHORITY="$VD/Xauthority" xdotool getmouselocation 2>&1)"
echo "$LOC"
echo "$LOC" | grep -q "x:321 y:234" || fail "光标位置不对：$LOC"

step "关掉虚拟桌面"
$SUDO "$BIN/lemon-galaxy-worker" desktop stop "$TUSER"
echo "通过"
