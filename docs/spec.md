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
| `lemon-galaxy-worker-linux` | Linux 纯被控端，命令行工具 + 后台服务 | Rust，静态编译单文件（x86_64 / ARM64） |
| `lemon-galaxy-worker-macos` | macOS 纯被控端（小体积，菜单栏状态 + 登录） | Swift + Rust 内核（后续期） |
| `lemon-galaxy-worker-windows` | Windows 纯被控端（小体积，Win7 SP1 起） | Rust + Win32（后续期） |
| `lemon-galaxy-control-macos` | Mac 主控 + 被控 | SwiftUI + AppKit，macOS 13 起 |
| `lemon-galaxy-control-windows` | Windows 主控 + 被控 | WPF（.NET Framework 4.8，Win7 SP1 起）（后续期） |
| `lemon-galaxy-mobile-android` | 安卓主控（只能控制别人） | Kotlin + Jetpack Compose，Android 8.0 起 |
| `lemon-galaxy-mobile-ios` | iOS / iPadOS 主控 | SwiftUI（后续期） |
| `lemon-galaxy-mobile-harmonyos` | 鸿蒙主控 | ArkTS + NAPI（后续期） |

### 2.1 三类客户端

- **control**（Windows / macOS）：主控 + 被控。**同一时间只能登录一个账号**，主控和被控都用这个账号。
- **worker**（Windows / macOS / Linux）：纯被控，体积小。**可以同时登录多个账号**，出现在每个账号各自的设备列表里，能被这些账号分别控制。Linux 只有 worker，是命令行工具。
- **mobile**（安卓 / iOS / 鸿蒙）：纯主控，不能被控。

被控能力在所有端是同一套内核逻辑（`galaxy-host`）。control 与 worker 的区别只在「能登录几个账号」这条规则上。

### 2.2 账号

- 服务器多用户。**普通用户只能由管理员在 Web 后台创建，不能自助注册**。
- 管理员账号与普通用户分开：管理员只登录 Web 后台（密码 + 强制两步验证），普通用户只登录客户端。
- 管理员创建用户时设初始密码；用户第一次在任意客户端登录时必须改密码，之后服务器和管理员都不知道该用户的密码（OPAQUE，见 security.md）。

## 3. 功能清单

### 3.1 远程桌面（Windows / macOS 被控）

- 画面：最高 4K、帧率按硬件最高 144；H.264 / H.265 硬件编解码（AV1 视硬件）；真彩 4:4:4（视硬件）；码率 / 画质 / 帧率可调；按网络自适应。
- 多屏：单屏模式（顶部标签切换）/ 分屏模式（每个屏一个窗口，可拖到本地不同显示器）；小窗悬浮。
- 虚拟屏：被控端最多创建 3 个虚拟屏，无头主机也能用（Windows 10 1607 起用虚拟显示驱动；macOS 用系统虚拟显示接口）。
- 隐私屏：黑屏遮罩 / 关闭物理屏改走虚拟屏；macOS 可自定义遮挡图。
- 操作：鼠标模式 / 触控模式；移动端虚拟键盘与快捷键栏；外接键鼠。
- 文件传输：双向、批量、断点续传、不限大小。剪贴板同步：文字、图片、文件（复制粘贴文件）。
- 声音：远端声音传回，被控端本地可静音。
- 其他：被控端重启后自动重连、Ctrl+Alt+Del、锁屏、禁止被控端本地输入、文字聊天、会话录像、远程结束自动锁屏。
- 不做：游戏手柄、按键映射、无线副屏。

### 3.2 终端与文件（所有被控端）

- 终端：Linux / macOS（用户默认 shell）、Windows（PowerShell / cmd，Win7 用 winpty 兼容层）。
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
| Windows 7 SP1 / 8 / 8.1 / Server 2008 R2–2012 R2 | 是（worker） | 是（control, WPF） | 截屏用 GDI（7）或桌面复制（8+）；无虚拟屏；终端用 winpty |
| Windows 10 / 11 / Server 2016+ | 是 | 是 | 全功能；10 1607 起支持虚拟屏；ARM64 原生 |
| macOS 13+ | 是 | 是 | 首次需本人授权「屏幕录制」「辅助功能」；Apple 芯片 + Intel 通用 |
| Linux（x86_64 / ARM64，任意发行版） | 是（终端 / 文件 / 端口映射 / AI 命令） | 否 | 静态编译单文件；有 root 装系统服务，无 root 装用户级服务 |
| Android 8.0+ | 否 | 是（mobile） | |
| iOS / iPadOS | 否 | 是（mobile，后续期） | TestFlight 分发 |
| HarmonyOS NEXT | 否 | 是（mobile，后续期） | |

服务器：Linux x86_64 / ARM64 静态单文件（另提供 macOS / Windows 版便于试用）。

## 6. 发布

- 公开仓库 `LemonITCN/lemon-galaxy`，MIT 许可证。GitHub Actions 构建全部工程。
- 每次推送 main 且构建、测试全部通过后，自动发布 Release「LemonGalaxy <版本>.<构建序号>」并标为最新；推送 `v*` 标签按标签版本发布。
- 签名：
  - macOS：Developer ID 签名 + 公证（Secrets 未配置时退回临时签名）。
  - iOS：TestFlight。
  - Windows：申请 SignPath Foundation 开源签名，批下来之前不签名。
  - Android：LemonGalaxy 专用 keystore，存在仓库 Actions Secrets。
- 所有发布文件附 SHA-256 与 LemonGalaxy 发布签名（Ed25519）。客户端自动更新只从自己的服务器下载（服务器定期从 GitHub 拉取并校验签名，也可在后台手动上传），安装前验签。

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
  lemon-galaxy-worker-linux/       Linux worker（Rust）
  lemon-galaxy-control-macos/      Mac 主控 + 被控（SwiftUI）
  lemon-galaxy-mobile-android/     安卓主控（Kotlin + Compose）
  scripts/                         构建与发布脚本
  .github/workflows/               Actions
```
