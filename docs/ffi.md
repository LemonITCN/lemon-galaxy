# 内核接口（galaxy-ffi）

Rust 内核通过 uniffi 导出给各端原生界面：Swift（macOS / iOS）、Kotlin（Android）、C#（Windows，后续）。
命名在 Swift / Kotlin 里自动转成驼峰（`open_terminal` → `openTerminal`），记录（record）是值类型，对象（object）是引用类型，
`async fn` 在 Swift 是 `async`、在 Kotlin 是 `suspend`。错误统一是 `GalaxyError`。

界面层只负责显示与平台能力（安全存储、截屏编码、解码显示、输入注入），所有网络、加密、协议逻辑都在内核里。

## 1. 初始化

```rust
fn core_version() -> String;
fn core_init(config: CoreConfig);            // 每个进程调用一次
record CoreConfig {
    data_dir: String,                          // 应用私有数据目录
    cache_dir: String,
    log_level: String,                         // "info" | "debug"
    platform: Platform,                        // Macos | Windows | Linux | Android | Ios | Harmonyos
    device_kind: DeviceKind,                   // Control | Worker | Mobile
    app_version: String,
    os_version: String,
    hostname: String,                          // 设备默认名称
    locale: String,                            // "zh-CN" | "en"
}
```

### 1.1 平台安全存储（界面层实现）

```rust
callback SecureStore {
    fn get(key: String) -> Option<Vec<u8>>;
    fn set(key: String, value: Vec<u8>);
    fn remove(key: String);
}
```
macOS / iOS 用 Keychain，Android 用 Android Keystore 包裹的加密文件，Windows 用 DPAPI。

## 2. GalaxyClient（主控端的账号会话，每个应用一个）

```rust
object GalaxyClient {
    constructor new(store: SecureStore, listener: ClientListener) -> GalaxyClient;

    fn state() -> ClientState;                 // LoggedOut | Connecting | Online | Offline（服务器连不上但有本地缓存）
    fn current_user() -> Option<UserInfo>;
    fn server() -> Option<ServerInfo>;

    // 登录：server 为后台给出的完整地址 "host:port#sha256-指纹"
    async fn login(server: String, username: String, password: String, totp: Option<String>) -> LoginResult;
    async fn complete_first_login(new_password: String);   // LoginResult::MustChangePassword 之后调用
    async fn change_password(old_password: String, new_password: String);
    async fn logout();
    fn restore() -> bool;                      // 启动时用本地保存的会话恢复登录，成功返回 true

    // 设备与加密库
    fn devices() -> Vec<DeviceInfo>;           // 本账号所有设备（含在线状态、局域网是否发现）
    async fn refresh_devices();
    async fn remove_device(device_id: String);
    fn vault() -> Vec<VaultEntry>;             // 已解密的条目
    async fn vault_save(entry: VaultEntry) -> VaultEntry;   // 新建时 id 传空字符串
    async fn vault_delete(id: String);

    // 连接
    async fn connect(device_id: String) -> PeerSession;
    async fn connect_assist(device_code: String, verification_code: String) -> PeerSession;
    async fn connect_ssh(host_id: String, secret_prompt: SecretPrompt) -> SshSession;   // vault 里的 ssh_host

    // AI
    async fn ai_models() -> AiModels;
    fn new_conversation(config: ConversationConfig, listener: AgentListener) -> AgentConversation;

    // 本机被控（control 客户端）
    fn host() -> HostService;
}

enum LoginResult { Success, NeedTotp, MustChangePassword }

callback ClientListener {
    fn on_state(state: ClientState);
    fn on_devices_changed();
    fn on_vault_changed();
    fn on_session_revoked();                   // 回登录页
}

record UserInfo { id: u64, username: String, display_name: String, ai_enabled: bool }
record ServerInfo { address: String, name: String, version: String }

record DeviceInfo {
    id: String, kind: DeviceKind, platform: Platform,
    hostname: String, os_version: String, arch: String, app_version: String,
    online: bool,                              // 服务器报告在线
    lan_seen: bool,                            // 局域网发现到
    last_seen_at: i64,
    is_self: bool,
}
```

### 2.1 加密库条目

```rust
record VaultEntry {
    id: String,
    folder_id: Option<String>,                 // 所在文件夹，None 为根
    sort: i64,
    data: VaultData,
    updated_at: i64,
}
enum VaultData {
    Folder { name: String, color: Option<String>, expanded: bool },
    DeviceMeta { device_id: String, alias: Option<String>, tags: Vec<String>, note: String, favorite: bool },
    SshHost { label: String, host: String, port: u16, username: String,
              auth: SshAuth, jump_device_id: Option<String>, jump_host_id: Option<String>,
              tags: Vec<String>, note: String, favorite: bool, terminal_theme: Option<String> },
    RdpHost { label: String, host: String, port: u16, username: String, password: Option<String>, domain: Option<String>,
              jump_device_id: Option<String>, tags: Vec<String>, note: String, favorite: bool },
    Credential { label: String, username: String, password: Option<String>, private_key: Option<String>, passphrase: Option<String> },
    Snippet { label: String, command: String, tags: Vec<String> },
    KnownHost { host: String, port: u16, key_type: String, fingerprint: String },
    Preferences { json: String },              // 跨设备同步的界面偏好（主题、终端配色、字体等）
}
enum SshAuth { Password { password: Option<String> }, PrivateKey { credential_id: String }, Agent, Interactive }
```

## 3. PeerSession（与一台被控设备的会话）

```rust
object PeerSession {
    fn info() -> PeerInfo;                     // 主机信息、权限、能力、当前路径
    fn path() -> PathInfo;                     // kind: Lan | Direct | RelayUdp | RelayTcp, rtt_ms, remote_addr
    fn set_listener(listener: PeerListener);
    fn close();

    async fn open_terminal(options: TerminalOptions, sink: TerminalSink) -> TerminalChannel;
    async fn list_terminals() -> Vec<TerminalSessionInfo>;
    async fn kill_terminal(session_id: String);

    fn files() -> FileClient;

    async fn exec(request: ExecRequest) -> ExecResult;

    async fn list_displays() -> Vec<DisplayInfo>;
    async fn open_display(options: DisplayOptions, sink: VideoSink) -> DisplayChannel;
    async fn screenshot(display_id: u32, max_width: u32) -> ImageData;

    async fn clipboard_get() -> ClipboardData;
    async fn clipboard_set(data: ClipboardData);

    async fn forward_port(local_port: u16, remote_host: String, remote_port: u16) -> PortForward;
}

callback PeerListener {
    fn on_path_changed(path: PathInfo);
    fn on_closed(reason: String);
}

record PeerInfo {
    device_id: String, hostname: String, platform: Platform, os_version: String, arch: String,
    permissions: Permissions, capabilities: Vec<String>, path: PathInfo,
}
record Permissions { view: bool, control: bool, files: bool, terminal: bool, clipboard: bool, audio: bool, ai_cap: AiMode }
record PathInfo { kind: PathKind, rtt_ms: u32, remote_addr: String }
```

### 3.1 终端

```rust
record TerminalOptions { attach_session: Option<String>, shell: Option<String>, cols: u16, rows: u16 }
callback TerminalSink {
    fn on_output(data: Vec<u8>);
    fn on_exit(code: Option<i32>);
}
object TerminalChannel {
    fn session_id() -> String;
    fn write(data: Vec<u8>);
    fn resize(cols: u16, rows: u16);
    fn detach();                               // 断开但被控端保留会话
    fn kill();
}
record TerminalSessionInfo { id: String, title: String, created_at: i64, attached: bool }
```
终端仿真与渲染由界面层的终端控件完成（macOS / iOS：SwiftTerm；Android：Termux terminal-view），内核只传原始字节。

### 3.2 文件

```rust
object FileClient {
    async fn home() -> String;
    async fn list(path: String) -> Vec<FileEntry>;
    async fn stat(path: String) -> FileEntry;
    async fn mkdir(path: String);
    async fn rename(from: String, to: String);
    async fn remove(path: String, recursive: bool);
    async fn chmod(path: String, mode: u32);
    async fn read_text(path: String, max_bytes: u64) -> String;
    async fn write_text(path: String, text: String);
    fn download(remote: String, local: String, listener: TransferListener) -> Transfer;   // 支持断点续传
    fn upload(local: String, remote: String, listener: TransferListener) -> Transfer;
}
record FileEntry { name: String, path: String, kind: FileKind /* File | Dir | Symlink | Other */, size: u64, mode: u32, modified_at: i64, owner: String, group: String, link_target: Option<String> }
callback TransferListener { fn on_progress(done: u64, total: u64); fn on_finished(error: Option<String>); }
object Transfer { fn cancel(); fn pause(); fn resume(); }
```
本地一侧（双栏里的「本机」）由界面层直接用系统文件接口实现。

### 3.3 远程画面

```rust
record DisplayInfo { id: u32, name: String, width: u32, height: u32, scale: f32, primary: bool, is_virtual: bool }
record DisplayOptions { display_id: u32, max_fps: u32, bitrate_kbps: u32, codecs: Vec<VideoCodec> /* H264 | H265 */ }
callback VideoSink {
    fn on_config(codec: VideoCodec, width: u32, height: u32);
    fn on_frame(data: Vec<u8>, pts_us: u64, keyframe: bool);   // Annex B 码流
    fn on_cursor(x: i32, y: i32, visible: bool);
}
object DisplayChannel {
    fn send_input(event: InputEvent);
    fn request_keyframe();
    fn set_bitrate(kbps: u32);
    fn stop();
}
enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: MouseButton, down: bool, x: i32, y: i32 },
    Wheel { dx: i32, dy: i32 },
    Key { hid_usage: u32, down: bool },
    Text { text: String },
    SpecialCombo { combo: SpecialCombo },      // CtrlAltDel | LockScreen | ...
}
```
界面层用平台硬件解码器（Android MediaCodec / Apple VideoToolbox）解码 `on_frame` 的数据并显示，把触控 / 鼠标 / 键盘转换成 `InputEvent`（坐标为被控屏幕像素）。

## 4. AI 智能体

```rust
record AiModels { enabled: bool, max_mode: AiMode, items: Vec<AiModel> }
record AiModel { provider_id: u64, provider_name: String, id: String, name: String, vision: bool, tools: bool, context_length: u32 }
enum AiMode { Off, ReadOnly, ConfirmAll, ConfirmDangerous, FullTrust }

record ConversationConfig {
    provider_id: u64, model_id: String,
    targets: Vec<AgentTarget>,                 // 设备或 SSH 主机
    mode: AiMode,
    plan_first: bool,
}
record AgentTarget { kind: AgentTargetKind /* Device | SshHost */, id: String }

object AgentConversation {
    fn id() -> String;
    async fn send(text: String);
    fn decide(call_id: String, decision: ApprovalDecision);   // 回应确认卡片
    fn answer(question_id: String, answer: String);          // 回应 ask_user
    fn provide_secret(request_id: String, secret: String);   // 回应 request_secret，不进模型上下文
    fn set_mode(mode: AiMode, trust_minutes: Option<u32>);
    fn stop();
    fn messages() -> Vec<AgentMessage>;        // 历史（恢复界面用）
}
enum ApprovalDecision { AllowOnce, AllowSimilar, Deny { reason: Option<String> }, EditAndRun { command: String } }

callback AgentListener { fn on_event(event: AgentEvent); }
enum AgentEvent {
    AssistantDelta { message_id: String, text: String },
    AssistantDone { message_id: String },
    Thinking { message_id: String, text: String },
    ToolProposed { call: ToolCallView, needs_approval: bool },
    ToolStarted { call_id: String },
    ToolOutput { call_id: String, text: String },             // 流式输出
    ToolFinished { call_id: String, result: ToolResultView },
    AskUser { question_id: String, question: String, options: Vec<String> },
    SecretRequested { request_id: String, prompt: String, target_id: String },
    Usage { prompt_tokens: u64, completion_tokens: u64 },
    Status { running: bool, step: u32 },
    Error { message: String },
}
record ToolCallView {
    call_id: String, tool: String, target_id: String, target_name: String,
    summary: String,                           // 一行描述，例如命令本身
    detail: String,                            // 完整参数 / 改动对比
    risk: RiskLevel,                           // Read | Normal | Dangerous | Unknown | Forbidden
    risk_reasons: Vec<String>, model_reason: String,
    image: Option<ImageData>,                  // 点击位置标注图等
}
record ToolResultView { ok: bool, summary: String, output: String, image: Option<ImageData>, denied: bool }
record ImageData { mime: String, data: Vec<u8>, width: u32, height: u32 }
```

## 5. HostService（本机被控，control 客户端与 worker）

```rust
object HostService {
    fn status() -> HostStatus;                 // 是否在线、登录的账号列表、当前被谁控制
    fn set_platform(platform: HostPlatform);   // 注册平台能力（截屏编码、输入注入、剪贴板）
    fn set_listener(listener: HostListener);
    fn accounts() -> Vec<HostAccount>;         // 每个账号的权限、系统用户、AI 上限
    fn update_account_policy(user_id: u64, policy: HostPolicy);
    fn set_remote_enabled(enabled: bool);      // 一键禁止远程
    fn disconnect_all();
    fn stop_ai();
    fn start_assist() -> AssistInfo;           // 临时协助：设备码 + 验证码
    fn stop_assist();
    fn set_assist_code(code: Option<String>);  // 自定义验证码，None 为随机
    fn respond_assist(request_id: String, allow: bool);
    fn recent_events() -> Vec<HostEvent>;      // 本地日志
}

callback HostPlatform {
    fn list_displays() -> Vec<DisplayInfo>;
    fn start_capture(display_id: u32, options: CaptureOptions, sink: EncodedFrameSink) -> bool;
    fn stop_capture(display_id: u32);
    fn request_keyframe(display_id: u32);
    fn set_bitrate(display_id: u32, kbps: u32);
    fn inject(event: InputEvent);
    fn screenshot(display_id: u32, max_width: u32) -> Option<ImageData>;
    fn ui_tree(display_id: u32, max_nodes: u32) -> Option<String>;   // JSON
    fn clipboard_get() -> Option<ClipboardData>;
    fn clipboard_set(data: ClipboardData);
}
object EncodedFrameSink {                      // 内核实现，平台层编码后推送
    fn push(data: Vec<u8>, pts_us: u64, keyframe: bool);
    fn set_config(codec: VideoCodec, width: u32, height: u32);
}
callback HostListener {
    fn on_status(status: HostStatus);
    fn on_assist_request(request_id: String, from_user: String, from_device: String);
    fn on_session_started(info: HostSessionInfo);   // 被控提示
    fn on_session_ended(session_id: String);
    fn on_ai_activity(session_id: String, description: String);
}
```

## 6. SSH 直连

```rust
callback SecretPrompt { fn ask(prompt: String, echo: bool) -> Option<String>; }   // 键盘交互认证、私钥口令、首次连接确认主机指纹
object SshSession {
    async fn open_terminal(cols: u16, rows: u16, sink: TerminalSink) -> TerminalChannel;
    fn files() -> FileClient;                  // SFTP，接口与 3.2 相同
    async fn exec(request: ExecRequest) -> ExecResult;
    async fn forward_local(local_port: u16, remote_host: String, remote_port: u16) -> PortForward;
    fn close();
}
```

## 7. 错误

```rust
enum GalaxyError {
    Network { message: String },
    Auth { code: String, message: String },    // invalid_credentials | need_totp | locked | disabled | must_change_password
    Permission { message: String },
    NotFound { message: String },
    Protocol { message: String },
    Cancelled,
    Other { message: String },
}
```
`message` 已按 `CoreConfig.locale` 本地化，界面直接显示。
