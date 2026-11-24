# LemonGalaxy

自建的远程控制与主机管理：像 UU 远程一样不开端口就能远程桌面、远程终端和传文件，像 Termius 一样管理一堆服务器，再加一个能直接操作远程电脑的 AI 智能体。服务器、账号、数据都在你自己手里。

[下载最新版](https://github.com/LemonITCN/lemon-galaxy/releases/latest)

- 能打洞就点对点直连，打不通才经自己的服务器中转；同一局域网自动直连，服务器连不上时局域网内照样能用。
- 端到端加密：账号密钥只在你的设备上，服务器和管理员都看不到你的会话内容和加密库（SSH/RDP 主机、凭据）。
- 远程桌面（多显示器、隐私屏、剪贴板、远端声音、按网络自适应码率、会话录像）、远程终端（断线不丢会话）、文件管理（像 SFTP 一样，断点续传）、端口转发。
- 远程开机：设备离线时，由同一网络里在线的另一台设备代发唤醒包；被控端可设「远程结束后自动锁屏」。
- SSH 直连（可经某台设备或另一台 SSH 主机跳转）、RDP 主机（经设备转发，目标不用开放端口）。
- AI 智能体：在主控端对话，让大模型在远程设备上执行命令、读改文件、看屏幕点鼠标；分档权限，危险操作要你确认，被控端机主还能设上限。
- 自带 Web 管理后台：账号只能由管理员创建，管理设备、连接、审计日志、大模型供应商与用量、自动更新。

## 三类客户端

| 类型 | 作用 | 平台 |
|---|---|---|
| 主控端（control） | 控制别的设备，同时自己也能被控；只能登录一个账号 | macOS、Windows |
| 被控端（worker） | 只被控制，体积小；可以同时登录多个账号，出现在每个账号的设备列表里 | Linux（窗口 + 托盘，或服务器上的命令行）、Windows（系统服务）、macOS |
| 手机端（mobile） | 只控制，也能用 AI | 安卓、iOS |

## 支持的系统

| 组件 | 系统 | 架构 |
|---|---|---|
| 服务器 | Linux 任意发行版（静态单文件）；Windows 10 / 11 / Server 2016 起；macOS 13 起。都内嵌管理后台，可装成系统服务 | x86_64、ARM64（macOS 为 Apple 芯片、Intel 通用包） |
| Linux 被控端 | 桌面版 .deb / .rpm：Ubuntu 20.04、Debian 10、CentOS / RHEL 8 起；单文件：glibc 2.17 起（CentOS 7 起）；musl 精简版：任意发行版（只有终端与文件）。X11 与 Wayland（GNOME、KDE 等）都能远程桌面，没人登录时开虚拟桌面，NVIDIA / Intel / AMD 显卡硬件编码 | x86_64、ARM64 |
| Windows 被控端 | Windows 10 / 11 / Server 2016 起；另有兼容版：Windows 7 SP1 / 8 / 8.1（x64），Vista SP2 / 7 / 8 / 10（32 位），XP SP3 / Server 2003 SP2（32 位，实验；没有声音、隐私屏、虚拟屏，终端为按行发送的简化终端）。老系统只做被控 | x64、ARM64、x86（32 位） |
| Mac 客户端 | macOS 13 起 | Apple 芯片、Intel（通用包） |
| 安卓 | 8.0 起 | arm64、armv7、x86_64 |

## 快速开始

1. 服务器，任选一种：
   - Linux：解压后运行 `sudo ./lemon-galaxy-server install-service --public-host <你的域名或公网 IP>`，装成开机自启的 systemd 服务（程序在 `/usr/local/bin`，数据在 `/var/lib/lemon-galaxy-server`）。
   - Windows：解压 `lemon-galaxy-server-<版本>-windows-x86_64.zip`（ARM64 电脑用 aarch64），以管理员身份打开 PowerShell，在解压目录运行 `.\lemon-galaxy-server.exe install-service --public-host <你的域名或公网 IP>`。装成系统服务（开机自启、出错自动重启），程序在 `C:\Program Files\LemonGalaxy\Server`，数据在 `C:\ProgramData\LemonGalaxy\server`，并在 Windows 防火墙放行端口。
   - macOS：解压 `lemon-galaxy-server-<版本>-macos-universal.tar.gz`（Apple 芯片、Intel 通用），运行 `sudo ./lemon-galaxy-server install-service --public-host <你的域名或 IP>`，装成开机自启、退出自动拉起的 launchd 守护进程（`/Library/LaunchDaemons/cn.lemonit.galaxy.server.plist`），程序在 `/usr/local/lib/lemon-galaxy-server`（`/usr/local/bin` 下有同名链接），数据在 `/Library/Application Support/LemonGalaxy/server`；系统的应用程序防火墙打开时自动把程序加进允许列表。程序暂未公证：用浏览器下载的先运行 `xattr -d com.apple.quarantine lemon-galaxy-server` 去掉隔离标记（用 curl、scp 拿到的不用）。
   - 临时试用：直接运行 `./lemon-galaxy-server --public-host <...>`，数据在当前目录的 `lemon-galaxy-data`。

   **交互菜单**：在终端里不带任何参数运行 `lemon-galaxy-server`（Windows 上双击 exe）会进入菜单，方向键或数字选择：前台运行、安装为系统服务并开机自启、修改端口 / 对外地址 / 证书、启动 / 停止 / 重启 / 卸载服务、查看状态与最近日志、显示客户端地址与初始化网址、创建管理员 / 重置密码、创建 / 列出用户、检查更新与升级。需要 root 时菜单会经 sudo 运行（Windows 上提示以管理员身份重新打开）。菜单文字跟随系统语言（中文 / 英文），可用 `LEMON_GALAXY_LANG=en` 指定。脚本、systemd、launchd、Docker 里不带参数运行时照常直接运行服务器；在终端里想直接前台运行用 `lemon-galaxy-server run`。

   安装后会打印初始化网址和「客户端填写的服务器地址」（带证书指纹），之后随时可用 `lemon-galaxy-server status` 再看。云服务器的安全组要放行 TCP 与 UDP 7443 端口。
   改端口、对外地址或证书：带上新参数再运行一次 `install-service`（没写的参数沿用上次）；启停：`start-service` / `stop-service` / `restart-service`；卸载：`uninstall-service`（数据目录保留）；升级：`self-update`（验发布签名，装了服务会自动重启）。
2. 打开初始化网址创建管理员、绑定两步验证，在后台「用户」里创建账号。
3. Linux 被控端：
   - 有桌面的机器：装 `.deb`（`sudo apt install ./lemon-galaxy-worker_<版本>_amd64.deb`）或 `.rpm`（`sudo dnf install ./lemon-galaxy-worker-<版本>.x86_64.rpm`），从应用菜单打开「LemonGalaxy 被控端」登录。服务开机自启，托盘显示状态。
   - 服务器：单文件 `./lemon-galaxy-worker login`，再 `sudo ./lemon-galaxy-worker install-service`。想远程看桌面就装 Xvfb 和一个桌面环境（如 `sudo apt install xvfb xfce4`），连上时自动开虚拟桌面。
   - Ubuntu 22.04 起默认是 Wayland：GNOME 不用额外设置；KDE 等第一次连接要有人在被控机上点一次「允许」。Wayland 锁屏时系统禁止远程桌面：远程期间被控端会阻止自动锁屏；无人值守的电脑可在被控端设置里打开「远程连入时自动解锁」（`sudo lemon-galaxy-worker unlock-on-connect on`，远程结束后自动重新锁上）。没人登录时自动开虚拟桌面（有 Xvfb 用 Xvfb，没有就用系统自带的 GNOME 无头桌面）。GNOME 48 及更早的 GDM 登录界面可用 `sudo lemon-galaxy-worker login-screen x11 on` 改成 X11，远程就能看到。
4. Windows 被控端：以管理员身份打开 PowerShell，运行 `.\lemon-galaxy-worker.exe login`，再 `.\lemon-galaxy-worker.exe install-service`。服务以 SYSTEM 运行，锁屏和 UAC 界面也能远程操作。XP / Vista 没有 PowerShell 时用管理员身份的命令提示符，命令相同；老系统用哪个文件见 [docs/research/windows-xp-32bit.md](docs/research/windows-xp-32bit.md)。
5. 在 Mac、Windows 或手机上用同一个服务器地址和账号登录。

被控端命令一览：`lemon-galaxy-worker help`。

## 目录

| 目录 | 内容 |
|---|---|
| `docs/` | 契约：`spec.md` 总体方案、`protocol.md` 协议、`security.md` 安全设计、`ai-agent.md` AI 智能体、`api.md` 服务器接口、`ffi.md` 内核导出接口 |
| `lemon-galaxy-common-rust/` | 各端共用的 Rust 内核：协议、加密、网络（打洞 / 中转 / 局域网）、客户端、被控端、AI 智能体、SSH、导出给 Swift / Kotlin / C# 的接口 |
| `lemon-galaxy-server/` | 服务器（Rust）与 Web 管理后台（Vue 3，打进同一个可执行文件） |
| `lemon-galaxy-worker-linux/`、`lemon-galaxy-worker-windows/` | 被控端（Linux 含 `gui/` 图形界面，设计见 `docs/linux-desktop.md`） |
| `lemon-galaxy-control-macos/`、`lemon-galaxy-control-windows/` | 主控端（SwiftUI、WPF） |
| `lemon-galaxy-mobile-android/`、`lemon-galaxy-mobile-ios/` | 手机端（Kotlin + Compose、SwiftUI） |
| `scripts/` | 构建与测试脚本 |
| `tools/` | 生成 C# 绑定的工具 |

## 构建与测试

```bash
cargo test --workspace --exclude galaxy-ffi
```

```bash
cargo build -p lemon-galaxy-server -p lemon-galaxy-worker-linux -p galaxy-dev && scripts/e2e.sh target/debug
```

端到端测试在本机起服务器、被控端和命令行主控端，验证登录、直连与强制 UDP / TCP 中转、命令执行、终端断线继承、远端声音（测试音）、文件传输和远程开机。
`scripts/server-service-test.sh <二进制目录>` 会真的把服务器装成系统服务、启停、改参数再卸载（Linux 与 macOS 要免密 sudo，Windows 要管理员），只在 CI 的一次性机器上跑。

- Mac：`scripts/build-ffi-apple.sh` 编内核，再 `lemon-galaxy-control-macos/scripts/build.sh [--dmg]`。
- 安卓：`cd lemon-galaxy-mobile-android && ./gradlew assembleRelease`（会自动编内核并生成绑定）。
- Windows：在 Mac 上可用 `scripts/check-windows.sh` 交叉检查（`TARGET=i686-pc-windows-gnullvm` 查 32 位）；正式包由 GitHub Actions 在 Windows 上用 MSVC 编译，老系统兼容版用 `scripts/build-windows-legacy.sh`（nightly 的 win7 目标 + YY-Thunks / VC-LTL5，并核对导入表）。

每次推送到 main，GitHub Actions 构建并测试全部工程，通过后自动发布 Release。发布文件附 SHA-256 清单与发布签名，客户端自动更新只安装签名有效的版本。

## 许可

MIT

## 代码签名

Windows 发布文件申请由 SignPath Foundation 免费签名，签名范围、审批人与隐私说明见 [代码签名策略](CODE_SIGNING_POLICY.md)。

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).
