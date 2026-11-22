#!/usr/bin/env bash
# 服务器系统服务测试：install-service → 服务运行、HTTPS 接口能访问、日志与防火墙规则（Windows）→
# stop-service / start-service / restart-service → 只改端口再装一次（其他参数沿用）→
# uninstall-service → 服务、规则、程序都已清掉，数据目录保留。
# 会真的安装系统服务（Windows 需管理员，Linux 需免密 sudo 与 systemd，macOS 需免密 sudo，装成 launchd 守护进程），
# 只在 CI 的一次性机器上跑。
#   scripts/server-service-test.sh <二进制目录>
set -euo pipefail
WIN=0
MAC=0
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) WIN=1 ;; Darwin) MAC=1 ;; esac
BIN="$(cd "${1:-target/debug}" && pwd)"
if [ "$WIN" = 1 ]; then
  net session >/dev/null 2>&1 || { echo "没有管理员权限，跳过"; exit 0; }
  SRV="$BIN/lemon-galaxy-server.exe"
  SUDO=()
  DATA="$(cygpath -u "$(powershell -NoProfile -Command '$env:ProgramData' | tr -d '\r')")/LemonGalaxy/server"
  PROG="$(cygpath -u "$(powershell -NoProfile -Command '$env:ProgramFiles' | tr -d '\r')")/LemonGalaxy/Server/lemon-galaxy-server.exe"
elif [ "$MAC" = 1 ]; then
  sudo -n true 2>/dev/null || { echo "没有免密 sudo，跳过"; exit 0; }
  SRV="$BIN/lemon-galaxy-server"
  SUDO=(sudo -n)
  DATA="/Library/Application Support/LemonGalaxy/server"
  PROG=/usr/local/lib/lemon-galaxy-server/lemon-galaxy-server
  LABEL=cn.lemonit.galaxy.server
else
  sudo -n true 2>/dev/null || { echo "没有免密 sudo，跳过"; exit 0; }
  [ -d /run/systemd/system ] || { echo "没有 systemd，跳过"; exit 0; }
  SRV="$BIN/lemon-galaxy-server"
  SUDO=(sudo -n)
  DATA=/var/lib/lemon-galaxy-server
  PROG=/usr/local/bin/lemon-galaxy-server
fi
WORK="$(mktemp -d)"
PORT=$((20000 + RANDOM % 20000))
PORT2=$((PORT + 1))

ps1() { powershell -NoProfile -NonInteractive -Command "$1" | tr -d '\r'; }
show_logs() {
  echo "== 服务日志"
  "${SUDO[@]}" tail -30 "$DATA/logs/server.log" 2>/dev/null || true
  [ "$WIN" = 0 ] && [ "$MAC" = 0 ] && sudo -n journalctl -u lemon-galaxy-server --no-pager -n 30 2>/dev/null || true
  [ "$MAC" = 1 ] && sudo -n launchctl print "system/$LABEL" 2>/dev/null | head -40 || true
}
fail() { echo "失败：$*" >&2; show_logs; exit 1; }
cleanup() { "${SUDO[@]}" "$SRV" uninstall-service >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT
step() { echo; echo "==== $*"; }

service_running() {
  if [ "$WIN" = 1 ]; then [ "$(ps1 "(Get-Service LemonGalaxyServer -ErrorAction SilentlyContinue).Status")" = Running ]
  elif [ "$MAC" = 1 ]; then sudo -n launchctl print "system/$LABEL" 2>/dev/null | grep -q "state = running"
  else systemctl is-active --quiet lemon-galaxy-server; fi
}
service_exists() {
  if [ "$WIN" = 1 ]; then [ -n "$(ps1 "Get-Service LemonGalaxyServer -ErrorAction SilentlyContinue")" ]
  elif [ "$MAC" = 1 ]; then [ -f "/Library/LaunchDaemons/$LABEL.plist" ] || sudo -n launchctl print "system/$LABEL" >/dev/null 2>&1
  else [ -f /etc/systemd/system/lemon-galaxy-server.service ]; fi
}
fw_port() { # 防火墙规则放行的端口（没有规则为空）
  ps1 "(Get-NetFirewallRule -DisplayName 'LemonGalaxy Server $1' -ErrorAction SilentlyContinue | Get-NetFirewallPortFilter).LocalPort"
}
https_ok() { curl -sk --max-time 5 "https://127.0.0.1:$1/api/v1/info" | grep -q '"version"'; }

step "安装服务（端口 ${PORT}）"
"${SUDO[@]}" "$SRV" install-service --port "$PORT" --public-host 127.0.0.1 | tee "$WORK/install.txt" || fail "安装失败"
grep -q "客户端填写的服务器地址：127.0.0.1:$PORT#" "$WORK/install.txt" || fail "没有打印客户端地址"
grep -q "/setup?token=" "$WORK/install.txt" || fail "没有打印初始化网址"
service_running || fail "服务没有在运行"
https_ok "$PORT" || fail "HTTPS 接口访问不了"
"${SUDO[@]}" grep -q "客户端填写的服务器地址" "$DATA/logs/server.log" || fail "日志文件里没有启动信息"
[ -f "$PROG" ] || fail "程序没有复制到 $PROG"
if [ "$WIN" = 1 ]; then
  [ "$(fw_port TCP)" = "$PORT" ] || fail "防火墙 TCP 规则不对：$(fw_port TCP)"
  [ "$(fw_port UDP)" = "$PORT" ] || fail "防火墙 UDP 规则不对：$(fw_port UDP)"
  echo "防火墙规则已添加"
fi
"${SUDO[@]}" "$SRV" status | tee "$WORK/status.txt"
grep -q "运行中" "$WORK/status.txt" || fail "status 没有显示运行中"
grep -q "开机自启：是" "$WORK/status.txt" || fail "status 没有显示开机自启"
if [ "$MAC" = 1 ]; then
  [ "$(readlink /usr/local/bin/lemon-galaxy-server)" = "$PROG" ] || fail "/usr/local/bin 下没有指向 $PROG 的链接"
  plutil -lint "/Library/LaunchDaemons/$LABEL.plist" || fail "plist 格式不对"
fi

step "停止、启动、重启服务"
"${SUDO[@]}" "$SRV" stop-service || fail "stop-service 失败"
for i in $(seq 1 20); do https_ok "$PORT" || break; sleep 0.5; done
service_running && fail "stop-service 后服务还在运行"
https_ok "$PORT" && fail "stop-service 后还能访问"
ST="$("${SUDO[@]}" "$SRV" status 2>&1 || true)"
echo "$ST" | grep -q "已停止" || fail "status 没有显示已停止：$ST"
"${SUDO[@]}" "$SRV" start-service | tee "$WORK/start.txt" || fail "start-service 失败"
grep -q "客户端填写的服务器地址：127.0.0.1:$PORT#" "$WORK/start.txt" || fail "start-service 没有打印客户端地址"
service_running || fail "start-service 后服务没有运行"
https_ok "$PORT" || fail "start-service 后访问不了"
"${SUDO[@]}" "$SRV" restart-service | tee "$WORK/restart.txt" || fail "restart-service 失败"
grep -q "已重启服务" "$WORK/restart.txt" || fail "restart-service 没有提示已重启"
https_ok "$PORT" || fail "restart-service 后访问不了"

step "只改端口再装一次（${PORT2}，对外地址沿用）"
"${SUDO[@]}" "$SRV" install-service --port "$PORT2" | tee "$WORK/install2.txt" || fail "再次安装失败"
grep -q "客户端填写的服务器地址：127.0.0.1:$PORT2#" "$WORK/install2.txt" || fail "端口或对外地址没有更新/沿用"
service_running || fail "服务没有在运行"
https_ok "$PORT2" || fail "新端口访问不了"
https_ok "$PORT" && fail "旧端口还能访问"
if [ "$WIN" = 1 ]; then
  [ "$(fw_port TCP)" = "$PORT2" ] || fail "防火墙规则没有跟着改端口：$(fw_port TCP)"
fi

step "卸载服务"
"${SUDO[@]}" "$SRV" uninstall-service | tee "$WORK/uninstall.txt" || fail "卸载失败"
for i in $(seq 1 20); do service_exists || break; sleep 0.5; done
service_exists && fail "服务还在"
https_ok "$PORT2" && fail "卸载后服务器还在运行"
[ -e "$PROG" ] && fail "程序文件还在：$PROG"
if [ "$MAC" = 1 ]; then
  [ -L /usr/local/bin/lemon-galaxy-server ] && fail "/usr/local/bin 下的链接没有删掉"
fi
if [ "$WIN" = 1 ]; then
  [ -z "$(fw_port TCP)$(fw_port UDP)" ] || fail "防火墙规则没有删掉"
fi
"${SUDO[@]}" test -f "$DATA/server.db" || fail "数据目录不应删除"

echo
echo "服务器系统服务测试通过"
