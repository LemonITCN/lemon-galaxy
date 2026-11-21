#!/usr/bin/env bash
# 端到端测试：本机起服务器 + 被控端 + 开发用主控端，验证登录、连接（直连 / UDP 中转 / TCP 中转）、
# 命令执行、终端断线继承、远端声音（测试音）、文件传输、复制粘贴文件、远程开机。
# Linux / macOS：有免密 sudo 时再以 root 运行被控端，验证切换系统用户与降权文件操作。
# Windows（Git Bash）：有管理员权限时再装成 SYSTEM 服务，验证命令与文件以指定账号身份执行。
#   scripts/e2e.sh <二进制目录>
set -euo pipefail
WIN=0
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) WIN=1 ;; esac
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

step "启动服务器（端口 ${PORT}）"
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
LEMON_GALAXY_TEST_TONE=1 "$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker" run >"$WORK/worker.log" 2>&1 &
WPID=$!
PIDS+=($WPID)
sleep 2
"$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker" status >"$WORK/status.txt"
head -6 "$WORK/status.txt"

check() { # 期望输出里包含某段文字
  local want="$1"; shift
  local out
  out="$("$@" 2>&1 | grep -v -E 'INFO|DEBUG' || true)"
  echo "$out" | tail -6
  echo "$out" | grep -q -- "$want" || fail "没有看到「${want}」：$*"
}

step "远程执行（自动选路）"
if [ "$WIN" = 1 ]; then
  check "hello-e2e" timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" exec "" "echo hello-e2e; whoami; [Environment]::OSVersion.VersionString"
else
  check "hello-e2e" timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" exec "" "echo hello-e2e && uname -sm && id -un"
fi

step "终端断线继承"
check "接续后环境变量还在（断线继承）：true" timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" term-test ""

step "远端声音（被控端以测试音代替系统声音，440 Hz）"
AUD="$(timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" audio-test "" 3 2>&1 | grep -v -E 'INFO|DEBUG' || true)"
echo "$AUD" | tail -2
echo "$AUD" | grep "声音：" | sed -E 's/.*声音：([0-9.]+) 秒，均方根 ([0-9.]+)，估算频率 ([0-9]+) Hz.*/\1 \2 \3/' |
  awk '{ ok = ($1 >= 2.0 && $2 >= 0.25 && $2 <= 0.45 && $3 >= 400 && $3 <= 480); exit ok ? 0 : 1 }' || fail "远端声音不对：$AUD"

step "文件上传下载（8MB）"
head -c 8000000 /dev/urandom >"$WORK/big.bin"
timeout 120 "$BIN/galaxy-dev" --data "$WORK/ctl" put "" "$WORK/big.bin" "$WORK/up.bin" >/dev/null 2>&1 || fail "上传失败"
cmp "$WORK/big.bin" "$WORK/up.bin" || fail "上传内容不一致"
timeout 120 "$BIN/galaxy-dev" --data "$WORK/ctl" get "" "$WORK/up.bin" "$WORK/down.bin" >/dev/null 2>&1 || fail "下载失败"
cmp "$WORK/big.bin" "$WORK/down.bin" || fail "下载内容不一致"
echo "上传下载内容一致"

step "复制粘贴文件（文件夹传到被控端暂存区，再取回比对）"
mkdir -p "$WORK/clip/目录 A/子目录" && echo "一" >"$WORK/clip/目录 A/1.txt" && head -c 300000 /dev/urandom >"$WORK/clip/目录 A/子目录/2.bin" && echo "顶层" >"$WORK/clip/顶层.txt"
PUSH="$(timeout 120 "$BIN/galaxy-dev" --data "$WORK/ctl" clip-push "" "$WORK/clip/目录 A" "$WORK/clip/顶层.txt" 2>&1 | grep -v -E 'INFO|DEBUG' || true)"
echo "$PUSH" | tail -2
REMOTES=()
while IFS= read -r line; do REMOTES+=("$line"); done < <(echo "$PUSH" | sed -n 's/^远端：//p')
[ "${#REMOTES[@]}" = 2 ] || fail "没有拿到远端暂存路径：$PUSH"
timeout 120 "$BIN/galaxy-dev" --data "$WORK/ctl" clip-pull "" "$WORK/clipback" "${REMOTES[@]}" 2>&1 | grep -v -E 'INFO|DEBUG' | tail -1
diff -r "$WORK/clip" "$WORK/clipback" || fail "取回的内容与原文件不一致"
echo "复制粘贴文件往返一致"
# 被控端就是本机当前用户：清掉写进家目录的暂存
STAGE="${REMOTES[0]%/*}"
[ "$WIN" = 1 ] && STAGE="$(cygpath -u "${STAGE#/}")"
rm -rf "$STAGE"

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

step "远程开机（第一台被控端离线，由同一网络里的第二台代发唤醒包）"
W1="$(od -An -tx1 "$WORK/worker/keys/device_id" | tr -d ' \n')"
sleep 1
"$BIN/galaxy-dev" --data "$WORK/ctl" devices >"$WORK/devices.txt" 2>&1 || true
cat "$WORK/devices.txt"
if grep "^$W1" "$WORK/devices.txt" | grep -qE "([0-9A-F]{2}:){5}[0-9A-F]{2}"; then
  LEMON_GALAXY_PASSWORD=alice-new-pass-456 "$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker2" login --server "$ADDR" --user alice --system-user "$(id -un)" | tail -1
  "$BIN/lemon-galaxy-worker" --data-dir "$WORK/worker2" run >"$WORK/worker2.log" 2>&1 &
  W2PID=$!
  PIDS+=($W2PID)
  sleep 2
  check "已发送唤醒包" timeout 60 "$BIN/galaxy-dev" --data "$WORK/ctl" wake "${W1:0:8}"
  kill "$W2PID" 2>/dev/null || true
else
  echo "这台机器的网卡没有可用于唤醒的地址（随机化 MAC 或只有虚拟网卡），跳过"
fi

if [ "$WIN" = 0 ] && sudo -n true 2>/dev/null; then
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
  [ "$OWNER" = "$ME" ] || fail "文件属主应为 ${ME}，实际 ${OWNER}（降权没生效）"
  echo "降权文件操作正确：属主 $OWNER"
  rm -f "/tmp/lg-e2e-$$.txt"
fi

if [ "$WIN" = 1 ] && net session >/dev/null 2>&1; then
  step "装成 Windows 服务（SYSTEM），命令与文件以当前账号身份执行"
  ME="${USERNAME:-$(id -un)}"
  SVC="$WORK/svc"
  LEMON_GALAXY_PASSWORD=alice-new-pass-456 "$BIN/lemon-galaxy-worker" --data-dir "$SVC" login --server "$ADDR" --user alice --system-user "$ME" | tail -1
  "$BIN/lemon-galaxy-worker" --data-dir "$SVC" install-service || fail "安装服务失败"
  cleanup_svc() { "$BIN/lemon-galaxy-worker" uninstall-service >/dev/null 2>&1 || true; }
  trap 'cleanup_svc; cleanup' EXIT
  for i in $(seq 1 30); do "$BIN/lemon-galaxy-worker" --data-dir "$SVC" status 2>/dev/null | grep -q "已连上服务器\|connected" && break; sleep 1; done
  "$BIN/lemon-galaxy-worker" --data-dir "$SVC" status >"$WORK/svc-status.txt" 2>&1 || true
  head -8 "$WORK/svc-status.txt"
  SID_HEX="$(od -An -tx1 "$SVC/keys/device_id" | tr -d ' \n')"
  check "$ME" timeout 90 "$BIN/galaxy-dev" --data "$WORK/ctl" exec "${SID_HEX:0:8}" "whoami"
  echo "svc-helper-test" >"$WORK/st.txt"
  # 「~」应解析到该账号自己的用户目录（不是 SYSTEM 的）
  timeout 90 "$BIN/galaxy-dev" --data "$WORK/ctl" put "${SID_HEX:0:8}" "$WORK/st.txt" "~/lg-e2e-$$.txt" >/dev/null 2>&1 || fail "服务被控端上传失败"
  DEST="$(cygpath -u "$USERPROFILE")/lg-e2e-$$.txt"
  [ -f "$DEST" ] || fail "文件没有落在 $USERPROFILE（以指定账号写文件没生效）"
  OWNER="$(powershell -NoProfile -Command "(Get-Acl '$(cygpath -w "$DEST")').Owner")"
  echo "文件已写到 $USERPROFILE，属主：$OWNER"
  echo "$OWNER" | grep -qi "NT AUTHORITY" && fail "文件属主是系统账号（以指定账号写文件没生效）"
  rm -f "$DEST"
  check "first-42" timeout 90 "$BIN/galaxy-dev" --data "$WORK/ctl" term-test "${SID_HEX:0:8}"
  cleanup_svc
fi

echo
echo "端到端测试全部通过"
