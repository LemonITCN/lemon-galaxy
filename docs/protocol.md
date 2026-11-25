# 连接与会话协议

## 1. 总览

```
主控端 ── WebSocket(TLS) ──┐                ┌── WebSocket(TLS) ── 被控端
                            ├── 服务器 7443 ──┤
主控端 ── UDP 探测/中转 ────┘                └──── UDP 探测/中转 ── 被控端

主控端 ════════ QUIC（局域网 / 公网直连 / 服务器中转，路径可随时切换）════════ 被控端
```

- 信令走 WebSocket（JSON）；两端之间的业务走 QUIC，端到端加密认证（security.md 第 5 节）。
- 每个端只开一个 UDP 套接字（下称「魔法套接字」），QUIC 数据包、两端探测包、服务器探测与中转包都从这一个口进出。
  QUIC 看到的对端地址是一个固定的虚拟地址，真实发往哪条路径由魔法套接字决定，所以换路径对 QUIC 完全透明，连接不断。

## 2. WebSocket 信令

`GET /api/v1/ws`，文本帧，JSON，`t` 为消息类型。

### 2.1 认证

1. 服务器 → `{"t":"hello","nonce":"b64","server_time":ms}`
2. 客户端 → `{"t":"auth","device_id":"hex32","signature":"b64","macs":["AA:BB:CC:DD:EE:FF"]?}`，签名内容 `"lemon-galaxy/ws-auth/v1" || nonce`，用设备身份密钥。
   被控连接附带本机网卡 MAC（有私网 IPv4、非虚拟、非随机化地址，最多 8 个），服务器记下供远程开机用。
   已登录设备同时带 HTTP 头 `Authorization: Bearer <session_token>`；临时协助模式（未登录）用 `?assist=1` 且不带令牌。
3. 服务器 → `{"t":"ready","device_id":"...","public_ip":"1.2.3.4","udp_port":7443,"assist_code":"123456789"?}`

### 2.2 推送

- `{"t":"presence","device_id":"...","online":true}`：本账号设备上下线
- `{"t":"devices_changed"}`、`{"t":"vault_changed","version":123}`、`{"t":"revocations_changed"}`
- `{"t":"session_revoked"}`：本设备会话被注销，客户端回到登录页
- `{"t":"account_reset","user_id":3}`：账号被管理员重置密码，worker 将该账号标为「需要重新登录」

### 2.3 发起连接

主控端 → `{"t":"connect","req_id":"r1","target":"<device_id>","mode":"control"}`
临时协助：`{"t":"connect","req_id":"r1","mode":"assist","assist_code":"123456789"}`

服务器校验（目标在线；control 模式下目标设备登录了发起者的账号且未吊销；assist 模式下设备码存在），分配：

- `conn_id`：16 字节随机
- 连接票据 `ConnTicket`（security.md），服务器签名
- 两个中转令牌（每端一个，16 字节随机）

服务器 → 被控端：
```json
{"t":"incoming","conn_id":"b64","mode":"control",
 "from_device":"hex32","from_user":{"id":3,"username":"alice","display_name":"Alice"},
 "ticket":"b64","ticket_sig":"b64","relay_token":"b64",
 "peer_public_ip":"1.2.3.4","same_public_ip":true}
```
服务器 → 主控端：
```json
{"t":"connect_ok","req_id":"r1","conn_id":"b64","target":"hex32",
 "ticket":"b64","ticket_sig":"b64","relay_token":"b64",
 "peer_public_ip":"5.6.7.8","same_public_ip":false}
```
失败：`{"t":"connect_error","req_id":"r1","code":"offline|forbidden|not_found|rate_limited"}`

### 2.4 交换候选地址

双方各自收集候选地址后发给服务器，服务器原样转给对方（可多次发送，增量）：
```json
{"t":"candidates","conn_id":"b64","items":[{"addr":"192.168.1.5:51820","kind":"host"},{"addr":"[240e::1]:51820","kind":"host"},{"addr":"1.2.3.4:40001","kind":"srflx"}]}
```
kind：`host`（本机网卡地址）、`srflx`（服务器看到的公网地址）、`lan`（局域网发现得到的地址）。

### 2.5 状态上报与关闭

- 客户端 → `{"t":"conn_state","conn_id":"b64","path":"lan|direct|relay_udp|relay_tcp","rtt_ms":12}`（只用于后台统计与展示）
- 任一方 → `{"t":"conn_close","conn_id":"b64"}`；服务器转给对方，并释放中转令牌。

## 3. 服务器 UDP 协议

服务器 UDP 端口与 TCP 端口号相同。每个包以 4 字节魔数 `00 4C 47 53`（`\0LGS`）开头，第 5 字节为类型：

| 类型 | 方向 | 内容 |
|---|---|---|
| `0x01` BindRequest | 客户端 → 服务器 | `txn[12]` |
| `0x02` BindResponse | 服务器 → 客户端 | `txn[12]` + 观察到的地址 |
| `0x10` RelayBind | 客户端 → 服务器 | `token[16]`：把这个令牌绑定到发送方 UDP 地址 |
| `0x11` RelayBound | 服务器 → 客户端 | `token[16]` |
| `0x12` RelayData | 客户端 → 服务器 | `token[16]` + 负载：转发给对端 |
| `0x13` RelayDeliver | 服务器 → 客户端 | `token[16]`（收方自己的令牌）+ 负载 |
| `0x20` Keepalive | 客户端 → 服务器 | `token[16]`，保持 NAT 映射 |

地址编码：`family u8 (4|6)` + IP（4 或 16 字节）+ `port u16 BE`。

中转只在同一 `conn_id` 的两个令牌之间转发；令牌在连接关闭或 10 分钟无流量后失效。服务器按设置限速。

### 3.1 TCP 中转

UDP 不通时，客户端打开 `wss://服务器/api/v1/relay?token=<b64url 令牌>`，二进制帧即负载。
两端可以一端走 UDP 中转、另一端走 TCP 中转，服务器负责桥接。

## 4. 两端之间的探测包（disco）

直连路径上的探测包以魔数 `00 4C 47 44`（`\0LGD`）开头（首字节 0x00 不可能是合法 QUIC 包，因为 QUIC 固定位必须为 1）：

- `0x01` Ping：`conn_tag[8]` + `txn[12]`
- `0x02` Pong：`conn_tag[8]` + `txn[12]` + 观察到的发送方地址

`conn_tag = SHA-256(conn_id || "lemon-galaxy/disco/v1")[0..8]`。探测包不做加密认证，只用来选路；安全由 QUIC 握手与端到端认证保证。
中转路径上同样跑 Ping/Pong（封装在 RelayData 里），用来测中转的 RTT。

## 5. 局域网发现

UDP 端口 7444；IPv4 组播 `239.255.71.71`、IPv4 广播 `255.255.255.255`、IPv6 组播 `ff02::4c47`。被控端在所有网卡上监听（允许端口复用）。

`lan_key = HKDF-SHA256(ikm = 账号公钥, salt = "lemon-galaxy/lan/v1", info = "", 32)`。只有同账号的设备（以及服务器）知道账号公钥，同一网络里的陌生人无法构造查询，也认不出回应。

查询（主控端 → 组播/广播）：
```
00 4C 47 51 ('\0LGQ') | ver u8 = 1 | nonce[16] | target_device[16]（全 0 表示同账号的任意设备）| reply_port u16 BE | mac[16]
mac = HMAC-SHA256(lan_key, "q" || nonce || target_device || reply_port)[0..16]
```
回应（被控端 → 查询方 IP:reply_port，即主控端的魔法套接字）：
```
00 4C 47 52 ('\0LGR') | ver u8 = 1 | nonce[16] | device_id[16] | port u16 BE | n u8 | n × 地址 | mac[16]
mac = HMAC-SHA256(lan_key, "r" || nonce || device_id || port || 地址列表)[0..16]
```
被控端对它登录的每个账号的 `lan_key` 都尝试校验查询。主控端收到合法回应后，把回应的源地址（以及回应里列出的地址）作为 `lan` 候选立即探测。

触发时机：每次发起连接时立即查询（并在 3 秒内重复 3 次）；设备列表页打开时每 10 秒查询一次「任意设备」，用于离线局域网模式下显示「局域网内在线」。

## 6. 选路

1. 收到对端候选后，主控端和被控端都向对方每个候选地址发 Ping（前 3 秒每 200 毫秒一轮，之后每 2 秒），双方都发是为了打开各自的 NAT。
2. 同时，双方立即绑定中转令牌（UDP，3 秒没回应就改 TCP），保证无论如何都能连上。
3. 收到 Pong 的路径算「可用」，记录 RTT。路径评分 = RTT + 类型惩罚：`lan`/私网 `host` 0 毫秒、IPv6 直连 3 毫秒、IPv4 直连 5 毫秒、UDP 中转 40 毫秒、TCP 中转 80 毫秒。
4. 魔法套接字始终把 QUIC 包发到当前评分最低的可用路径；初始没有直连路径时走中转。
5. 连接建立后持续每 5 秒探测一次所有候选，评分好出 10 毫秒以上就切换。当前路径连续 3 次探测无回应，立即退回次优路径。
6. 网卡变化（换 Wi-Fi、插网线）时重新收集候选并通过信令发给对方。
7. 虚拟网卡（docker*、br-*、veth*、vmnet*、vboxnet*、utun*、tun*、tap*、zt*、tailscale*）的地址仍然尝试，但类型惩罚加 20 毫秒。

### 6.1 离线局域网模式

服务器不可达时：主控端只用局域网发现得到的地址直接探测，QUIC 建立后端到端认证（无票据）。被控端只在「自己也连不上服务器」时接受无票据连接。

## 7. QUIC 参数

- ALPN `lemon-galaxy/1`；主控端是 QUIC 客户端，被控端是 QUIC 服务端。
- 双方用设备身份密钥生成自签证书；TLS 校验交给端到端认证（security.md 5 节）。
- 空闲超时 30 秒，保活 5 秒；启用 DATAGRAM 扩展；最多 256 条双向流、256 条单向流。
- 魔法套接字给每个对端分配一个虚拟地址 `[fd4c:4778::<序号>]:7` 交给 QUIC 使用。

## 8. 会话协议

所有流上的消息都是「4 字节小端长度 + postcard 编码」的帧，单帧最大 4 MiB。

### 8.1 控制流

主控端打开的第一条双向流。双方先互发 `Hello`（security.md 5 节），被控端校验通过后回 `HelloAck`：

```
HelloAck {
  host: HostInfo { device_id, hostname, platform, os_version, arch, app_version },
  account: Option<u64>,               // 以哪个账号的身份被控（协助模式为 None）
  permissions: Permissions { view, control, files, terminal, clipboard, audio, ai_cap },
  capabilities: Vec<String>,          // "terminal" "files" "exec" "display" "input" "clipboard" "clipboard_offer" "port_forward" "screenshot" "ui_tree" "privacy" "audio" "video_keepalive"
                                      // "virtual_display" + "virtual_display_max=N"（8.6）
}
```
之后控制流用于心跳、会话级事件（被控端本地用户断开、权限变化）。

### 8.2 功能流

主控端每打开一个功能，就开一条新的双向流，第一帧是 `StreamOpen`：

| StreamOpen | 后续消息 |
|---|---|
| `Terminal { attach: Option<String>, shell: Option<String>, cols, rows, env }` | 被控端回 `TerminalOpened { session_id, title }`；之后 主控→被控 `Input(bytes)` / `Resize{cols,rows}` / `Detach` / `Kill`，被控→主控 `Output(bytes)` / `Exited{code}` |
| `Mgmt` | 请求/应答：`ListTerminals`、`KillTerminal{id}`、`ListDisplays`、`Screenshot{display, max_width, max_height, format}`、`UiTree{display, max_nodes}`、`ClipboardGet`、`ClipboardSet`、`WakeOnLan{mac}`、`HostStatus`、`Inject`、`SetPrivacy{on}`、`VirtualDisplayAdd{width, height, refresh}`（回 `Display(DisplayInfo)`）、`VirtualDisplayRemove{display_id}`（8.6） |
| `Files(op)` | 一次文件操作一条流：`List{path}`、`Stat{path}`、`Read{path, offset, len}`（被控端回多个 `Chunk` + `End`）、`Write{path, offset, truncate, mode, expect_sha256}`（主控端发多个 `Chunk` + `End`，被控端回结果）、`Mkdir{path, parents}`、`Remove{path, recursive}`、`Rename{from, to, overwrite}`、`Chmod{path, mode}`、`Home`、`Realpath{path}`、`Search{root, pattern, max}` |
| `Exec { command, shell, cwd, env, timeout_ms, origin, max_output }` | 被控端流式回 `Stdout(bytes)` / `Stderr(bytes)`，最后 `Done { code, signal, timed_out, truncated, duration_ms }`；主控端可发 `Stdin(bytes)` / `Cancel` |
| `PortForward { host, port }` | 被控端回 `Ok` 后双向透传字节 |
| `Display { display_id, codecs, max_fps, bitrate_kbps }` | 被控端回 `Started { width, height, codec, channel }`；画面走 DATAGRAM；本流上 主控→被控 `Input(InputEvent)` / `RequestKeyframe` / `SetBitrate` / `Stop` / `ClientCursor(bool)`，被控→主控 `Cursor{x, y, visible}` / `Resized{...}`。`ClientCursor(true)` 表示主控端在本地画光标（跟手）：只有这块屏的所有观看者都要求时，被控端发出的画面才不画光标，并在光标变化时发 `Cursor`（画面像素，光标热点）；主控端只在能力列表有 `client_cursor` 时发 |
| `Clipboard` | 双向 `Update { kind, data }` |
| `Audio { codecs, bitrate_kbps }` | 被控端回 `Started { codec, sample_rate, channels, channel }` 或 `Error`；声音走 DATAGRAM（8.5）；主控端发 `Stop` 或关流即停止 |

`origin`：`User` 或 `Ai { conversation, step }`。被控端对 AI 发起的操作单独套用 AI 上限与绝对禁止清单（ai-agent.md），并单独记日志。

任何 `StreamOpen` 在权限不足时，被控端回 `Error { code: "permission_denied", message }` 并关闭流。

### 8.3 画面数据报

```
channel u8 | frame_id u32 LE | frag_idx u16 LE | frag_count u16 LE | flags u8 (bit0 关键帧) | [frag_idx == 0 时: pts_us u64 LE] | 负载
```
接收方按 frame_id 重组。编码端无 B 帧、低延迟模式、无限 GOP（只在被要求时出关键帧），所以丢帧后必须等关键帧：
- 丢帧的判定：一帧的分片 150 毫秒没有新的到达（按最后一片算，大关键帧慢慢传不算丢）；后一帧先收齐（前面没收齐的作废）；帧号跳号（整帧一片都没收到）。
- 丢帧后到下一个关键帧之前，收齐的帧也不交给解码器（画面停在最后一张好的帧，不花屏），立刻发 `RequestKeyframe`，之后每 500 毫秒重催；刚打开画面时也从第一个关键帧开始交。

画面结束：本流结束（被控端结束、出错 `Error`、断线）时主控端必须告诉界面（内核 `VideoSink::on_closed`），自己 `Stop` 的不算。
能力列表有 `video_keepalive` 的被控端（Windows、Linux）空闲时也每秒至少发一帧；主控端 4 秒没有画面数据先发 `RequestKeyframe` 探一下，
10 秒（刚打开、还没收到过数据时 20 秒）仍没有就判定「画面没有响应」，`Stop` 并通知界面。界面收到画面结束或会话断开时显示「正在重新连接」并自动重连。

码率自适应（被控端）：主控端要求的码率是上限；被控端每秒看一次 QUIC 连接的丢包率、拥塞事件和数据报发送缓冲占用，
拥塞时立即降到七成（3 秒内不再降），连续 15 秒平稳再升 25%，最低 300 kbps。主控端发 `SetBitrate` 视为用户主动选择，直接按新值重新开始。

### 8.4 输入事件

```
InputEvent =
  MouseMove { x, y }                     // 被控屏幕像素坐标
  MouseButton { button: Left|Right|Middle|Back|Forward, down: bool, x, y }
  Wheel { dx, dy }                       // 像素；dy 为正 = 滚轮往上推（内容下移），dx 为正 = 往左（内容右移），与 macOS 关闭自然滚动时的滚轮一致
  Key { code: u32 /* USB HID 用法码 */, down: bool }
  Text { text }                          // 输入法提交的文字
  Touch { id, phase, x, y }              // 触控模式
  SpecialCombo { kind: CtrlAltDel|LockScreen|... }
```

### 8.4.1 复制粘贴文件（复制时只交换清单，粘贴时才传）

文字照旧即时同步：主控端发现本机复制了文字就 `ClipboardSet(Text)`，鼠标进入画面时 `ClipboardGet`。
文件像 UU 远程：**复制时只交换清单，有人粘贴时才传内容，进度显示在粘贴的那一边**。

清单 `ClipOffer { id, entries: [ClipOfferEntry { path, is_dir, size, mtime }] }`：`id` 是复制方生成的随机数；`path` 是用 `/` 分隔的相对路径，
顶层项就是复制的那几项（同名的第二个起加「 2」「 3」），文件夹排在它的内容前面，最多 1 万项（文件与文件夹合计）。
粘贴方落盘前必须 `ClipOffer::validate`：只接受安全的相对路径（没有空段、`.`、`..`、反斜杠、NUL），父文件夹在前，不能写出粘贴目标目录。

**本机复制 → 远端粘贴**（被控端能力列表有 `clipboard_offer`）：
1. 主控端把本机复制的文件展开成清单，`MgmtReq::ClipboardOffer(offer)`（需要剪贴板与文件权限）。被控端校验后交给平台层延迟放进系统剪贴板，应答 `Ok`。
2. 有人粘贴时，被控端向主控端**开反向流**（被控端 `open_bi`，主控端内核一直在 `accept_bi`），第一帧
   `ReverseOpen::ClipboardRead { offer_id, index, offset }`；主控端回 `ClipReadReply::Ok { size }`（文件现在的大小）或 `Error`，
   是 Ok 时接着发原始字节（不分帧）到文件末尾并结束流。被控端中途不要了就 stop 这条流。
   主控端只认本会话最近 4 份清单里的文件项，别的一律拒绝。
3. 没有 `clipboard_offer` 能力的老被控端：主控端照旧把文件整批上传到 `~/.lemon-galaxy/clipboard/<毫秒时间戳>-<随机数>/` 再 `ClipboardSet(Files)`。

**远端复制 → 本机粘贴**（不需要新消息，老被控端也行）：鼠标进入画面时 `ClipboardGet` 得到 `Files[远端路径]`，
主控端用 `fs_stat` / `fs_list` 展开成清单（不下载），放进本机剪贴板；粘贴时按项用 `FileOp::Read { offset }` 读。

**各平台的粘贴方做法**（粘贴方是被控端或主控端都一样）：
- Windows：OLE 数据对象 `CFSTR_FILEDESCRIPTORW` + `CFSTR_FILECONTENTS`（按项给 IStream，读时才取数据），带 `FD_PROGRESSUI`；
  资源管理器按 Ctrl+V、右键粘贴都显示它自己的「正在复制」进度框。
- Linux：FUSE 虚拟目录 `$XDG_RUNTIME_DIR/lemon-galaxy/clip/<清单 id>/…`，剪贴板给 `text/uri-list` 与 `x-special/gnome-copied-files`（copy）指向它，
  文件管理器复制时才读，显示它自己的进度；没有 FUSE（没有 /dev/fuse 或 fusermount3）时，粘贴请求到来时先下载到暂存目录（自己发进度通知）再给路径。
- macOS（访达不支持从剪贴板延迟粘贴）：照 UU 的做法，剪贴板里放一个隐藏的 0 字节标记文件 `.lemongalaxy-paste-<清单 id 十六进制>`（public.file-url）；
  用 FSEvents 盯全盘，访达在哪个文件夹粘贴，标记文件就出现在哪里：删掉标记，在那个文件夹弹出仿访达的「正在将 N 个项目拷贝到 “xxx”」进度窗口开始传，可取消；
  同名按访达的规则加「 2」。粘贴到桌面、文稿、下载等受保护文件夹时系统会问一次是否允许访问。
- 安卓、iOS 主控端不做文件剪贴板。
- 平台层自己放上去的清单不当成「远端复制了文件」：`ClipboardGet` 不返回指向自己虚拟目录或标记文件的路径，免得来回传。
- 被控端在剪贴板管理请求里把文件路径与协议格式互转（Windows `C:\x` ↔ `/C:/x`），文件类剪贴板要同时有剪贴板与文件权限。

### 8.5 声音

```
channel u8 | seq u32 LE | 一个 Opus 包（48 kHz 立体声，20 毫秒）
```
- 被控端每台机器只采一路系统声音（Windows WASAPI 环回、macOS ScreenCaptureKit），统一转成 48 kHz 立体声后 Opus 编码一次，分发给所有在听的会话；
  第一个会话开始听时才开始采集，最后一个停止时关掉。没有声音时平台层不给数据，也就不发包，`seq` 只在发包时递增。
- 主控端按 `seq` 发现丢包，最多用解码器的丢包补偿补 3 包（60 毫秒），迟到的旧包丢弃。播放端先攒 40 毫秒再出声，积压超过 250 毫秒丢旧的，延迟不累积。
- 权限：需要账号策略里的 `audio`；被控端没有声音输出设备（服务器、虚拟机）时能力列表里没有 `audio`。

### 8.6 虚拟屏

被控端用虚拟屏驱动加出“看不见的显示器”，扩展进桌面，主控像普通屏幕一样开画面（Windows：LemonGalaxy Display 驱动，见 `lemon-galaxy-worker-windows/driver/`；macOS：系统私有接口 CGVirtualDisplay，见 docs/research/virtual-display.md 5.10）。

- 能力：被控端装了驱动、主控有控制权限时，能力列表里有 `virtual_display` 和 `virtual_display_max=N`（主控最多能加的块数，Windows、macOS 为 3）。
  取值用 `galaxy_proto::peer::capability_value`。Linux 被控端、没装驱动的 Windows、系统没有 CGVirtualDisplay 或没授权「屏幕录制」的 macOS 都没有这两项。
- `MgmtReq::VirtualDisplayAdd { width, height, refresh }`：要控制权限。宽 640–7680、高 480–4320、刷新率 24–240（0 表示 60）。
  成功回 `MgmtResp::Display(DisplayInfo)`（`is_virtual` 为真），主控拿它的 `id` 开 `Display` 流；之后 `ListDisplays` 里它也带 `is_virtual: true`。
  失败：`permission_denied`、`unsupported`（没有驱动）、`bad_request`（分辨率不合法）、`limit`（已到上限）、`virtual_display`（驱动或系统报错，`message` 是原因）。
  被控端给新屏的模式表里放了要求的模式（首选）和常见 16:9 / 16:10 分辨率 × 30/60/120/144 Hz。
- `MgmtReq::VirtualDisplayRemove { display_id }`：只能删本会话加的（别的会话的回 `permission_denied`，没有这块回 `not_found`），成功回 `Ok`。
- 归属：虚拟屏属于加它的会话，会话结束时被控端自动删掉（主控没发 QUIC 关闭时要等空闲超时，最多约 30 秒）。最后一块删掉后，显示配置和加屏前不同就恢复原样。
- 无头兜底屏：被控端本机没有任何亮着的显示器（笔记本合盖、没接显示器；不算我们自己的虚拟屏）时，主控 `ListDisplays` 会先自动加一块虚拟屏
  （默认 1920x1080@60，被控端 `lemon-galaxy-worker headless-display off|宽x高[@刷新率]|default` 可改，存在 host.json 的 `headless_display`），
  不占主控的上限；它不属于任何会话，最后一个会话结束 10 秒后仍没有新会话就删掉。
- 消息变体都追加在枚举最后（`MgmtReq` 第 11、12 号，`MgmtResp::Display` 第 7 号），老版本的编码不变；老被控端解不出新请求会直接关流。
- Windows 实现要点：驱动句柄放在会话里的画面代理里，代理退出或崩溃（服务停止、控制台会话切换）时驱动随句柄删屏，不会留下幽灵屏；
  控制台会话切换后代理换会话重启，原来的虚拟屏随之消失，主控要重新加（无头兜底屏在下次列屏时自动补）。
- macOS 实现要点：虚拟屏由有窗口服务器连接的进程建并持有（主控端、菜单栏被控端；无人值守时是负责画面的会话代理，root 后台服务不建），
  进程退出系统即回收；每块屏只有要求的那一个模式（非 HiDPI）。无人值守下换会话时旧代理删掉自己的屏，同样要主控重新加。
