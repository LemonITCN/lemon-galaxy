#!/usr/bin/env bash
# 打 Linux 被控端安装包（.deb 与 .rpm）：内核 + 图形界面 + systemd 系统服务 + 菜单图标 + 登录后托盘自启 + polkit 规则。
#   scripts/package-linux.sh <架构 x86_64|aarch64> <版本> <内核程序> <界面程序> <输出目录>
# 需要 dpkg-deb；有 rpmbuild 时同时出 .rpm。素材在 lemon-galaxy-worker-linux/packaging/。
set -euo pipefail
ARCH="$1"; VER="$2"; CORE="$3"; GUI="$4"; OUT="$5"
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PKG="$ROOT_DIR/lemon-galaxy-worker-linux/packaging"
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64) DEB_ARCH=arm64 ;;
  *) echo "不支持的架构 $ARCH" >&2; exit 2 ;;
esac
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
R="$STAGE/root"

install -Dm755 "$CORE" "$R/usr/bin/lemon-galaxy-worker"
install -Dm755 "$GUI" "$R/usr/bin/lemon-galaxy-worker-gui"
install -Dm644 "$PKG/cn.lemonit.galaxy.worker.desktop" "$R/usr/share/applications/cn.lemonit.galaxy.worker.desktop"
install -Dm644 "$PKG/lemon-galaxy-worker-gui.desktop" "$R/etc/xdg/autostart/lemon-galaxy-worker-gui.desktop"
install -Dm644 "$PKG/cn.lemonit.galaxy.worker.policy" "$R/usr/share/polkit-1/actions/cn.lemonit.galaxy.worker.policy"
if [ -d "$PKG/icons/hicolor" ]; then
  mkdir -p "$R/usr/share/icons"
  cp -r "$PKG/icons/hicolor" "$R/usr/share/icons/"
fi
[ -f "$PKG/icons/lemon-galaxy-worker.png" ] && install -Dm644 "$PKG/icons/lemon-galaxy-worker.png" "$R/usr/share/pixmaps/lemon-galaxy-worker.png"
# 系统服务：装好即启动；没登录账号时服务只开本机控制接口，等界面登录
cat >"$STAGE/unit" <<'EOF'
[Unit]
Description=LemonGalaxy worker
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/bin/lemon-galaxy-worker run --data-dir /var/lib/lemon-galaxy-worker
Environment=LEMON_GALAXY_SUPERVISED=1
Restart=always
RestartSec=3
KillMode=process

[Install]
WantedBy=multi-user.target
EOF
install -Dm644 "$STAGE/unit" "$R/usr/lib/systemd/system/lemon-galaxy-worker.service"
install -Dm644 "$ROOT_DIR/LICENSE" "$R/usr/share/doc/lemon-galaxy-worker/copyright"

# 装好后（deb 的 postinst、rpm 的 %post 共用；FIRST=1 表示首次安装，由各自的开头按参数算出）：
#   首次安装：开机自启并启动服务。
#   升级（含被控服务自动更新：更新守护进程在单独的 systemd 单元里调用 dpkg / rpm，不受这里重启服务的影响）：
#   不改用户设的开机自启；开着开机自启就重启成新版本（没在运行也启动），关了就只在它正在运行时重启。
POSTINST_BODY='mkdir -p /var/lib/lemon-galaxy-worker && chmod 700 /var/lib/lemon-galaxy-worker
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
  if [ -n "$FIRST" ]; then
    systemctl enable lemon-galaxy-worker.service >/dev/null 2>&1 || true
    systemctl restart lemon-galaxy-worker.service || true
  elif systemctl is-enabled --quiet lemon-galaxy-worker.service 2>/dev/null; then
    systemctl restart lemon-galaxy-worker.service || true
  else
    systemctl try-restart lemon-galaxy-worker.service || true
  fi
fi'
# deb：postinst configure <上次配置好的版本>，首次安装时没有第二个参数
POSTINST="#!/bin/sh
set -e
FIRST=
[ \"\$1\" = configure ] && [ -z \"\$2\" ] && FIRST=1
$POSTINST_BODY
exit 0"
PRERM='#!/bin/sh
set -e
# 升级时不停服务（新版装好后 postinst 会重启）；卸载时停掉并取消开机自启
case "$1" in remove|purge|0)
  if [ -d /run/systemd/system ]; then
    systemctl disable --now lemon-galaxy-worker.service >/dev/null 2>&1 || true
  fi ;;
esac
exit 0'
POSTRM='#!/bin/sh
[ -d /run/systemd/system ] && systemctl daemon-reload >/dev/null 2>&1 || true
exit 0'

# ---------------- .deb ----------------
D="$STAGE/deb"
cp -a "$R" "$D"
# Debian 系的 systemd 单元也认 /usr/lib/systemd/system（Debian 10 / Ubuntu 20.04 起）
mkdir -p "$D/DEBIAN"
SIZE="$(du -sk "$D" | cut -f1)"
cat >"$D/DEBIAN/control" <<EOF
Package: lemon-galaxy-worker
Version: $VER
Architecture: $DEB_ARCH
Maintainer: LemonITCN <lemonitcn@users.noreply.github.com>
Installed-Size: $SIZE
Depends: libc6 (>= 2.17), libgtk-3-0, librsvg2-common
Recommends: pkexec | policykit-1, xvfb
Suggests: xfce4, gnome-shell-extension-appindicator
Section: net
Priority: optional
Homepage: https://github.com/LemonITCN/lemon-galaxy
Description: LemonGalaxy remote control worker
 Lets authorised LemonGalaxy accounts control this computer: remote desktop
 (X11 and Wayland), terminal, files and AI assistance, peer to peer or via
 your own server. Includes a system service and a desktop app with tray icon.
EOF
printf '%s\n' "$POSTINST" >"$D/DEBIAN/postinst"
printf '%s\n' "$PRERM" >"$D/DEBIAN/prerm"
printf '%s\n' "$POSTRM" >"$D/DEBIAN/postrm"
chmod 755 "$D/DEBIAN/postinst" "$D/DEBIAN/prerm" "$D/DEBIAN/postrm"
DEB="$OUT/lemon-galaxy-worker_${VER}_${DEB_ARCH}.deb"
dpkg-deb --root-owner-group -Zxz --build "$D" "$DEB" >/dev/null
echo "$DEB"

# ---------------- .rpm ----------------
if command -v rpmbuild >/dev/null; then
  T="$STAGE/rpmtop"
  mkdir -p "$T"/{BUILD,RPMS,SOURCES,SPECS,SRPMS,BUILDROOT}
  FILES="$(cd "$R" && find . -type f | sed 's|^\.||' | sort)"
  cat >"$T/SPECS/w.spec" <<EOF
Name: lemon-galaxy-worker
Version: $(echo "$VER" | tr '-' '.')
Release: 1
Summary: LemonGalaxy remote control worker
License: MIT
URL: https://github.com/LemonITCN/lemon-galaxy
Requires: gtk3, librsvg2
Recommends: polkit, xorg-x11-server-Xvfb
AutoReqProv: no
%global __os_install_post %{nil}
%global debug_package %{nil}

%description
Lets authorised LemonGalaxy accounts control this computer: remote desktop
(X11 and Wayland), terminal, files and AI assistance. Includes a system
service and a desktop app with tray icon.

%install
cp -a "$R/." %{buildroot}/

%post
FIRST=
[ "\$1" = 1 ] && FIRST=1
$POSTINST_BODY

%preun
if [ "\$1" = 0 ] && [ -d /run/systemd/system ]; then systemctl disable --now lemon-galaxy-worker.service >/dev/null 2>&1 || true; fi

%postun
[ -d /run/systemd/system ] && systemctl daemon-reload >/dev/null 2>&1 || true

%files
$FILES
EOF
  rpmbuild --quiet -bb --target "$ARCH" --define "_topdir $T" "$T/SPECS/w.spec" >/dev/null
  RPM="$(find "$T/RPMS" -name '*.rpm' | head -1)"
  cp "$RPM" "$OUT/lemon-galaxy-worker-${VER}.${ARCH}.rpm"
  echo "$OUT/lemon-galaxy-worker-${VER}.${ARCH}.rpm"
fi
