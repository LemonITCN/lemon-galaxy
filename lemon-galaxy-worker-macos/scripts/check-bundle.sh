#!/bin/bash
# 检查打好的 LemonGalaxyWorker.app 里无人值守需要的东西（scripts/build.sh 打包后自动运行，也可单独运行）：
#   - Contents/Library/LaunchDaemons/cn.lemonit.galaxy.worker.daemon.plist（SMAppService.daemon 要求的位置）：
#     Label 与文件名一致、BundleProgram 指向包内主程序、参数含 --daemon、AssociatedBundleIdentifiers 是本应用、常驻
#   - Contents/Library/LaunchAgents/cn.lemonit.galaxy.worker.agent.plist（会话代理模板）：登录界面 + 用户会话、参数含 --session-agent
#   - 签名有效（--strict --deep），launchd 配置在签名封存的资源里，签名标识等于 Bundle ID，开了 Hardened Runtime
# 不安装、不登记、不运行应用。用法：scripts/check-bundle.sh <LemonGalaxyWorker.app>
set -euo pipefail
APP="${1:?用法：check-bundle.sh <LemonGalaxyWorker.app>}"
FAIL=0
ok() { echo "  [通过] $1"; }
bad() { echo "  [失败] $1" >&2; FAIL=1; }
json() { plutil -extract "$2" json -o - "$1" 2>/dev/null || echo "null"; }
raw() { plutil -extract "$2" raw -o - "$1" 2>/dev/null || echo ""; }

echo "检查包：$APP"
BID="$(raw "$APP/Contents/Info.plist" CFBundleIdentifier)"
EXE="$(raw "$APP/Contents/Info.plist" CFBundleExecutable)"
[ "$BID" = "cn.lemonit.galaxy.worker" ] && ok "Bundle ID $BID" || bad "Bundle ID 是 $BID"

D="$APP/Contents/Library/LaunchDaemons/cn.lemonit.galaxy.worker.daemon.plist"
if [ -f "$D" ] && plutil -lint -s "$D"; then
  ok "后台服务配置存在且格式正确"
  [ "$(raw "$D" Label)" = "cn.lemonit.galaxy.worker.daemon" ] && ok "后台服务 Label 与文件名一致" || bad "后台服务 Label"
  BP="$(raw "$D" BundleProgram)"
  [ "$BP" = "Contents/MacOS/$EXE" ] && [ -x "$APP/$BP" ] && ok "BundleProgram 指向包内主程序（$BP）" || bad "BundleProgram：$BP"
  json "$D" ProgramArguments | grep -q '"--daemon"' && ok "后台服务参数含 --daemon" || bad "后台服务参数"
  json "$D" AssociatedBundleIdentifiers | grep -q "\"$BID\"" && ok "后台服务关联本应用（登录项里显示应用名）" || bad "AssociatedBundleIdentifiers"
  [ "$(raw "$D" KeepAlive)" = "true" ] && [ "$(raw "$D" RunAtLoad)" = "true" ] && ok "后台服务开机启动、退出即重启" || bad "KeepAlive / RunAtLoad"
else
  bad "缺少后台服务配置 $D"
fi

A="$APP/Contents/Library/LaunchAgents/cn.lemonit.galaxy.worker.agent.plist"
if [ -f "$A" ] && plutil -lint -s "$A"; then
  ok "会话代理模板存在且格式正确"
  T="$(json "$A" LimitLoadToSessionType)"
  echo "$T" | grep -q '"LoginWindow"' && echo "$T" | grep -q '"Aqua"' && ok "会话代理加载到登录界面与用户会话" || bad "LimitLoadToSessionType：$T"
  json "$A" ProgramArguments | grep -q '"--session-agent"' && ok "会话代理参数含 --session-agent" || bad "会话代理参数"
else
  bad "缺少会话代理模板 $A"
fi

if codesign --verify --strict --deep "$APP" 2>/dev/null; then ok "签名有效（--strict --deep）"; else bad "签名无效"; fi
INFO="$(codesign -dv "$APP" 2>&1 || true)"
ID="$(printf '%s\n' "$INFO" | sed -n 's/^Identifier=//p')"
TEAM="$(printf '%s\n' "$INFO" | sed -n 's/^TeamIdentifier=//p')"
[ "$ID" = "$BID" ] && ok "签名标识 $ID（团队 ${TEAM:-无}）" || bad "签名标识 $ID"
printf '%s\n' "$INFO" | grep -q 'flags=.*runtime' && ok "Hardened Runtime" || bad "没有 Hardened Runtime"
if [ "$TEAM" = "not set" ] || [ -z "$TEAM" ]; then
  echo "  [提示] 临时签名：每次重新编译系统授权都会失效，SMAppService 登记后台服务也可能被拒绝；正式测试请用开发者证书签名"
fi
RES="$APP/Contents/_CodeSignature/CodeResources"
if [ -f "$RES" ] && plutil -p "$RES" | grep -q "Library/LaunchDaemons/cn.lemonit.galaxy.worker.daemon.plist"; then
  ok "launchd 配置在签名封存范围内（被改动后签名失效）"
else
  bad "launchd 配置没有被签名封存"
fi
ARCHS="$(lipo -archs "$APP/Contents/MacOS/$EXE" 2>/dev/null || true)"
case "$ARCHS" in *arm64*x86_64*|*x86_64*arm64*) ok "通用二进制（$ARCHS）" ;; *) echo "  [提示] 主程序架构：$ARCHS" ;; esac

if [ "$FAIL" = 0 ]; then echo "包检查通过"; else echo "包检查失败" >&2; exit 1; fi
