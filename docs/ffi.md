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

async connect(device_id) -> PeerSession
async connect_assist(device_code, verification_code) -> PeerSession   会等被控端本人点允许（最多约 2 分钟）

async ai_models() -> AiModels
async new_conversation(ConversationConfig, AgentListener) -> AgentConversation   会先连上所有目标设备
list_conversations() -> [ConversationSummary]                                    本机保存（加密）
async open_conversation(id, ConversationConfig, AgentListener) -> AgentConversation
delete_conversation(id)

host() -> HostService?                   control 客户端登录上线后才有
```

回调 `ClientListener`：`on_state`、`on_devices_changed`、`on_vault_changed`、`on_session_revoked`（回登录页）。
回调 `SecureStore`：`get(key) -> bytes?`、`set(key, bytes)`、`remove(key)`。

加密库 `VaultEntry { id, folder_id?, sort, data: VaultData, updated_at }`，`VaultData`：
`Folder`、`DeviceMeta`、`SshHost`（`auth: SshAuth`）、`RdpHost`、`Credential`、`Snippet`、`KnownHost`、`Preferences{json}`。

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
async list_displays() -> [DisplayInfo]
async open_display(DisplayOptions{display_id, max_fps, bitrate_kbps, codecs, view_only}, VideoSink) -> DisplayChannel
async screenshot(display_id, max_width) -> ImageData
async clipboard_get() -> ClipboardData? / clipboard_set(ClipboardData)      Text{text} | Image{image} | Files{paths}
async forward_port(local_port, remote_host, remote_port) -> PortForward     local_port() / stop()
async wake_on_lan(mac)
```

- `TerminalChannel`：`session_id()`、`title()`、`write(bytes)`、`resize(cols, rows)`、`detach()`（被控端保留会话）、`kill()`。写入按调用顺序送达。
  `TerminalSink`：`on_output(bytes)`、`on_exit(code?)`。终端仿真由界面层控件完成（SwiftTerm / Termux terminal-view）。
- `FileClient`：`home`、`list`、`stat`、`realpath`、`mkdir`、`rename`、`remove(path, recursive)`、`chmod`、`search(root, pattern, content)`、`read_text`、`write_text`、
  `download(remote, local, TransferListener) -> Transfer`、`upload(local, remote, TransferListener) -> Transfer`（都支持断点续传；只接受本机文件路径）。
  `Transfer`：`cancel()`、`pause()`、`resume()`。
- `DisplayChannel`：`send_input(InputEvent)`、`request_keyframe()`、`set_bitrate(kbps)`、`stop()`。
  `VideoSink`：`on_config(codec, width, height)`（分辨率变化时会再次调用）、`on_frame(annexb, pts_us, keyframe)`、`on_cursor(x, y, visible)`（被控屏幕像素）、`on_closed(reason)`。
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
  decide(call_id, ApprovalDecision)          AllowOnce | AllowSimilar | Deny{reason?} | EditAndRun{command}
  answer(question_id, answer)
  provide_secret(request_id, secret)         内容直接送到被控端，不进模型上下文
  set_mode(mode, trust_minutes?)
  stop()
  set_foreground(Bool)                       移动端切后台时 false：停止当前任务
  messages() -> [AgentMessage]               User{text} | Assistant{id, text, thinking} | Tool{call, result?}
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
async start_assist() -> AssistInfo{device_code, code}
stop_assist() / set_assist_code(code?) / respond_assist(request_id, allow)
recent_events(limit) -> [HostEvent]
```

`HostPlatform`（界面层实现）：`list_displays`、`start_capture(display_id, CaptureOptions, EncodedFrameSink) -> CaptureStarted?`、`stop_capture`、`request_keyframe`、`set_bitrate`、
`inject(InputEvent)`、`screenshot(display_id, max_w, max_h) -> ImageData?`、`ui_tree(display_id, max_nodes) -> String?`、`clipboard_get/set`、`capabilities() -> [String]`
（已获得系统授权的能力：`display`、`screenshot`、`input`、`ui_tree`、`clipboard`）。
`EncodedFrameSink`（内核实现）：`push(annexb, pts_us, keyframe)`、`set_resolution(width, height)`。

`HostListener`：`on_status`、`on_assist_request(request_id, from_user, from_device)`（弹窗后调 `respond_assist`）、`on_session_started`、`on_session_ended`、`on_ai_activity`。

## 6. 构建

- Apple：`scripts/build-ffi-apple.sh [--debug] [--ios]` 生成 `lemon-galaxy-control-macos/Vendor/LemonGalaxyCore.xcframework` 与 `Sources/LemonGalaxy/Core/Generated/galaxy_ffi.swift`。
- Android：Gradle 任务按 ABI 调 `cargo build -p galaxy-ffi --release --target <abi>`，再用 `target/<profile>/uniffi-bindgen generate --library ... --language kotlin` 生成 `uniffi/galaxy_ffi/galaxy_ffi.kt`。
