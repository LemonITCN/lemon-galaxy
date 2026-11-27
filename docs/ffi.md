# 内核接口（galaxy-ffi）

Rust 内核通过 uniffi 0.32.0 导出给各端原生界面：Swift（macOS / iOS）、Kotlin（Android）、C#（Windows 主控端，用 tools/uniffi-bindgen-cs 生成，`scripts/gen-csharp-bindings.sh`）。三种生成器必须对应同一个 uniffi 版本，升级时一起升。
**以源码为准**：`lemon-galaxy-common-rust/galaxy-ffi/src/`（`types.rs` 记录与枚举、`callbacks.rs` 回调接口、`client.rs` / `session.rs` / `agent.rs` / `host.rs` 对象）。本文件是概览。

约定：

- 命名在 Swift / Kotlin 里自动转成驼峰（`open_terminal` → `openTerminal`）；记录是值类型，对象是引用类型。
- `async fn` 在 Swift 是 `async throws`、在 Kotlin 是 `suspend`。所有异步函数都在内核自己的多线程运行时执行，不占用宿主线程。
- **回调可能在任意内核线程触发**，界面层要自行切回主线程。回调实现里不要长时间阻塞。
- 错误统一是 `GalaxyError`（Kotlin 里是 `GalaxyException`）：`Network`、`Auth{code,message}`、`Permission`、`NotFound`、`Protocol`、`Cancelled`、`Other`。`message` 已本地化，直接显示。
- 对象不提供 `close()`（会与绑定自动生成的释放方法冲突）：会话用 `disconnect()`，端口映射用 `stop()`。

## 1. 初始化

```
core_version() -> String
core_init(config: CoreConfig)          每个进程一次
core_set_locale(locale: String)        运行时切换语言（影响内核返回的错误文字）
CoreConfig { data_dir, cache_dir, log_level, platform, device_kind, app_version, os_version, hostname, locale }
```

## 2. GalaxyClient

```
GalaxyClient(store: SecureStore, listener: ClientListener)
device_id() -> String
state() -> ClientState                   LoggedOut | Connecting | Online | Offline
current_user() -> UserInfo?              含 totp_enabled
server() -> ServerInfo?
restore() -> Bool                        启动时恢复登录（离线也能显示缓存的设备与加密库）
async login(server, username, password, totp?) -> LoginResult      Success | NeedTotp | MustChangePassword
async complete_first_login(new_password)
async change_password(old, new, totp?)
async logout()
async totp_setup() -> TotpSetup{secret, otpauth_uri} / totp_verify(code) / totp_disable(code)

devices() -> [DeviceInfo]                lan_seen 表示局域网内发现
async refresh_devices() / remove_device(id)
set_lan_discovery(active)                设备列表页打开期间设为 true（安卓同时持有 MulticastLock）

vault() -> [VaultEntry]
async vault_save(entry) -> VaultEntry    id 为空则新建
async vault_delete(id)

async start_port_forward(entry_id, SecretPrompt) -> PortForward   按加密库里的端口映射规则开启（经由设备或 SSH 主机）
can_wake(device_id) -> Bool             离线、能被控、上报过网卡地址时为真
async wake_device(device_id) -> [String] 远程开机：本机或同一网络（同公网 IP）里在线的设备代发唤醒包，返回代发设备名（空串为本机）
async connect(device_id) -> PeerSession
async connect_assist(device_code, verification_code) -> PeerSession   会等被控端本人点允许（最多约 2 分钟）

async ai_models() -> AiModels
async new_conversation(ConversationConfig, AgentListener) -> AgentConversation   targets 可以为空（聊天式）；选了的目标视同用户引用过，用到时才连接
async account_activity(limit) -> [ActivityItem]                               最近的登录与连接记录（新的在前）
list_conversations() -> [ConversationSummary]                                    本机保存（加密）
async open_conversation(id, ConversationConfig, AgentListener) -> AgentConversation
delete_conversation(id)

host() -> HostService?                   control 客户端登录上线后才有
```

回调 `ClientListener`：`on_state`、`on_devices_changed`、`on_vault_changed`、`on_session_revoked`（回登录页）。
回调 `SecureStore`：`get(key) -> bytes?`、`set(key, bytes)`、`remove(key)`。

加密库 `VaultEntry { id, folder_id?, sort, data: VaultData, updated_at }`，`VaultData`：
`Folder`、`DeviceMeta`、`SshHost`（`auth: SshAuth`）、`RdpHost`、`Credential`、`Snippet`、`KnownHost`、`Preferences{json}`、
`PortForward{label, via_device_id?, via_ssh_host_id?, local_port, remote_host, remote_port, auto_start, note}`（端口映射规则，经由设备与经由 SSH 主机恰好一个有值）。

## 3. PeerSession

```
info() -> PeerInfo { device_id, hostname, platform, os_version, arch, system_user, shell, permissions, capabilities, path }
path() -> PathInfo { kind: Connecting|Lan|Direct|RelayUdp|RelayTcp|Closed, rtt_ms, remote_addr }
is_closed() -> Bool
set_listener(PeerListener)               on_path_changed(path) / on_closed(reason)
disconnect()

async open_terminal(TerminalOptions{attach_session?, shell?, cols, rows, env:[EnvVar]}, TerminalSink) -> TerminalChannel
async list_terminals() -> [TerminalSessionInfo]
async kill_terminal(session_id)
files() -> FileClient
async exec(ExecRequest{command, shell?, cwd?, timeout_ms, max_output}) -> ExecResult{code?, stdout, stderr, timed_out, truncated, duration_ms}
async list_displays() -> [DisplayInfo]                                   DisplayInfo{id, name, width, height, scale, primary, is_virtual}；被控端没有亮着的显示器时会先自动加无头兜底屏（is_virtual 为真）
async open_display(DisplayOptions{display_id, max_fps, bitrate_kbps, codecs, view_only}, VideoSink) -> DisplayChannel
async screenshot(display_id, max_width) -> ImageData
async clipboard_get() -> ClipboardData? / clipboard_set(ClipboardData)      Text{text} | Image{image} | Files{paths}
async forward_port(local_port, remote_host, remote_port) -> PortForward     local_port() / stop()
async set_privacy(on)                                                     能力列表有 "privacy" 才支持（需要控制权限）
virtual_display_max() -> UInt32                                           主控最多能加几块虚拟屏（能力 "virtual_display" + "virtual_display_max=N"）；不支持或没有控制权限时为 0
async add_virtual_display(width, height, refresh) -> DisplayInfo          在被控端加一块虚拟屏（扩展桌面，refresh 为 0 用 60 Hz），返回的 id 直接用于 open_display；本会话断开时被控端自动删
async remove_virtual_display(display_id)                                  删掉本会话加的虚拟屏
async start_audio(volume) -> AudioPlayback                                能力列表有 "audio" 才支持
clipboard_push_files(local_paths, TransferListener) -> ClipboardTransfer   复制粘贴文件（本机→远端）：传到被控端暂存区再设成远端剪贴板
clipboard_pull_files(remote_paths, local_dir, TransferListener) -> ClipboardTransfer   远端剪贴板里的文件取回本机目录，完成后 paths() 是本机路径
async wake_on_lan(mac)
```

- `TerminalChannel`：`session_id()`、`title()`、`write(bytes)`、`resize(cols, rows)`、`detach()`（被控端保留会话）、`kill()`。写入按调用顺序送达。
  `TerminalSink`：`on_output(bytes)`、`on_exit(code?)`。终端仿真由界面层控件完成（SwiftTerm / Termux terminal-view）。
- `FileClient`：`home`、`list`、`stat`、`realpath`、`mkdir`、`rename`、`remove(path, recursive)`、`chmod`、`search(root, pattern, content)`、`read_text`、`write_text`、
  `download(remote, local, TransferListener) -> Transfer`、`upload(local, remote, TransferListener) -> Transfer`（都支持断点续传；只接受本机文件路径）。
  `Transfer`：`cancel()`、`pause()`、`resume()`。
- `DisplayChannel`：`send_input(InputEvent)`、`request_keyframe()`、`set_bitrate(kbps)`、`stop()`、
  `set_client_cursor(on)`（能力列表有 "client_cursor" 才调用：画面里不再画光标，界面自己画，被控端光标移动经 `on_cursor` 告知）；
  录像 `async start_recording(path, with_audio)`、`stop_recording() -> RecordingInfo{paths, duration_ms, bytes}`、`is_recording()`。
  录像在内核里把收到的 H.264 与 Opus 直接封装成分片 MP4（不重新编码，边录边写，异常退出已写部分仍可播放）；
  被控端分辨率变化时自动分段为 `名字-2.mp4` 等。Opus 音轨在 VLC、浏览器、ffmpeg 里能播，部分系统自带播放器可能只出画面。
  `VideoSink`：`on_config(codec, width, height)`（分辨率变化时会再次调用）、`on_frame(annexb, pts_us, keyframe)`、`on_cursor(x, y, visible)`（画面像素，光标热点；开了 set_client_cursor 才有）、`on_closed(reason)`。
- `AudioPlayback`：`set_volume(0..1)`、`set_muted(Bool)`、`is_active()`、`stop()`。内核收 Opus 数据报、解码（丢包补偿）后直接用本机默认输出设备播放
  （macOS / iOS CoreAudio、Windows WASAPI、安卓 AAudio），界面层只放开关与音量。iOS 播放前要把 AVAudioSession 设成 playback；
  Mac 应用要链接 AudioToolbox 并弱链接 CoreAudio（内核引用了 macOS 14.2 才有的接口，弱链接后 macOS 13 照样能启动）。
- `InputEvent`：`MouseMove{x,y}`、`MouseButton{button: PointerButton, down, x, y}`、`Wheel{dx,dy}`、`Key{hid_usage, down}`、`Text{text}`、`Touch{id, phase: TouchPhase, x, y}`、`SpecialCombo{combo: ComboKey}`。
  坐标为被控屏幕像素。`PointerButton`：Left/Right/Middle/Back/Forward。`ComboKey`：CtrlAltDel/LockScreen/AltTab/WinKey/TaskManager/ShowDesktop。

## 3.1 SshSession（SSH 直连）

- `GalaxyClient.connect_ssh(host_id, prompt: SecretPrompt) -> SshSession`：连接加密库里的 `ssh_host` 条目。
  - 认证：`Password`（没存密码时经 `prompt.ask` 问）、`PrivateKey`（引用 `credential` 条目里的私钥与口令，口令没存时问）、`Interactive`（键盘交互，逐个问题问）；`Agent` 暂不支持。
  - 跳转：`jump_device_id` 经该 LemonGalaxy 设备打开到目标的 TCP（目标在对方内网时用）；`jump_host_id` 经另一台 SSH 主机（direct-tcpip），最多 3 层。
  - 主机密钥：与加密库 `known_host` 条目比对；首次连接或不一致时调 `prompt.confirm_host_key(host, port, key_type, fingerprint, changed)`，确认后自动存回加密库。
- `SecretPrompt`（界面实现）：`ask(prompt, echo) -> Option<String>`、`confirm_host_key(...) -> bool`。内核在自己的线程上同步调用，界面可以阻塞等用户回答。
- `SshSession`：`info()`、`is_closed()`、`open_terminal(options, sink) -> TerminalChannel`（没有断线继承，detach 即结束）、`files() -> FileClient`（SFTP，接口与设备的完全相同，`content` 搜索只看 2 MB 以内文件）、`exec(request) -> ExecResult`、`forward_port(local_port, remote_host, remote_port) -> PortForward`、`disconnect()`。
- AI 对话的目标可以是 `SshHost`：用加密库里保存的凭据与已确认的主机密钥连接（对话里不能弹窗问密码，需要询问时报错提示先在主机列表里连一次）；SSH 主机没有图形界面，只有命令、文件、终端工具。

## 3.1.1 RDP 主机

- `open_rdp(host_id) -> RdpTarget`：加密库里的 `rdp_host`。设了 `jump_device_id` 时在本机 127.0.0.1 开临时端口，经那台 LemonGalaxy 设备转发到目标的 RDP 端口（目标在对方内网、不用开放端口）；否则直接给主机地址。
- 界面把 `host:port`、用户名（与可选的保存密码）交给系统的远程桌面客户端：Windows 用 mstsc（密码可经 cmdkey 临时存入凭据管理器，用完删除），macOS / iOS / 安卓用微软 Windows App（`rdp://full%20address=s:host:port&username=s:...`，密码由用户在那边输入）。
- `close_rdp(forward_id)`：关掉转发（经设备时用完要关）。iOS 上转发要应用留在前台才稳定。

## 3.2 TerminalScreen 与 H264Decoder（自绘界面用）

- `TerminalScreen(rows, cols, scrollback)`：`feed(bytes)`、`resize`、`state(scroll_offset)`（带颜色分段的行、光标、标题、备用屏、应用光标键、括号粘贴、鼠标上报模式、历史长度、响铃计数）、`text_between`、`text`。颜色：0 默认，`0x0100_00nn` 调色板，`0x02RRGGBB` 真彩色；flags 1 粗 2 斜 4 下划线 8 反色。
- `H264Decoder()`：`decode(annex_b) -> Option<DecodedFrame{width, height, bgra}>`。只在桌面平台编进去（手机端用系统硬件解码）。

## 4. AI

```
AiModels { enabled, max_mode: AiMode, items: [AiModel{provider_id, provider_name, id, name, vision, tools, context_length}] }
ConversationConfig { provider_id, model_id, targets: [AgentTarget{kind: Device|SshHost, id}], mode: AiMode, plan_first, trust_minutes?, title? }

AgentConversation:
  id() / title()
  send(text)                                 后台运行，进度经 AgentListener
  send_with_refs(text, [ContextRef])         带引用发送：ContextRef{kind, target_id, id, label}，kind 为
                                             Device | SshHost | File | Folder | Snippet | PortForward | Activity。
                                             设备与主机：本对话内可直接使用；文件（最多 256 KB）、目录列表、片段、端口映射、连接记录：展开后随消息给模型
  set_model(provider_id, model_id)           中途换模型（上下文保留）；model() -> ConversationModel
  decide(call_id, ApprovalDecision)          AllowOnce | AllowSimilar | Deny{reason?} | EditAndRun{command}
  answer(question_id, answer)
  provide_secret(request_id, secret)         内容直接送到被控端，不进模型上下文
  set_mode(mode, trust_minutes?)
  stop()
  set_foreground(Bool)                       移动端切后台时 false：停止当前任务
  messages() -> [AgentMessage]               User{text, refs} | Assistant{id, text, thinking} | Tool{call, result?}
```

`AgentEvent`：`AssistantDelta`、`AssistantDone`、`Thinking`、`ToolProposed{call: ToolCallView, needs_approval}`、`ToolStarted`、`ToolOutput`、
`ToolFinished{result: ToolResultView}`、`AskUser`、`SecretRequested`、`Usage`、`Status{running, step}`、`ModeChanged{mode, effective, until_ms?}`、`Error`。
`ToolCallView.risk: RiskLevel`（Read / Normal / Dangerous / Unknown / Forbidden）。

## 5. HostService（本机被控，control 客户端）

```
status() -> HostStatus { online, remote_enabled, device_id, public_ip, accounts: [HostAccount], sessions: [HostSessionInfo], assist: AssistInfo? }
set_platform(HostPlatform)
set_listener(HostListener)
accounts() -> [HostAccount{user_id, username, display_name, status, policy: HostPolicy}]
update_account_policy(user_id, HostPolicy{system_user, view, control, files, terminal, clipboard, audio, ai_cap, notify})
set_remote_enabled(Bool) / disconnect_all() / stop_ai()
set_lock_on_end(Bool) / lock_on_end()   远程看过桌面的会话都结束 10 秒后自动锁屏（默认关；断线重连不算结束）
set_headless_display(value) -> Bool / headless_display()   无头兜底屏（host.json 的 headless_display）："" 默认 1920x1080@60、"off" 关闭、"宽x高@刷新率" 自定义；写法不对返回 false
async start_assist() -> AssistInfo{device_code, code}
stop_assist() / set_assist_code(code?) / respond_assist(request_id, allow)
recent_events(limit) -> [HostEvent]
```

`HostPlatform`（界面层实现）：`list_displays`、`start_capture(display_id, CaptureOptions, EncodedFrameSink) -> CaptureStarted?`、`stop_capture`、`request_keyframe`、`set_bitrate`、
`inject(InputEvent)`、`screenshot(display_id, max_w, max_h) -> ImageData?`、`ui_tree(display_id, max_nodes) -> String?`、`clipboard_get/set`、
`set_privacy(on) -> Bool`、`set_cursor_in_video(display_id, show)`、`start_audio(AudioPcmSink) -> Bool`、`stop_audio()`、`capabilities() -> [String]`
（支持 `set_cursor_in_video` 的平台在 capabilities 里列 "client_cursor"；show 为 false 时画面里不画光标，并用 `EncodedFrameSink.set_cursor` 报光标位置）
（已获得系统授权的能力：`display`、`screenshot`、`input`、`ui_tree`、`clipboard`、`privacy`、`audio`）。
虚拟屏（归属、上限、无头兜底屏的时机由内核 galaxy-host 的 vdisplay.rs 管，平台层只管加删；都在内核的阻塞线程里调，可以阻塞几秒）：
`virtual_display_max() -> u32`（0 = 不支持）、`add_virtual_display(width, height, refresh) -> VirtualDisplayResult{display?, error?}`、
`remove_virtual_display(display_id) -> String?`（成功或已不存在为空，失败为原因）、`virtual_display_ids() -> [u32]`、`needs_headless_display() -> Bool`（没有亮着的显示器，不算自己的虚拟屏）。
macOS 用 CGVirtualDisplay 实现（`Platform/VirtualDisplays.swift`），安卓、iOS 不实现 HostPlatform；以后别的语言实现时不支持就返回 0 / 带原因的结果 / 空。
`EncodedFrameSink`（内核实现）：`push(annexb, pts_us, keyframe)`、`set_resolution(width, height)`、`set_cursor(x, y, visible)`（画面像素，变化时调用）。
`AudioPcmSink`（内核实现）：`push(pcm, sample_rate, channels)`，`pcm` 是交错排列的 32 位浮点样本（小端），格式随系统；内核统一转成 48 kHz 立体声、Opus 编码一次后分发给所有在听的会话。

`HostListener`：`on_status`、`on_assist_request(request_id, from_user, from_device)`（弹窗后调 `respond_assist`）、`on_session_started`、`on_session_ended`、`on_ai_activity`。

## 5.0 自动更新

- `check_update(component, platform, arch, current) -> Option<UpdateInfo>`：向自己的服务器查询；返回前已用内置发布公钥验过 SHA256SUMS 的签名，并确认文件在清单里。`component`：control / mobile / worker；`platform`：macos / android / windows / ios。
- `download_update(info, dest, listener)`：下载到本地文件并核对 SHA-256；`TransferListener.on_progress / on_finished`。安装由各端自己做（Mac 换 .app、安卓调安装器、Windows 换程序）。
  - Windows 主控端（`lemon-galaxy-control-windows/LemonGalaxy/App/UpdateInstall.cs`）：登录后后台每 6 小时查一次并预先下载，侧栏「设置」上出现标记，不点不装。设置 → 关于点「立即更新」后一路自动、不弹任何窗口：解压并核对新程序文件齐全 → 当前版本的 exe 复制到 `%LOCALAPPDATA%\LemonGalaxy\updates\helper-*` 当辅助程序（`--apply-update plan.json`）→ 主程序记下界面状态后退出 → 辅助程序等旧进程退出（20 秒不退就结束它），新文件先复制进程序目录下的 `.lg-update-*` 并核对 SHA-256，再逐个改名替换（正在运行的文件也能改名；带替换日志，断电后下次启动时自动换回）→ 启动新程序（`--update-token`），新程序界面画出来后置位事件 `Local\LemonGalaxy.UpdateStarted.<令牌>` 报到；启动后退出、两分钟没打开或替换出错都换回旧文件并打开旧程序（`--update-failed`，原因在 `updates\result.json`）。重新打开后回到原来的窗口位置、分区与设置页，设备终端接回原来的会话，远程桌面、文件标签与开着的端口映射重新连上（要问密码的 SSH 主机除外）。文本编辑窗口里有没保存的改动时不更新，先提示保存。
  - 程序目录当前用户不可写（如 Program Files）或在临时目录里时不提权，改装到 `%LOCALAPPDATA%\Programs\LemonGalaxy`：旧位置不动；用户的开始菜单、桌面、固定到任务栏里指向旧 exe 的快捷方式改到新位置（公共的改不了就留着），开始菜单里没有就建一个；新位置记在 `%LOCALAPPDATA%\LemonGalaxy\install.json`，从旧位置打开时转去新位置，不会两份同时在跑。设置与数据目录本来就在用户目录，不受影响。日志 `updates\update.log`。
- 服务器定期（默认 6 小时，后台可改）从 GitHub 拉最新发布，签名无效或没有签名的发布不镜像。
- 远程升级（主控端在设备详情里「升级被控端」，不用先开远程桌面）：
  - 设备列表 `DeviceInfo.update_version`：服务器按设备上报的更新通道「组件/平台/架构」（已登录的被控端每次连上信令都报，老版本按设备类型与系统推断）算出的可升级版本，没有新版本为空。
  - 主控一侧 `upgrade_device(device_id, UpgradeListener)`：立即返回；内核连上对方，能力列表里没有 `remote_update` 时报「对方版本太旧」，否则发 `MgmtReq::ApplyUpdate`（要控制权限、不能是临时协助），对方查到新版本后先回复版本号、再在后台自己走零交互升级。之后每 3 秒刷新设备列表，直到对方以新版本重新上线；断开后又以旧版本在线满一分钟算没装上，最多等 10 分钟。
    `UpgradeListener`：`on_started(version)` 对方开始升级；`on_offline()` 对方已断开、正在替换重启；`on_finished(version?, error?)` 成功时 version 是对方现在的版本（本来就是最新也走这里，error 为空），失败时 error 是原因（升级特有的几种按 `core_set_locale` 的语言）。回调在内核线程。
  - 被控一侧 `set_update_request_handler(UpdateRequestHandler)`：App 启动时登记一次，之后内核起的本机被控服务（control 的本机被控、`WorkerHost`）都报 `remote_update` 能力。收到请求时内核先用本机任一有效账号查新版本，有就调 `on_update_requested(UpdateInfo)`，App 必须立即返回、在后台走自己的零交互升级（Mac 两个应用接到 `AppUpdateEntry.startUpdateNow`）。没登记回调的程序不报这个能力。纯 Rust 被控端（Windows、Linux 的 lemon-galaxy-worker）不经 FFI，在服务模式下自己装上同一个入口，见 `docs/auto-update.md`。
  - 安卓主控端的界面（`lemon-galaxy-mobile-android` 的 `app/DeviceUpgrades.kt`、`ui/hosts/UpgradeViews.kt`）：能被控的设备（worker，或带本机被控的 macOS 客户端；不含本机、手机、Windows 客户端）`DeviceInfo.update_version` 不为空时，设备列表标题旁标「可升级」，升级进行中标「升级中」。点设备打开的详情面板（不用先连远程）显示当前版本、新版本与「升级被控端」，设备离线时不可点；点了直接调 `upgrade_device`，不再确认，面板不关、进度就地显示：正在请求升级 → 正在升级到 x（`on_started`）→ 对方已断开、正在替换并重启（`on_offline`）→ 已升级到 x / 已是最新版本 / 失败原因（`on_finished`），结束后刷新设备列表；失败后按钮变「重试」。进度按设备记在应用里，关掉面板再打开仍能看到，退出登录时清掉。
  - Mac 客户端与 iOS 客户端的界面（`lemon-galaxy-control-macos` 的 `UI/Hosts/DeviceUpgrade.swift`；`lemon-galaxy-mobile-ios` 的 `GalaxyKit/Logic/DeviceUpgrade.swift` 与 `GalaxyAccount.upgradeDevice`，界面在 `HostsView.swift`、`Editors.swift`）：判定同上，`update_version` 不为空时 Mac 侧栏与概览卡片、iOS 主机列表标题旁标「可升级」，进行中标「升级中」。设备详情（Mac 是连接方式下面的版本卡片，iOS 是设备详情弹出页里的一节）显示新版本、当前版本、i 说明与「升级被控端」，离线时按钮不可用并就地写明要在线；点了直接调 `upgrade_device`，不再确认，进度就地显示：正在升级到 x → 对方已断开、正在替换并重启 → 已升级到 x / 已是最新版本 / 失败原因（按钮变「重试」）。进度按设备记在应用里，离开详情再回来还在，退出登录清掉；结束后刷新设备列表，Mac 不在看这台设备的详情时另弹一条提示（iOS 一律弹）。模拟内核里构建服务器能走完整个升级、前台电脑离线且版本太旧（开机后再升级会失败）；Mac 自测截图 `*-19b` 到 `*-19i-hosts-upgrade-*`，iOS 由 `UpgradeTests` 覆盖。

## 5.1 WorkerHost（纯被控端，如 macOS 被控端）

- `WorkerHost(store)`：数据在 `CoreConfig.data_dir/worker`，设备身份存在传入的 `store`（与主控端分开）。`CoreConfig.device_kind` 用 worker。
- `device_id()`、`server()`、`start() -> Option<HostService>`（有账号时启动被控服务，应用启动时调一次）、`service()`。
- `login(server, username, password, totp, new_password, system_user) -> WorkerLoginResult`：`Success{user_id, username}` / `NeedTotp` / `MustChangePassword`（带上相应参数再调一次）。可以登录多个账号，但必须在同一台服务器上。
- `logout(user_id)`。账号权限、事件、临时协助等用返回的 `HostService`（与 control 的本机被控相同）。
- `set_server(server) -> Option<HostService>`：还没登录账号时先设服务器（全新安装也能开临时协助）。
- `check_update(platform, current)` / `download_update(info, dest, listener)`：用任一有效账号的令牌查询与下载被控端新版本（验签同 5.0）。
- HostService 另有 `disconnect_session(id)` 断开单个会话；`HostSessionInfo.account_id` 是连进来的账号（临时协助为空）；`on_status` 推来的状态里带账号列表。

## 6. 构建

- Apple：`scripts/build-ffi-apple.sh [--debug] [--ios]` 生成 `lemon-galaxy-control-macos/Vendor/LemonGalaxyCore.xcframework` 与 `Sources/LemonGalaxy/Core/Generated/galaxy_ffi.swift`。
- Android：Gradle 任务按 ABI 调 `cargo build -p galaxy-ffi --release --target <abi>`，再用 `target/<profile>/uniffi-bindgen generate --library ... --language kotlin` 生成 `uniffi/galaxy_ffi/galaxy_ffi.kt`。
