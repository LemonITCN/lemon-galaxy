#!/usr/bin/env bash
# 被控端自动更新端到端测试：本机起服务器，被控端装成系统服务，发布新版本后经本机控制接口「立即更新」（与界面按钮同一条路），验证
#   1. 好版本：服务下载、换上（Windows 装了桌面端时整套换：界面、被控服务、资源）、自己重启、以新版本连回服务器、确认；
#   2. 起不来的版本（文件坏了）：装之前试跑就发现，不换、服务照常；
#   3. 起来就崩的版本：服务管理器一次次拉起，更新守护进程换回旧版本、记成坏版本，之后不再装它；
#   4. 一直连不上服务器的版本（只在 Linux 测，等试运行超时）：同样换回；
#   5. 主控端远程「升级被控端」（galaxy-dev remote-update）：同一条路，以新版本重新上线。
# 用的是只认测试发布密钥的测试构建（--features galaxy-proto/test-release-key，见 galaxy-proto/src/update.rs）：
# 正式发布私钥只在发布任务里，测试密钥的私钥种子就写在下面，正式构建不认它。
#
#   scripts/e2e-update.sh <测试构建目录>
#       测试构建目录里要有 lemon-galaxy-server、galaxy-dev、lemon-galaxy-worker（Windows 带 .exe），版本号编成 9.8.7.6000：
#       LEMONGALAXY_BUILD_VERSION=9.8.7.6000 cargo build -p lemon-galaxy-server -p galaxy-dev -p <被控端> --features galaxy-proto/test-release-key
#   scripts/e2e-update.sh --deb <测试构建目录> <界面程序> <架构>
#       Linux 安装包：装 9.8.7.6000 的 .deb，再经自动更新装 9.8.7.6001 的 .deb（更新守护进程调 dpkg，包脚本重启服务）
#
# 被控服务怎么跑：Linux 有免密 sudo 与 systemd 时装成 systemd 系统服务；Windows（Git Bash）有管理员权限时装成 Windows 服务；
# 都没有时（开发机）用一个 shell 循环当服务管理器（只测好版本与起来就崩的版本）。
set -euo pipefail
DEB=0
if [ "${1:-}" = --deb ]; then DEB=1; shift; fi
BIN="$(cd "${1:?用法见脚本开头}" && pwd)"
GUI_BIN="${2:-}"
DEB_ARCH_IN="${3:-}"
HERE="$(cd "$(dirname "$0")" && pwd)"
WIN=0
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) WIN=1 ;; esac
X=""
[ "$WIN" = 1 ] && X=".exe"
PY=python3
command -v python3 >/dev/null 2>&1 && python3 -c 'import sys' >/dev/null 2>&1 || PY=python
TOOL=("$PY" -I "$HERE/e2e-update-tool.py")
# 测试发布密钥的私钥种子（公开的；只有测试构建认它）：SHA-256("lemon-galaxy/test-release-key/v1: only trusted by builds with feature test-release-key")
TEST_SEED="4S6sdo18V95wqRdDdh9iT2pYWUhEjmvrFyJJ3msphlQ="
V0=9.8.7.6000
TRIAL=90
WORK="$(mktemp -d)"
PORT=$((20000 + RANDOM % 20000))
PIDS=()
MODE=loop
if [ "$WIN" = 1 ]; then
  net session >/dev/null 2>&1 && MODE=windows
elif [ "$(uname -s)" = Linux ] && sudo -n true 2>/dev/null && [ -d /run/systemd/system ]; then
  MODE=systemd
fi
[ "$DEB" = 1 ] && [ "$MODE" != systemd ] && { echo "安装包测试要 Linux、免密 sudo 与 systemd" >&2; exit 1; }
SUDO=""
[ "$MODE" = systemd ] && SUDO="sudo -n"
# 没有 timeout 的系统（macOS）直接跑
to() { if command -v timeout >/dev/null 2>&1; then timeout "$@"; else shift; "$@"; fi; }
sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
ME="$(id -un)"
[ "$WIN" = 1 ] && ME="${USERNAME:-$ME}"

step() { echo; echo "==== $*"; }
SPID=""
fail() {
  echo "失败：$*" >&2
  for f in "$WORK"/*.log; do [ -f "$f" ] && { echo "== $f"; tail -40 "$f"; }; done
  svc_logs || true
  exit 1
}

# ---------------- 被控服务（三种跑法） ----------------
SVC="$WORK/svc"
[ "$DEB" = 1 ] && SVC=/var/lib/lemon-galaxy-worker
INST="$WORK/inst"
# Windows：被控服务装在 %ProgramFiles%\LemonGalaxy\Worker（install-service 复制到这里）
if [ "$MODE" = windows ]; then INST="$(cygpath -u "$(powershell -NoProfile -Command '$env:ProgramFiles' | tr -d '\r')")/LemonGalaxy/Worker"; fi
CTL=system
[ "$MODE" = loop ] && CTL="$WORK/ctl.sock"

svc_logs() {
  case "$MODE" in
    systemd) sudo -n journalctl -u lemon-galaxy-worker --no-pager -n 60 2>/dev/null | tail -60 ;;
    windows) tail -60 "$SVC/logs/worker.log" 2>/dev/null ;;
    loop) tail -60 "$WORK/svc.log" 2>/dev/null ;;
  esac
  echo "== 更新守护进程日志"
  $SUDO tail -40 "$SVC/logs/update.log" 2>/dev/null || true
  echo "== 更新记录"
  $SUDO cat "$SVC/update/state.json" 2>/dev/null || true
}

svc_install() {
  case "$MODE" in
    systemd)
      if [ "$DEB" = 0 ]; then
        sudo -n "$BIN/lemon-galaxy-worker" --data-dir "$SVC" install-service >/dev/null || fail "安装服务失败"
      fi
      # 测试把试运行期缩短（只有 root 能改服务的环境变量）
      sudo -n mkdir -p /etc/systemd/system/lemon-galaxy-worker.service.d
      printf '[Service]\nEnvironment=LEMON_GALAXY_UPDATE_TRIAL_SECS=%s LEMON_GALAXY_UPDATE_HEALTHY_SECS=10\n' "$TRIAL" | sudo -n tee /etc/systemd/system/lemon-galaxy-worker.service.d/e2e.conf >/dev/null
      sudo -n systemctl daemon-reload
      sudo -n systemctl restart lemon-galaxy-worker
      ;;
    windows)
      "$BIN/lemon-galaxy-worker" --data-dir "$SVC" install-service >/dev/null || fail "安装服务失败"
      powershell -NoProfile -Command "New-ItemProperty -Path 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\LemonGalaxyWorker' -Name Environment -PropertyType MultiString -Value @('LEMON_GALAXY_UPDATE_TRIAL_SECS=$TRIAL','LEMON_GALAXY_UPDATE_HEALTHY_SECS=10') -Force | Out-Null"
      # 装了桌面端：安装目录里有界面程序（测试用一个别的程序代替，只核对文件被换掉）
      cp "$BIN/galaxy-dev.exe" "$INST/LemonGalaxyWorker.exe"
      mkdir -p "$INST/Resources/Locales"
      echo '{"v":"old"}' >"$INST/Resources/Locales/en.json"
      net stop LemonGalaxyWorker >/dev/null 2>&1 || true
      net start LemonGalaxyWorker >/dev/null || fail "启动服务失败"
      ;;
    loop)
      mkdir -p "$INST"
      cp "$BIN/lemon-galaxy-worker" "$INST/lemon-galaxy-worker"
      (
        while [ ! -f "$WORK/stop" ]; do
          LEMON_GALAXY_SUPERVISED=1 LEMON_GALAXY_CONTROL_SOCKET="$CTL" LEMON_GALAXY_UPDATE_TRIAL_SECS=$TRIAL LEMON_GALAXY_UPDATE_HEALTHY_SECS=10 \
            "$INST/lemon-galaxy-worker" --data-dir "$SVC" run >>"$WORK/svc.log" 2>&1 || true
          sleep 3
        done
      ) &
      PIDS+=($!)
      ;;
  esac
}

svc_uninstall() {
  case "$MODE" in
    systemd)
      sudo -n rm -rf /etc/systemd/system/lemon-galaxy-worker.service.d
      if [ "$DEB" = 1 ]; then
        sudo -n apt-get remove -y -qq lemon-galaxy-worker >/dev/null 2>&1 || sudo -n dpkg -r lemon-galaxy-worker >/dev/null 2>&1 || true
      else
        sudo -n "$BIN/lemon-galaxy-worker" uninstall-service >/dev/null 2>&1 || true
        sudo -n rm -f /usr/local/bin/lemon-galaxy-worker /usr/local/bin/lemon-galaxy-worker.old*
      fi
      sudo -n systemctl daemon-reload || true
      ;;
    windows)
      "$BIN/lemon-galaxy-worker" uninstall-service >/dev/null 2>&1 || true
      rm -rf "$INST" 2>/dev/null || true
      ;;
    loop)
      touch "$WORK/stop"
      pkill -f "$INST/lemon-galaxy-worker" 2>/dev/null || true
      ;;
  esac
}

cleanup() {
  svc_uninstall >/dev/null 2>&1 || true
  # E2E_KEEP=1：留下数据目录与日志，排查用
  if [ -n "${E2E_KEEP:-}" ]; then
    for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done
    echo "保留了 $WORK"
    return
  fi
  for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done
  pkill -f "__lg-update-guard" 2>/dev/null || sudo -n pkill -f "__lg-update-guard" 2>/dev/null || true
  sudo -n rm -rf "$WORK" 2>/dev/null || rm -rf "$WORK"
}
trap cleanup EXIT

ctl() { $SUDO "${TOOL[@]}" ctl "$CTL" "$1" 2>&1; }
state() { $SUDO cat "$SVC/update/state.json" 2>/dev/null | tr -d '\n ' || true; }

# 等服务以某个版本运行并连上服务器：wait_version <版本> <秒>
wait_version() {
  local want="$1" secs="$2" i out=""
  for i in $(seq 1 "$secs"); do
    out="$(ctl status 2>/dev/null || true)"
    if echo "$out" | grep -q "\"version\":\"$want\"" && echo "$out" | grep -q '"online":true'; then
      echo "服务以 $want 运行，已连上服务器（${i} 秒）"
      return 0
    fi
    sleep 1
  done
  echo "$out" | head -c 600
  fail "等了 $secs 秒，服务没有以 $want 连上服务器"
}

# 等更新记录到某个结论：wait_phase <phase> <秒>
wait_phase() {
  local want="$1" secs="$2" i
  for i in $(seq 1 "$secs"); do
    state | grep -q "\"phase\":\"$want\"" && { echo "更新记录：$want（${i} 秒）"; return 0; }
    sleep 1
  done
  state
  fail "等了 $secs 秒，更新记录没有到 $want"
}

# ---------------- 准备 ----------------
step "测试构建"
"$BIN/lemon-galaxy-worker" version | tee "$WORK/version.txt"
grep -q "测试构建" "$WORK/version.txt" || fail "$BIN 里不是测试构建（要 --features galaxy-proto/test-release-key）"
grep -q "^lemon-galaxy-worker $V0\$" "$WORK/version.txt" || fail "测试构建的版本要编成 ${V0}（LEMONGALAXY_BUILD_VERSION=$V0）"
ARCH="$(uname -m)"
case "$ARCH" in arm64 | aarch64) ARCH=aarch64 ;; x86_64 | amd64) ARCH=x86_64 ;; esac
[ -n "$DEB_ARCH_IN" ] && ARCH="$DEB_ARCH_IN"
if [ "$WIN" = 1 ]; then
  case "${PROCESSOR_ARCHITECTURE:-AMD64}" in ARM64) ARCH=aarch64; ZARCH=arm64 ;; *) ARCH=x86_64; ZARCH=x64 ;; esac
fi

step "启动服务器（端口 ${PORT}）"
"$BIN/lemon-galaxy-server" --data-dir "$WORK/srv" --port "$PORT" user create alice init-pass-123 --display-name Alice >/dev/null
"$BIN/lemon-galaxy-server" --data-dir "$WORK/srv" --port "$PORT" admin create e2eadmin admin-pass-123 >/dev/null
"$BIN/lemon-galaxy-server" --data-dir "$WORK/srv" --port "$PORT" >"$WORK/server.log" 2>&1 &
SPID=$!
PIDS+=($SPID)
for i in $(seq 1 50); do grep -q "客户端填写的服务器地址" "$WORK/server.log" 2>/dev/null && break; sleep 0.2; done
ADDR="$(grep -o '客户端填写的服务器地址：.*' "$WORK/server.log" | sed 's/.*：//' | sed 's/\x1b\[[0-9;]*m//g' | tr -d '\r')"
[ -n "$ADDR" ] || fail "服务器没有启动"
"$BIN/galaxy-dev" --data "$WORK/ctl" login "$ADDR" alice init-pass-123 alice-new-pass-456 2>/dev/null | tail -1

# 发布一个版本：release <版本> <故障 none|crash|offline|broken>
release() {
  local v="$1" fault="$2" d="$WORK/rel-$1" core files=()
  mkdir -p "$d"
  local marker="LG-E2E-FAULT:none_______________" want
  case "$fault" in
    crash) want="LG-E2E-FAULT:crash______________" ;;
    offline) want="LG-E2E-FAULT:offline____________" ;;
    *) want="$marker" ;;
  esac
  if [ "$WIN" = 1 ]; then core="lemon-galaxy-worker-$v-windows-$ARCH.exe"
  elif [ "$(uname -s)" = Darwin ]; then core="LemonGalaxyWorker-$v-macOS.dmg" # 开发机：服务器把 Mac 的被控端包归到 worker / macos
  else core="lemon-galaxy-worker-$v-linux-$ARCH"; fi
  "${TOOL[@]}" patch "$BIN/lemon-galaxy-worker$X" "$d/$core" "$V0=$v" "$marker=$want" >/dev/null
  if [ "$fault" = broken ]; then head -c 200000 "$d/$core" >"$d/x" && mv "$d/x" "$d/$core"; fi
  if [ "$(uname -s)" = Darwin ]; then codesign --force -s - "$d/$core" 2>/dev/null || true; fi
  files+=("$d/$core")
  if [ "$WIN" = 1 ]; then
    # 桌面端压缩包：同名文件夹里是界面（测试用 galaxy-dev 加一段标记代替）、被控服务、许可证、资源
    local top="$d/LemonGalaxyWorker-$v-windows-$ZARCH"
    mkdir -p "$top/Resources/Locales"
    cp "$BIN/galaxy-dev.exe" "$top/LemonGalaxyWorker.exe" && printf 'gui-%s' "$v" >>"$top/LemonGalaxyWorker.exe"
    cp "$d/$core" "$top/lemon-galaxy-worker.exe"
    echo "license $v" >"$top/LICENSE.txt"
    echo "{\"v\":\"$v\"}" >"$top/Resources/Locales/en.json"
    "${TOOL[@]}" zip "$d/LemonGalaxyWorker-$v-windows-$ZARCH.zip" "$top"
    files+=("$d/LemonGalaxyWorker-$v-windows-$ZARCH.zip")
  fi
  if [ "$DEB" = 1 ]; then
    "$HERE/package-linux.sh" "$ARCH" "$v" "$d/$core" "$GUI_BIN" "$d/pkg" >/dev/null
    for f in "$d"/pkg/*; do mv "$f" "$d/"; files+=("$d/$(basename "$f")"); done
  fi
  local names=()
  for f in "${files[@]}"; do names+=("$(basename "$f")"); done
  (cd "$d" && sha "${names[@]}" >SHA256SUMS.txt)
  (cd "$d" && LEMONGALAXY_RELEASE_SIGNING_KEY="$TEST_SEED" "$BIN/galaxy-dev" release-sign SHA256SUMS.txt >/dev/null) || fail "签名失败"
  "${TOOL[@]}" publish "127.0.0.1:$PORT" e2eadmin admin-pass-123 "$WORK/admin-totp.txt" "${files[@]}" || fail "发布 $v 失败"
}

# 立即更新：apply <期望出现在回复里的文字>
apply() {
  local out
  out="$(ctl apply)"
  echo "回复：$out"
  echo "$out" | grep -Eq -- "$1" || fail "立即更新的回复里没有「$1」"
}

# ---------------- 装好旧版本 ----------------
if [ "$DEB" = 1 ]; then
  step "装 $V0 的安装包"
  release "$V0" none
  sudo -n dpkg -i "$WORK/rel-$V0"/lemon-galaxy-worker_*.deb >/dev/null || fail "装 $V0 的安装包失败"
  systemctl is-active --quiet lemon-galaxy-worker || fail "装好后服务没有运行"
fi

step "被控端登录并装成服务（$MODE）"
$SUDO env LEMON_GALAXY_PASSWORD=alice-new-pass-456 "$BIN/lemon-galaxy-worker" --data-dir "$SVC" login --server "$ADDR" --user alice --system-user "$ME" >"$WORK/login.log" 2>&1 || fail "被控端登录失败"
svc_install
wait_version "$V0" 60

V1=9.8.7.6001
step "好版本 $V1：立即更新 → 换上 → 自己重启 → 连回服务器 → 确认"
release "$V1" none
apply "\"version\":\"$V1\""
wait_version "$V1" 180
wait_phase committed 90
out="$(to 90 "$BIN/galaxy-dev" --data "$WORK/ctl" exec "" "echo after-update" 2>&1 || true)"
echo "$out" | grep -q "after-update" || fail "更新后主控端连不上被控端：$out"
echo "主控端连得上更新后的被控端"
if [ "$MODE" = windows ]; then
  cmp "$INST/LemonGalaxyWorker.exe" "$WORK/rel-$V1/LemonGalaxyWorker-$V1-windows-$ZARCH/LemonGalaxyWorker.exe" || fail "界面程序没有换成新版本"
  grep -q "$V1" "$INST/Resources/Locales/en.json" || fail "资源没有换成新版本"
  grep -q "$V1" "$INST/LICENSE.txt" || fail "许可证没有换成新版本"
  echo "桌面端整套换好：界面、被控服务、资源、许可证"
fi
if [ "$DEB" = 1 ]; then
  [ "$(dpkg-query -W -f='${Version}' lemon-galaxy-worker)" = "$V1" ] || fail "包管理器里的版本不是 $V1"
  systemctl is-enabled --quiet lemon-galaxy-worker || fail "升级后服务的开机自启没了"
  echo "安装包升级到 $V1，开机自启保留"
fi
$SUDO test -d "$SVC/update/backup" && fail "确认后备份没有删"

if [ "$DEB" = 0 ]; then
  V2=9.8.7.6002
  step "起不来的版本 $V2（文件坏了）：装之前试跑发现，不换"
  release "$V2" broken
  apply "起不来|cannot start"
  sleep 3
  wait_version "$V1" 30
  state | grep -q '"phase":"committed"' || fail "试跑失败后更新记录不该变"
fi

V3=9.8.7.6003
step "起来就崩的版本 $V3：守护进程换回 $V1，记成坏版本"
release "$V3" crash
apply "\"version\":\"$V3\""
wait_phase rolled_back $((TRIAL + 150))
wait_version "$V1" 120
state | grep -q "\"bad\":\[[^]]*\"$V3\"" || fail "$V3 没有记成坏版本"
step "坏版本不再装"
apply '"restarting":false'
ctl check | grep -q '"latest"' && fail "检查更新还在报坏版本 $V3"
echo "坏版本 $V3 不再安装"

if [ "$MODE" = systemd ] && [ "$DEB" = 0 ]; then
  V4=9.8.7.6004
  step "一直连不上服务器的版本 $V4：试运行 $TRIAL 秒到期，换回 $V1"
  release "$V4" offline
  apply "\"version\":\"$V4\""
  wait_phase rolled_back $((TRIAL + 150))
  wait_version "$V1" 120
fi
out="$(to 90 "$BIN/galaxy-dev" --data "$WORK/ctl" exec "" "echo after-rollback" 2>&1 || true)"
echo "$out" | grep -q "after-rollback" || fail "换回后主控端连不上被控端：$out"

V5=9.8.7.6005
step "主控端远程升级被控端到 $V5（设备详情里的「升级被控端」，与立即更新同一条路）"
release "$V5" none
DEV="$($SUDO cat "$SVC/keys/device_id" | od -An -tx1 | tr -d ' \n')"
out="$(to 400 "$BIN/galaxy-dev" --data "$WORK/ctl" remote-update "${DEV:0:8}" 300 2>&1 || true)"
echo "$out" | grep -v -E 'INFO|DEBUG|WARN' | tail -3
echo "$out" | grep -q "已以新版本 $V5 重新上线" || fail "远程升级后没有以 $V5 重新上线"
wait_version "$V5" 60
wait_phase committed 90

echo
echo "自动更新端到端测试全部通过（$MODE$([ "$DEB" = 1 ] && echo "，安装包")）"
