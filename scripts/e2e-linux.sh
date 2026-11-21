#!/usr/bin/env bash
# 端到端测试：本机起服务器 + Linux 被控端 + 开发用主控端，验证登录、连接（直连 / UDP 中转 / TCP 中转）、
# 命令执行、终端断线继承、文件传输；有免密 sudo 时再以 root 运行被控端，验证切换系统用户与降权文件操作。
#   scripts/e2e-linux.sh <二进制目录>
set -euo pipefail
BIN="$(cd "${1:-target/debug}" && pwd)"
WORK="$(mktemp -d)"
PORT=$((20000 + RANDOM % 20000))
PIDS=()
cleanup() {
  for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || sudo -n kill "$p" 2>/dev/null || true; done
  sudo -n rm -rf "$WORK" 2>/dev/null || rm -rf "$WORK"
}
trap cleanup EXIT
fail() { echo "失败：$*" >&2; for f in "$WORK"/*.log; do echo "== $f"; tail -40 "$f"; done; exit 1; }
step() { echo; echo "==== $*"; }

step "启动服务器（端口 $PORT）"
"$BIN/lemon-galaxy-server" --data-dir "$WORK/srv" --port "$PORT" user create alice init-pass-123 --display-name Alice
"$BIN/lemon-galaxy-server" --data-dir "$WORK/srv" --port "$PORT" >"$WORK/server.log" 2>&1 &
PIDS+=($!)
for i in $(seq 1 50); do grep -q "客户端填写的服务器地址" "$WORK/server.log" 2>/dev/null && break; sleep 0.2; done
ADDR="$(grep -o '客户端填写的服务器地址：.*' "$WORK/server.log" | sed 's/.*：//' | sed 's/\x1b\[[0-9;]*m//g' | tr -d '\r')"
[ -n "$ADDR" ] || fail "服务器没有启动"
echo "地址 $ADDR"

step "主控端首次登录（改密码、生成账号密钥）"
"$BIN/galaxy-dev" --data "$WORK/ctl" login "$ADDR" alice init-pass-123 alice-new-pass-456 | tail -1

step "被控端登录并运行"
LEMON_GALAXY_PASSWORD=alice-new-pass-456 "$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker" login --server "$ADDR" --user alice --system-user "$(id -un)" | tail -2
"$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker" run >"$WORK/worker.log" 2>&1 &
WPID=$!
PIDS+=($WPID)
sleep 2
"$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker" status | head -6

check() { # 期望输出里包含某段文字
  local want="$1"; shift
  local out
  out="$("$@" 2>&1 | grep -v -E 'INFO|DEBUG' || true)"
  echo "$out" | tail -6
  echo "$out" | grep -q -- "$want" || fail "没有看到「$want」：$*"
}

step "远程执行（自动选路）"
check "hello-e2e" timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" exec "" "echo hello-e2e && uname -sm && id -un"

step "终端断线继承"
check "接续后环境变量还在（断线继承）：true" timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" term-test ""

step "文件上传下载（8MB）"
head -c 8000000 /dev/urandom >"$WORK/big.bin"
timeout 120 "$BIN/galaxy-dev" --data "$WORK/ctl" put "" "$WORK/big.bin" "$WORK/up.bin" >/dev/null 2>&1 || fail "上传失败"
cmp "$WORK/big.bin" "$WORK/up.bin" || fail "上传内容不一致"
timeout 120 "$BIN/galaxy-dev" --data "$WORK/ctl" get "" "$WORK/up.bin" "$WORK/down.bin" >/dev/null 2>&1 || fail "下载失败"
cmp "$WORK/big.bin" "$WORK/down.bin" || fail "下载内容不一致"
echo "上传下载内容一致"

for mode in udp tcp; do
  step "强制 $mode 中转"
  kill "$WPID" 2>/dev/null || true
  sleep 0.5
  LEMON_GALAXY_FORCE_RELAY=$mode "$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker" run >"$WORK/worker-$mode.log" 2>&1 &
  WPID=$!
  PIDS+=($WPID)
  sleep 2
  out="$(LEMON_GALAXY_FORCE_RELAY=$mode timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" exec "" "echo via-$mode" 2>&1 || true)"
  echo "$out" | grep -E "路径|via-" | tail -3
  echo "$out" | grep -q "via-$mode" || fail "$mode 中转不通"
  case $mode in
    udp) echo "$out" | grep -q "RelayUdp" || fail "没有走 UDP 中转" ;;
    tcp) echo "$out" | grep -q "RelayTcp" || fail "没有走 TCP 中转" ;;
  esac
done
kill "$WPID" 2>/dev/null || true

if sudo -n true 2>/dev/null; then
  step "以 root 运行被控端，终端与文件以普通用户身份执行"
  ME="$(id -un)"
  sudo -n env LEMON_GALAXY_PASSWORD=alice-new-pass-456 "$BIN/lemon-galaxy-worker" --data-dir "$WORK/rootworker" login --server "$ADDR" --user alice --system-user "$ME" | tail -1
  sudo -n "$BIN/lemon-galaxy-worker" --data-dir "$WORK/rootworker" run >"$WORK/rootworker.log" 2>&1 &
  PIDS+=($!)
  sleep 2
  # 设备列表里会有两台被控端，取 root 那台（最后登录的）
  RID="$(sudo -n cat "$WORK/rootworker/keys/device_id" | od -An -tx1 | tr -d ' \n')"
  check "$ME" timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" exec "${RID:0:8}" "id -un"
  echo "root-helper-test" >"$WORK/rt.txt"
  timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" put "${RID:0:8}" "$WORK/rt.txt" "/tmp/lg-e2e-$$.txt" >/dev/null 2>&1 || fail "root 被控端上传失败"
  OWNER="$(stat -c %U "/tmp/lg-e2e-$$.txt")"
  [ "$OWNER" = "$ME" ] || fail "文件属主应为 $ME，实际 $OWNER（降权没生效）"
  echo "降权文件操作正确：属主 $OWNER"
  rm -f "/tmp/lg-e2e-$$.txt"
fi

echo
echo "端到端测试全部通过"
