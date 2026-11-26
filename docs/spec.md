# LemonGalaxy 总体方案（契约）

本文件是所有子工程的总契约。各端实现与本文件冲突时，以本文件为准；需要改动时先改本文件。
细节分册：

- [security.md](security.md)：安全设计（账号密钥、设备证书、端到端认证、威胁模型）
- [protocol.md](protocol.md)：连接建立（局域网/P2P/中转）与端到端会话协议
- [api.md](api.md)：服务器 HTTP / WebSocket 接口
- [ai-agent.md](ai-agent.md)：AI 智能体、大模型网关、权限档位
- [ffi.md](ffi.md)：Rust 内核给各端原生界面的接口

产品名在所有语言下都写作 **LemonGalaxy**（无空格）。包名、Bundle ID 前缀 `cn.lemonit.galaxy`。

## 1. 产品定位

像 UU 远程一样的远程控制系统，但服务器自己部署、账号由管理员开、数据全部在自己手里；
同时具备 Termius 式的主机管理（SSH / RDP 直连、SFTP、分组、主题），以及能直接对话控制远程设备的 AI 智能体。

## 2. 组成

| 子工程 | 作用 | 技术 |
|---|---|---|
| `lemon-galaxy-server` | 服务器：账号、设备、在线状态、信令、打洞协助、中转、端到端加密库同步、大模型网关、自动更新分发、Web 管理后台 | Rust（axum + rustls + quinn）、SQLite 单文件、Vue 3 + antdv-next 后台打进同一个可执行文件 |
| `lemon-galaxy-common-rust` | 公共内核：协议、加密、连接层、会话、终端/文件/命令执行、AI 智能体、SSH 客户端、FFI | Rust workspace |
| `lemon-galaxy-worker-linux` | Linux 纯被控端：CLI（单文件内核：命令行 + 系统服务 + 桌面会话画面代理 + 虚拟桌面）；桌面端（.deb / .rpm）另带图形界面（窗口 + 托盘），见 [linux-desktop.md](linux-desktop.md) | Rust；内核只依赖 glibc 2.17（另有 musl 静态精简版），界面 GTK 3；x86_64 / ARM64 |
| `lemon-galaxy-worker-macos` | macOS 纯被控端（桌面端）：菜单栏图标 + 设置窗口，有人值守 / 无人值守两种运行方式（3.0.1） | SwiftUI + AppKit + Rust 内核 |
| `lemon-galaxy-worker-windows` | Windows 纯被控端：CLI（单文件被控服务：系统服务 + 会话画面代理）；`gui/` 是桌面端的界面（窗口 + 托盘 + 自带安装与卸载），经本机控制接口遥控服务（3.0.2） | Rust + Win32；界面 WPF（.NET Framework 4.8，老系统兼容版 4.0，纯代码界面） |
| `lemon-galaxy-control-macos` | Mac 主控 + 被控 | SwiftUI + AppKit，macOS 13 起 |
| `lemon-galaxy-control-windows` | Windows 主控 + 被控 | WPF（.NET Framework 4.8，Win7 SP1 起）（后续期） |
| `lemon-galaxy-mobile-android` | 安卓主控（只能控制别人） | Kotlin + Jetpack Compose，Android 8.0 起 |
| `lemon-galaxy-mobile-ios` | iOS / iPadOS 主控 | SwiftUI（后续期） |
| `lemon-galaxy-mobile-harmonyos` | 鸿蒙主控 | ArkTS + NAPI（后续期） |

### 2.1 三类客户端

- **control**（Windows / macOS）：主控 + 被控。**同一时间只能登录一个账号**，主控和被控都用这个账号。
- **worker**（Windows / macOS / Linux）：纯被控，体积小。**可以同时登录多个账号**，出现在每个账号各自的设备列表里，能被这些账号分别控制。worker 的发布物统一叫法：带窗口与托盘（菜单栏）的叫「桌面端」，命令行单文件叫「CLI」，两者是同一个被控服务。Linux 只有 worker：有桌面的机器装桌面端 .deb / .rpm，服务器用 CLI；都能远程桌面（2026-11-24 起，见 [linux-desktop.md](linux-desktop.md)）。Windows worker 也是桌面端（zip 里的 LemonGalaxyWorker.exe 自带安装）与 CLI 两种，老系统兼容版同样两种（3.0.2）；macOS worker 只有桌面端，是菜单栏应用（3.0.1）。
- **mobile**（安卓 / iOS / 鸿蒙）：纯主控，不能被控。

被控能力在所有端是同一套内核逻辑（`galaxy-host`）。control 与 worker 的区别只在「能登录几个账号」这条规则上。

### 2.2 账号

- 服务器多用户。**普通用户只能由管理员在 Web 后台创建，不能自助注册**。
- 管理员账号与普通用户分开：管理员只登录 Web 后台（密码 + 强制两步验证），普通用户只登录客户端。
- 管理员创建用户时设初始密码；用户第一次在任意客户端登录时必须改密码，之后服务器和管理员都不知道该用户的密码（OPAQUE，见 security.md）。

## 3. 功能清单

### 3.0 Windows 被控端的进程结构

- 服务进程（LocalSystem，开机自启，失败自动重启）：联网、账号、终端、文件、命令执行，与 Linux 被控端共用 galaxy-worker 命令行与 galaxy-host 内核。
- 画面代理：服务把自己的令牌复制一份、放进当前控制台会话启动（`__lg-session-agent`），负责 DXGI 桌面复制截屏（不可用时退回 GDI）、openh264 编码、SendInput 键鼠、剪贴板；锁屏与 UAC 安全桌面也能看到和操作。控制台会话切换时自动换到新会话重启，进行中的画面续上。
- 以指定账号运行：终端、命令、文件子进程用该账号的令牌启动——已登录则取会话令牌，否则 S4U 登录（无需密码，只能访问本机资源）；管理员账号用提升后的完整令牌（与 Windows 自带 OpenSSH 一致）。
- 终端用 ConPTY（Windows 10 1809 / Server 2019 起），更老的系统退回按行发送的简化终端（输出按控制台代码页转成 UTF-8）；没有 PowerShell 的系统（XP、Vista）默认 shell 用 cmd。
- 协议里的路径一律用「/」：Windows 写成 `/C:/Users/x`，`/` 列出各个盘。
- 数据目录：管理员为 `%ProgramData%\LemonGalaxy\worker`（只给 SYSTEM 与管理员组），普通用户为 `%LOCALAPPDATA%\LemonGalaxy\worker`。
- 正式包要求 Windows 10 / Server 2016 起（Rust 标准库的要求）；另用 win7 编译目标出老系统兼容版（`scripts/build-windows-legacy.sh`，调研见 [research/windows-xp-32bit.md](research/windows-xp-32bit.md)），截屏退回 GDI：
  - `windows7-x86_64`：Windows 7 SP1 / 8 / 8.1 / Server 2008 R2–2012 R2；
  - `windows7-x86`：Vista SP2 / 7 / 8 / 8.1 / 10 的 32 位系统（YY-Thunks 补 Win7 起的接口）；
  - `windowsxp-x86`（实验）：XP SP3 / Server 2003 SP2 的 32 位系统（YY-Thunks 补 Vista 起的接口，VC-LTL5 改用系统自带的 msvcrt.dll）。XP 上没有显卡编码，远程发不了 Ctrl+Alt+Del；声音录声卡的「立体声混音」（没有 WASAPI，声卡驱动没提供立体声混音就传不了声音），隐私屏靠分层黑窗 + GDI 截屏不带 CAPTUREBLT，虚拟屏用自带的 XP 显示驱动（XPDM，见 `lemon-galaxy-worker-windows/driver/xp/`，嵌在 exe 里，装服务或 `install-display-driver` 时安装，XP 会弹「没有通过 Windows 徽标测试」要点「仍然继续」）。
  - 老系统只做被控（主控端要 .NET Framework 4.8）。CI 逐个核对导入表：每个导入的函数目标系统都要有。

### 3.0.1 macOS 被控端的无人值守

默认（无人值守关闭）菜单栏应用在登录用户的会话里自己跑内核，注销、重启后不在线。开启「无人值守」后（通用页，macOS 13 起；调研与测试计划见 [research/macos-unattended.md](research/macos-unattended.md)）：

- 后台服务（LaunchDaemon，root，开机启动、退出即重启）：`LemonGalaxyWorker --daemon`，应用用 `SMAppService.daemon` 登记包内 `Contents/Library/LaunchDaemons/cn.lemonit.galaxy.worker.daemon.plist`，管理员在「系统设置 → 通用 → 登录项」批准。负责联网、账号、终端、文件、命令（内核按账号对应的系统用户切换身份）。数据在 `/Library/Application Support/LemonGalaxy/worker`（0700，只有 root 能读），设备身份存在其下 `keys/`（文件，0600）。
- 会话代理（LaunchAgent）：`LemonGalaxyWorker --session-agent`，`LimitLoadToSessionType` 为 LoginWindow + Aqua，登录界面里以 root、每个已登录用户的图形会话里以该用户各跑一份；负责截屏编码、键鼠、剪贴板、声音、隐私屏。`SMAppService.agent` 只能登记到当前用户、也不能加载到登录界面，所以由后台服务写 `/Library/LaunchAgents/cn.lemonit.galaxy.worker.agent.plist`（程序指向包内主程序的绝对路径）。代理平时不建 NSApplication（否则以应用的 Bundle ID 登记到 LaunchServices，双击应用、登录项会打开代理而不是界面），只在开隐私屏时建，空闲后退出由 launchd 重启。
- 后台服务的平台层把请求转给「当前在屏幕上」的会话代理（代理报告自己是否在控制台；都不报时按控制台用户找，没人登录用登录界面的代理）；登录、注销、快速切换用户时换到新会话的代理，进行中的画面、声音、隐私屏续上。登录界面截屏：macOS 14.4 起先用 ScreenCaptureKit，不行用 CGDisplayStream；不提供隐私屏与声音。
- 菜单栏应用改当后台服务的界面（同样的页面，状态由后台服务推送）。三种角色是同一个签名的可执行文件，系统授权（屏幕录制、辅助功能）对它们同样生效。
- 本机通信：Unix 域套接字 `/var/run/cn.lemonit.galaxy.worker.sock`（0666），每条消息 `[u32 头长][u32 正文长][u32 附件长]` + JSON 头 + JSON 正文 + 原始附件（画面帧、声音）。对端身份取内核给的 uid 与审计令牌，代码签名必须满足后台服务自己的指定要求；会话代理报的 uid 必须与对端一致。界面的权限：管理员（root 或 admin 组）全部可做；坐在屏幕前的普通用户可查看、开关远程、断开会话、管理对应到自己的账号；其他用户拒绝。账号登录时对应到发起登录的用户（不会对应到 root）。临时协助的会话目前以后台服务身份执行，所以后台服务以 root 运行时只有管理员能开启或允许协助。
- 开启时把菜单栏应用原有的数据（`~/Library/Application Support/LemonGalaxyWorker/worker` 与本机安全存储里的设备身份）移交给后台服务，设备号不变，原目录改名 `worker.migrated` 留作备份（只有管理员能移交）；关闭时管理员把数据导回当前用户并让后台服务删除自己的数据，非管理员退回移交前的备份。两种运行方式切换时应用自动重新打开一次。应用更新后版本与后台服务不同时，后台服务与会话代理自动重启换成新程序。
- 开了文件保险箱的 Mac 重启后要先在开机界面解锁磁盘（或用 `fdesetup authrestart` 计划重启），之后才能远程控制登录界面。

菜单栏应用的功能对照 Linux 界面清单（[linux-desktop.md](linux-desktop.md) 第 9 节，2026-11-25 核对，有人值守与无人值守两种方式界面相同）：状态（设备 ID、公网 IP、服务器、账号、允许 / 禁止远程、逐个断开、全部断开、停止 AI）、账号（登录三步、退出、逐账号权限与系统用户、AI 上限、被控时提示）、临时协助（设备码与验证码、固定验证码、请求弹窗）、通用（开机启动 / 无人值守、远程结束后锁屏、无头兜底屏、通知、语言与外观）、事件日志、关于与检查更新、菜单栏图标与菜单都已有。这次补上两项：

- 已有账号时换服务器：登录表单的地址先锁着，点「更换服务器」才能改；提交时地址变了先在表单里确认（会退出本机已登录的账号），确认后逐个退出旧服务器上的账号，再用新服务器登录（与 Linux 的 `ConfirmServerChange` 一致；内核 `WorkerHost.login` 在有账号时拒绝换服务器，所以要先退出）。
- 有人连入的系统通知带「断开」「查看」按钮（断开那个会话 / 打开状态页），会话结束后收回这条通知；设置窗口在前台时也弹横幅。

不做的：Linux 专有的「远程连入时自动解锁」「登录界面使用 X11」；「自动更新」只自动检查、下载 DMG 由用户替换。

### 3.0.2 Windows 被控端桌面端与本机控制接口

2026-11-25 加。CLI（`lemon-galaxy-worker-<版本>-windows-<架构>.exe`）照旧发布；新增桌面端 `LemonGalaxyWorker-<版本>-windows-<x64|arm64>.zip`（老系统兼容版见本节末尾）：同一个被控服务 `lemon-galaxy-worker.exe` + 界面 `LemonGalaxyWorker.exe`。界面只是服务的「遥控器」，与 Linux 的 GTK 界面同一套逻辑、同一套消息（[linux-desktop.md](linux-desktop.md) 第 7、9 节），外观按 Windows 主控端（配色、矢量图标、控件与主控端共用源码），系统原生标题栏与托盘菜单。

**本机控制接口（命名管道）**：系统服务（SYSTEM）监听 `\\.\pipe\LemonGalaxyWorker.control`；普通用户前台 `run` 时是 `…control.<用户 SID>`；环境变量 `LEMON_GALAXY_CONTROL_SOCKET` 可改（服务和界面都认）。消息与 Linux 相同：一行一个 JSON，类型在 `galaxy-worker/src/control.rs`，界面（C#）按同一格式读写，自测核对与 Rust 测试里的字面量逐字一致。

- 管道的访问控制：属主 SYSTEM；SYSTEM、管理员组完全控制；Authenticated Users 只能读写、不能再建同名实例（第一个实例加 `FILE_FLAG_FIRST_PIPE_INSTANCE`，名字被占就不开控制接口）；前台运行的只给本人与 SYSTEM。不接受远程客户端：访问控制第一条拒绝网络登录（NU），Vista 起建管道时再加 `PIPE_REJECT_REMOTE_CLIENTS`（XP 不认这个标志，只靠前者）。
- 界面连上后先核对管道属主是 SYSTEM 或管理员组（前台运行的是本人），防止别的程序抢先建同名管道冒充服务骗密码；以「识别」级别连接（服务能认人，不能冒用身份）。
- 服务读到第一行后模拟对方取令牌：账号 SID 与「域\用户名」、管理员组是否生效（已提升）、是不是管理员组成员（没提升也算）、所在会话；对方进程号用 `GetNamedPipeClientProcessId`（Vista 起才有，运行时找，XP 上不填）。XP / 2003 没有会话 0 隔离，控制台用户就在会话 0，判断「坐在屏幕前」时会话 0 在 Vista 起不算、XP 上照常看连接状态（终端服务被停用、查不到时当作控制台）。

**鉴权**（Linux 第 7 节的表在 Windows 上的对应）：

| 操作 | 谁能做 |
|---|---|
| 看状态 | 任何本机用户 |
| 断开远程、停止 AI、禁止远程、开始 / 结束临时协助 | SYSTEM、已提升的管理员；或对方所在会话正连着（控制台或远程桌面里坐在屏幕前的人）。重新允许远程要管理员授权 |
| 看账号、设置、操作日志 | 管理员组成员（没提升也行）；账号绑定的系统用户 |
| 改设置、登录 / 退出账号、改权限、开机自启、更新 | SYSTEM、已提升的管理员；其他人经 UAC 授权（`ManageRight::Elevate`） |

UAC 授权（相当于 polkit 的 `auth_admin_keep`）：服务对要授权的操作回 `elevation_required`；界面向服务要一把一次性钥匙（`elevation_key`，2 分钟内有效），以管理员身份运行自己 `--elevate <钥匙> --pipe <管道>`（弹「用户账户控制」；标准用户输管理员密码），这个进程连上来发 `elevate`，服务核对它确实是 SYSTEM 或已提升的管理员后，把钥匙对应的那个连接提升 `ELEVATION_SECS`（300 秒），界面重试刚才的操作。授权过期时服务推一次状态。界面需要授权的按钮、开关旁画盾牌图标（Windows 惯例）。

**界面**（功能清单同 Linux 第 9 节，去掉 Linux 专有的两项）：

- 概览：设备名、系统、设备 ID（可复制）、状态；允许 / 禁止远程；服务器、公网 IP、账号数、服务版本；正在控制本机的会话（谁、哪台设备、直连还是中转、有没有 AI），逐个断开、全部断开、停止所有 AI；没账号、要重新登录、这份程序比已装的新时给提示条。
- 账号与权限：列表；登录向导（服务器地址、用户名、密码 → 两步验证码 → 首次登录设新密码 → 选系统用户，选 SYSTEM 二次确认；换服务器确认）；退出；权限（系统用户、看画面、控制、文件、终端、剪贴板、声音、被控时提示、AI 上限）。可选的系统用户取注册表 ProfileList（在这台电脑上登录过的本机与域账号）加 SYSTEM，默认是界面那头的人自己。
- 临时协助：开始 / 结束，设备码与验证码（可复制），固定验证码；有人请求时弹置顶的小窗口「允许 / 拒绝」（主窗口藏在托盘里也看得到）并发系统通知。
- 设置：开机自动启动被控服务（服务启动类型自动 / 手动，`sc config`）、登录后在托盘显示图标、远程结束后自动锁屏、没有显示器时自动加虚拟屏（`headless_display`，开关 + 分辨率）、自动更新被控服务 + 检查更新 / 立即更新；我的偏好（有人连入时通知、语言、外观）。
- 事件日志（可只看警告）、关于（界面与服务版本、运行方式、数据目录、控制接口、程序位置、系统、卸载）。
- 托盘：状态图标（标志右下角的点：绿在线、橙被控、红禁止或服务没运行，没连上服务整体变灰），提示文字写谁在控制；右键是系统原生菜单：逐个断开、全部断开、停止 AI、允许远程（打勾）、临时协助（开着时显示设备码与验证码，点一下复制）、打开主界面、退出界面。单击 / 双击打开主窗口。关窗口只藏到托盘（第一次提示一下），注销、关机时直接关。
- Linux 专有、Windows 不显示：远程连入时自动解锁（Windows 的锁屏界面本来就能远程看到和操作）、登录界面使用 X11。

**两种「开机自启」分开**：被控服务是 Windows 服务（`LemonGalaxyWorker`，开机就运行、没人登录也能被控制），设置里「开机自动启动被控服务」改它的启动类型，要管理员；界面是每个用户登录后自己的托盘程序：安装时写 `HKLM\…\Run`（所有用户登录后 `LemonGalaxyWorker.exe --tray`），每个用户在「登录后在托盘显示图标」里关掉只对自己生效（记在 `%LOCALAPPDATA%\LemonGalaxy\worker-ui\settings.json`，`--tray` 启动时看到关了就直接退出）；没安装、直接从解压目录运行时写本用户的 `HKCU\…\Run`。

**安装与卸载**（界面自带，没有另外的安装程序）：

- 解压后双击 `LemonGalaxyWorker.exe`：服务没装时整页是安装卡片（安装位置、开机自动启动被控服务、所有用户登录后显示托盘），点「安装」弹 UAC，以管理员身份运行 `--install`：复制到 `%ProgramFiles%\LemonGalaxy\Worker`（与 `lemon-galaxy-worker install-service` 复制到的目录相同），运行那里的 `lemon-galaxy-worker.exe install-service` 建服务并启动，写托盘自启、开始菜单快捷方式（名字按安装时的界面语言）与「应用和功能」里的卸载入口（`--uninstall`）。装好后打开装好的那份（普通权限，`--replace` 接替同位置正在运行的旧实例），解压出来的那份退出。
- 已装旧版本时同一张卡片变成「更新」：先停服务，正在运行的文件（旧界面、会话代理）先改名 `*.old*` 再复制，下次启动时删掉；账号与设置在 `%ProgramData%\LemonGalaxy\worker`，不动。服务停着时显示「启动服务」，服务在跑却连不上（没有本机控制接口的老版本）时提示用新包更新。
- 卸载：「关于」页或「设置 → 应用」。停止并删除服务（`uninstall-service`），关掉所有用户正开着的界面，删 Run 项、快捷方式、卸载入口，延时删程序目录；账号与设置保留。
- 单实例按程序位置区分（解压目录里的新版本与已装的旧版本可以同时开）；再开一次已在运行的那份就把窗口拉到前面。
- 自动更新只换被控服务（桌面端里的被控服务就是同一版本的 CLI，按 CLI 的发布文件更新），界面不随之更新，下次用新包更新；桌面端的 zip 不参与自动更新分类。

**发布与测试**：CI 任务「Windows 被控端桌面端」用 `windows` 任务编好的被控服务，`lemon-galaxy-worker-windows/gui/scripts/build.sh` 编界面（AnyCPU，x64 与 ARM64 共用）、打两个 zip；自测（`--selftest`）核对请求格式与 Rust 一致、读样例状态，离屏渲染各页（浅色 / 深色 × 中文 / 英文）与托盘图标，核对中英文语言包；`--live` 再前台起一个真的被控服务（临时数据目录与临时管道），界面连上去握手、读状态与设置、改一项设置再读回、拿授权钥匙、读系统用户与账号。Rust 侧 `galaxy-worker` 的 Windows 单元测试走真管道：认出管道那头是自己、同名管道不让第二个实例抢、关闭后管道消失。

**老系统兼容版桌面端**（2026-11-25 加）：`LemonGalaxyWorker-<版本>-windows7-x64.zip`（Windows 7 SP1 / 8 / 8.1 / Server 2008 R2–2012 R2 x64）、`-windows7-x86.zip`（Vista SP2 / 7 / 8 / 8.1 / 10 32 位、Server 2008）、`-windowsxp-x86.zip`（XP SP3 / Server 2003 SP2 32 位，实验），里面的被控服务是对应的兼容版 CLI。

- 老系统兼容版（windows7-x64 / windows7-x86 / windowsxp-x86）的桌面端是原生界面（`lemon-galaxy-worker-windows/gui-native`，Rust + Win32，与兼容版被控端一起用 win7 目标编，XP 版子系统 5.01），不需要 .NET：XP / Vista / 7 默认都没有 .NET 4.0，用户不该先去装运行库。功能与参数对齐 WPF 版（概览、账号与登录、临时协助、设置、事件日志、关于与卸载、托盘、单实例、安装），没有浅色 / 深色切换（跟随系统配色）和离屏自测。Windows 10 起的包仍是 WPF（.NET Framework 4.8，Windows 10 1903 起自带）。
- 4.0 的类库没有 async/await 要的类型和值元组：`gui/App/Net40.cs` 只在 `NET40` 时补最小实现（await 有界面线程的同步上下文就回界面线程、失败时抛出里面的那个异常，与 4.5 起一致）；`Task.Run`、`Task.FromResult`、Unix 时间换算、`Array.Empty`、`StreamReader` 的 `leaveOpen` 等 4.5 / 4.6 起的接口改成两份都有的写法（`Bg`、`Clock`）。只装了 4.0 的系统上没人看的失败任务会让进程退出，启动时登记 `UnobservedTaskException` 记日志。
- 清单用 `app.legacy.manifest`：多报 Vista，DPI 只按系统缩放（WPF 4.0 不会按显示器换 DPI）。共用控件里的 `WindowChrome`（4.5 起）在 4.0 版本是什么都不做的占位。链接进来的主控端源码（配色、图标、控件、语言包底座）也按 4.0 编，改那几个文件时别用 4.5 起才有的接口。
- 被控服务（XP 兼容版也一样）带本机控制接口：管道拒绝远程客户端靠访问控制、取对方进程号运行时找、XP 的控制台会话 0 算坐在屏幕前（见上）。导入表照旧由 `scripts/check-legacy-imports.py` 核对。
- CI 任务「Windows 被控端桌面端（老系统兼容版）」在 Windows Server 2025 上对三个包各跑一遍界面自测（含 await 行为核对）与 `--live`（真的兼容版被控服务），与兼容版 CLI 一样先不挡发布。这验证的是同一份程序在新系统上的行为：界面跑在 Server 2025 自带的 .NET 4.8 运行时上（用的仍是本程序补的 await 实现），4.0 运行时本身的差异（比如 WPF 4.0 在 XP 上走软件渲染）与 XP / Vista / 7 上的安装、托盘、授权都要实机看，还没跑过。

### 3.1 远程桌面（Windows / macOS 被控）

- 画面：最高 4K、帧率按硬件最高 144；H.264 / H.265 硬件编解码（AV1 视硬件）；真彩 4:4:4（视硬件）；码率 / 画质 / 帧率可调；按网络自适应。
- 多屏：单屏模式（顶部标签切换）/ 分屏模式（每个屏一个窗口，可拖到本地不同显示器）；小窗悬浮。
- 虚拟屏：被控端最多创建 3 个虚拟屏，无头主机也能用（Windows 10 1607 起用虚拟显示驱动；XP / Server 2003 用自带的 XPDM 驱动，按系统列出的显示设备数，目前 2 块；macOS 用系统虚拟显示接口）。
- 隐私屏：黑屏遮罩 / 关闭物理屏改走虚拟屏；macOS 可自定义遮挡图。Windows 10 2004 起黑窗用 WDA_EXCLUDEFROMCAPTURE 排除在截屏外；XP / Vista / 7 用分层黑窗、开着时 GDI 截屏不带 CAPTUREBLT（Vista / 7 先关 Aero 桌面合成；代价是别的程序的分层窗口远端也看不到）；Windows 8 到 10 1909 不支持。
- 操作：鼠标模式 / 触控模式；移动端虚拟键盘与快捷键栏；外接键鼠。
- 文件传输：双向、批量、断点续传、不限大小。剪贴板同步：文字、图片、文件（复制粘贴文件）。
- 声音：远端声音传回，被控端本地可静音。
- 其他：被控端重启后自动重连、Ctrl+Alt+Del、锁屏、禁止被控端本地输入、文字聊天、会话录像、远程结束自动锁屏。
- 不做：游戏手柄、按键映射、无线副屏。

### 3.2 终端与文件（所有被控端）

- 终端：Linux / macOS（用户默认 shell）、Windows（PowerShell / cmd；Windows 10 1809 之前的系统没有 ConPTY，是按行发送的简化终端）。
  - 多会话并存；断线后会话保留在被控端，重连接续；全屏程序（vim / htop / TUI）正常；移动端输入框 + 语音输入。
- 文件管理（Termius SFTP 式）：双栏（本地 ↔ 远端、远端 ↔ 远端）、拖拽、传输队列、断点续传、改权限、新建 / 重命名 / 删除、在线编辑。
  走 LemonGalaxy 自己的端到端加密通道，不依赖 SSH，因此 Windows / macOS 被控端同样可用。
- 端口映射：把被控端能访问到的 TCP 端口映射到主控端本机（访问内网网页、数据库、RDP 等）。
- 远程开机：由同一局域网内任意一台在线被控端代发唤醒包（WOL）。

### 3.3 直连主机（不装 worker）

- SSH 直连：密码 / 私钥 / 键盘交互认证；跳板机；本地 / 远程 / 动态端口转发；SFTP。
- RDP 直连：Windows 主控嵌入系统自带远程桌面控件（效果同 mstsc）；其他平台用 Rust 实现的 RDP 客户端（后续期）。
- 可以「经由某台 worker 跳转」连接它所在内网里的 SSH / RDP 主机。
- 主机地址、账号、密码、私钥全部**端到端加密后存在服务器**，同账号任意一端登录即可使用，服务器看不到明文。

### 3.4 设备列表与界面

- 设备、SSH 主机、RDP 主机放在同一棵「文件夹」树里，可多级分组、拖拽排序、标签、搜索、收藏、别名、备注。
  文件夹结构、别名、备注属于端到端加密库（vault），服务器看不到。
- 界面风格参照 Termius：界面主题（深色 / 浅色 / 跟随系统 + 若干配色）可换；终端配色主题（Dracula、Nord、Solarized、Gruvbox、Monokai、One Dark、Tomorrow 等）、字体、字号可换。
- 多语言：本期中文、英文；多语言底座按「语言包文件」组织，新增语言只加文件。

### 3.5 AI 智能体

见 [ai-agent.md](ai-agent.md)。要点：

- 服务器可配置多家大模型供应商（首期 OpenCode Go），按用户开通供应商与模型，可对某些用户整体关闭 AI。
- control 与 mobile 都有智能体界面，直接对话控制远程设备：Linux 像直连 SSH 一样执行命令；Windows / macOS 图形界面由模型自己截图识屏、点击、输入（远程版 computer use）。
- 权限档位：只读 / 每步确认 / 危险操作确认（默认）/ 完全信任（可限时）+ 自定义规则 + 绝对禁止清单；实际权限取界面档位、管理员上限、被控端机主上限三者最严。
- AI 决策循环只在主控端运行，服务器只当大模型网关。服务器只存用量，不存对话。

### 3.6 临时协助

被控端不登录账号时显示「设备码 + 验证码」；同一服务器上已登录的用户输入两个码即可发起连接，被控端本人点「允许」后才建立。
验证码只显示在被控端屏幕上、不经过服务器，作为端到端口令认证的口令（见 security.md），服务器无法冒充。

### 3.7 服务器管理后台

用户管理（创建 / 禁用 / 重置密码 / 强制两步验证 / AI 开关与额度 / 开通模型）、设备管理与吊销、在线状态与当前连接、
会话审计日志、中转流量统计与限速、大模型供应商管理、AI 用量统计、自动更新包管理、系统设置、管理员管理。

## 4. 连接方式总览

细节见 [protocol.md](protocol.md)。

- 所有端与服务器保持一条 TLS 长连接（WebSocket），用于在线状态与信令。
- 发起连接时同时尝试所有路径：**局域网 > 公网直连（IPv6 / IPv4 打洞）> 服务器中转（UDP，再退 TCP）**。先通先用，后台持续探测，找到更优路径无缝切换，不断线。
- 局域网检测三路并行：组播 / 广播查询、双方交换全部网卡地址、服务器提示「两端公网 IP 相同」。
- 服务器不可达时，局域网内已登录过的设备凭本地缓存的账号公钥互认，照样能连（AI 除外）。
- 两端之间是 QUIC；所有业务流量端到端加密认证，中转和服务器只看到密文。

## 5. 平台兼容

| 平台 | 被控 | 主控 | 备注 |
|---|---|---|---|
| Windows XP SP3 / Server 2003 SP2（32 位） | 是（worker，实验） | 否 | GDI 截屏 + 软件编码；声音要声卡有「立体声混音」；隐私屏；虚拟屏（自带 XP 驱动）；简化终端；CPU 要支持 SSE2 |
| Windows Vista SP2（32 位）、Windows 7 / 8 / 8.1 / 10（32 位） | 是（worker） | 否 | 截屏用 GDI（Vista / 7）或桌面复制（8+）；无虚拟屏；隐私屏 Vista / 7 与 10 2004 起有；简化终端（10 1809 起 ConPTY） |
| Windows 7 SP1 / 8 / 8.1 / Server 2008 R2–2012 R2 | 是（worker） | 是（control, WPF） | 截屏用 GDI（7）或桌面复制（8+）；无虚拟屏；隐私屏 7 有、8 / 8.1 没有；简化终端 |
| Windows 10 / 11 / Server 2016+ | 是 | 是 | 全功能；10 1607 起支持虚拟屏；ARM64 原生 |
| macOS 13+ | 是 | 是 | 首次需本人授权「屏幕录制」「辅助功能」；Apple 芯片 + Intel 通用 |
| Linux（x86_64 / ARM64，glibc 2.17 起；musl 精简版任意发行版） | 是（远程桌面、终端、文件、端口映射、AI） | 否 | X11 全支持（含登录界面）；GNOME Wayland 走 Mutter（无人值守），其他 Wayland 走桌面门户（首次本人点允许）；没人登录时开虚拟桌面；NVENC / VA-API 显卡编码；有 root 装系统服务，无 root 装用户级服务 |
| Android 8.0+ | 否 | 是（mobile） | |
| iOS / iPadOS | 否 | 是（mobile，后续期） | TestFlight 分发 |
| HarmonyOS NEXT | 否 | 是（mobile，后续期） | |

服务器：

| 平台 | 形态 | 备注 |
|---|---|---|
| Linux（x86_64 / ARM64，任意发行版） | 静态单文件 | `install-service` 装成 systemd 服务（root 运行，程序 `/usr/local/bin`，数据 `/var/lib/lemon-galaxy-server`） |
| Windows 10 / 11 / Server 2016+（x64 / ARM64） | 单个 exe（zip） | `install-service` 装成 LocalSystem 服务（开机自启、失败自动重启，程序 `%ProgramFiles%\LemonGalaxy\Server`，数据 `%ProgramData%\LemonGalaxy\server` 只给 SYSTEM 与管理员），自动加 Windows 防火墙入站规则；日志写数据目录 `logs/server.log` |
| macOS 13+（Apple 芯片 + Intel 通用） | 单文件（tar.gz，临时签名，暂未公证） | `install-service` 装成 launchd 守护进程（`/Library/LaunchDaemons/cn.lemonit.galaxy.server.plist`，root 运行，RunAtLoad + KeepAlive；程序 `/usr/local/lib/lemon-galaxy-server`，`/usr/local/bin` 下放同名链接；数据 `/Library/Application Support/LemonGalaxy/server`，0700）；应用程序防火墙打开时用 socketfilterfw 把程序加进允许列表，卸载时移除；日志写数据目录 `logs/server.log`。浏览器下载的要先 `xattr -d com.apple.quarantine` |

三个平台用法相同：`install-service` 的端口、对外地址、监听地址、证书参数写进服务，再次运行可修改（没写的沿用上次）；`start-service` / `stop-service` / `restart-service` 启停服务（停止后开机仍会自动启动）；`uninstall-service` 删服务、防火墙规则与程序，保留数据目录；`status` 显示服务状态、是否开机自启、客户端地址与初始化网址；`self-update` 从 GitHub 取最新发布（macOS 取通用包 `lemon-galaxy-server-<版本>-macos-universal.tar.gz`）、验发布签名后替换程序并重启服务（Windows 上运行中的程序先改名为 `.old`，下次启动时删掉）。前台运行时数据目录默认为当前目录下的 `lemon-galaxy-data`。

交互菜单：标准输入输出都是终端且不带任何参数时（含 Windows 双击打开的控制台）不直接运行，而是进菜单（方向键 / 数字选择，Esc 返回，q 退出；终端不认 ANSI 控制码时退回输入编号）。顶部显示版本、服务是否安装 / 运行 / 开机自启、数据目录、端口、对外地址、客户端地址（带指纹，取自运行中服务器写的 `run-info.json`）、还没有管理员时的初始化网址、管理员与用户数。菜单项：系统服务（安装并开机自启 / 修改参数 / 启动 / 停止 / 重启 / 卸载）、前台运行、状态与最近日志、客户端地址与初始化网址、管理员（创建 / 重置密码）、用户（创建 / 列出）、检查更新与升级、退出。服务相关操作调用同名子命令，非 root 时经 sudo；读不了系统服务数据目录时可「用 sudo 重新打开菜单」，Windows 上为「以管理员身份重新打开」（ShellExecute runas）。停止、卸载、重置密码前要确认。菜单文字跟随系统语言（`LEMON_GALAXY_LANG=zh|en` 可覆盖），子命令输出仍为中文。非终端（脚本、systemd、launchd、Docker 里没有 TTY 或在容器内）不带参数时仍直接前台运行。

## 6. 发布

- 公开仓库 `LemonITCN/lemon-galaxy`，MIT 许可证。GitHub Actions 构建全部工程。
- 每次推送 main 且构建、测试全部通过后，自动发布 Release「LemonGalaxy <版本>.<构建序号>」并标为最新；推送 `v*` 标签按标签版本发布。
- 签名：
  - macOS：LemonGalaxy 专用自签名代码签名证书（2046 年到期，存在仓库 Actions Secrets，私钥备份在维护者本机）。签名身份固定，系统记的「指定要求」（标识 + 证书）每个版本都一样，屏幕录制、辅助功能等授权升级后保留；从临时签名的旧版换过来时要再授权一次。未公证：首次打开要在「隐私与安全性」点「仍要打开」。以后换 Developer ID 时同名替换 Secrets 并配公证密钥即可。
  - iOS：TestFlight。
  - Windows：申请 SignPath Foundation 开源签名，批下来之前不签名。
  - Android：LemonGalaxy 专用 keystore，存在仓库 Actions Secrets。
- 所有发布文件附 SHA-256 与 LemonGalaxy 发布签名（Ed25519）。客户端自动更新只从自己的服务器下载（服务器定期从 GitHub 拉取并校验签名，也可在后台手动上传），安装前验签。
- 被控端的发布物统一叫「桌面端」（窗口 + 托盘 / 菜单栏）与「CLI」（命令行单文件），Release 说明的资源表格、快速开始、README、文档都用这两个词，不再写「带界面版」「桌面版」「命令行版」「单文件版」。Windows 发两种：桌面端 `LemonGalaxyWorker-<版本>-windows-<x64|arm64>.zip`（双击里面的 LemonGalaxyWorker.exe 安装）与 CLI `lemon-galaxy-worker-<版本>-windows-<架构>.exe`（服务器、批量部署、脚本用），老系统兼容版也各有桌面端与 CLI；Linux 发桌面端 .deb / .rpm 与 CLI（含 musl 精简版）；macOS 发桌面端。同一系统的桌面端与 CLI 是同一个被控服务，CLI 装的服务也能用桌面端管理，反之亦然；自动更新只换被控服务。

## 7. 分期

1. **第一期**：服务器 + Web 后台；公共内核（账号、设备、端到端加密库、连接层含局域网 / P2P / 中转、终端、文件、命令执行、AI 智能体）；Linux worker；Mac 主控 + 被控（含远程桌面画面和键鼠）；安卓主控（含 AI、远程桌面查看、终端、文件）。
2. Windows worker 与 Windows 主控（WPF），SSH / RDP 直连完善，虚拟屏、隐私屏、声音。
3. iOS、鸿蒙；macOS 纯被控小包；无人值守登录界面控制（macOS LaunchDaemon / Windows 安全桌面）；自动更新。
4. 会话录像、远程开机、端口映射图形界面、更多大模型供应商等。

## 8. 目录结构

```
lemon-galaxy/
  Cargo.toml                       Rust workspace 根（公共内核 + 服务器 + Linux worker）
  docs/                            本契约与分册
  lemon-galaxy-common-rust/
    galaxy-proto/                  线上消息与数据结构（服务器 / 客户端 / 两端会话共用）
    galaxy-crypto/                 账号密钥、设备证书、加密库、OPAQUE 封装
    galaxy-net/                    连接层：magicsock、候选地址、局域网发现、中转客户端
    galaxy-peer/                   QUIC 端点、端到端认证握手、会话流
    galaxy-client/                 服务器接口客户端：登录、设备、加密库、信令、连接编排
    galaxy-host/                   被控端会话服务：终端、文件、命令执行、显示与输入（平台钩子）
    galaxy-agent/                  AI 智能体：决策循环、工具、权限引擎、命令风险判定、敏感信息遮蔽
    galaxy-ffi/                    uniffi 导出给 Swift / Kotlin / C#
    galaxy-dev/                    开发与集成测试用命令行主控端
  lemon-galaxy-server/             服务器（Rust）+ web/（管理后台源码）
  lemon-galaxy-worker-linux/       Linux worker（Rust 内核 + gui/ GTK 界面）
  lemon-galaxy-worker-windows/     Windows worker（Rust 被控服务 + gui/ WPF 界面 + driver/ 虚拟屏驱动）
  lemon-galaxy-worker-macos/       macOS worker（菜单栏应用，SwiftUI）
  lemon-galaxy-control-macos/      Mac 主控 + 被控（SwiftUI）
  lemon-galaxy-mobile-android/     安卓主控（Kotlin + Compose）
  scripts/                         构建与发布脚本
  .github/workflows/               Actions
```
