# 本机 MCP 服务

桌面客户端（Mac、Windows 主控端）可以开一个只听本机的 MCP 服务，让这台电脑上的外部智能体
（Claude Code、Codex、Cursor、Claude Desktop 等）像内置 AI 一样操作账号里的设备、SSH 主机和本机：
远程桌面（截图、键鼠、控件树）、后台操作应用、命令与交互终端、文件读写与传输、远程开机等。

```
外部智能体 ──HTTP（127.0.0.1，Bearer 令牌）──> galaxy-mcp（协议） ──> galaxy-agent Agent::call_external
   或 stdio ──> LemonGalaxy --mcp-stdio（桥接）──┘          │  目标确认 / 风险判定 / 档位 / 确认卡片（同内置 AI）
                                                           └──> 端到端会话 ──> 被控端（机主上限 + 绝对禁止，最终判定）
```

外部智能体只是代替了内置 AI 的大模型循环：工具、风险判定（docs/ai-agent.md 第 5 节）、确认卡片、被控端的强制检查一模一样。

## 1. 用户拍板的口径

- 默认关闭，用户在客户端「MCP」页打开。默认端口 47821（固定，外部配置不用跟着变），可改。
- 默认档位「危险操作确认」，可在 MCP 页改（只读 / 每步确认 / 危险操作确认 / 完全信任可限时），立即作用于所有外部客户端。
- **不受服务器管理员的 AI 开关与上限限制**（账号没开通 AI、没开通模型也能用）；被控端机主给账号设的 AI 上限和
  绝对禁止清单照旧生效（被控端强制执行）。
- 「本机」可以用，但不预先授权：第一次用时和其他设备一样弹「连接」确认。
- 批准只在 LemonGalaxy 界面里做，不交给外部客户端（外部客户端可能自动同意）。

## 2. 协议（galaxy-mcp）

只监听 `127.0.0.1`，端点 `POST /mcp`。同一端点服务两代 MCP：

| | 2026-07-28（无状态） | 2025-11-25 / 2025-06-18 / 2025-03-26 / 2024-11-05（有会话） |
|---|---|---|
| 识别 | `params._meta["io.modelcontextprotocol/protocolVersion"]` | `initialize` 请求 |
| 发现 | `server/discover` → `resultType`、`supportedVersions`、`capabilities.tools`、`instructions`、`ttlMs`、`cacheScope`、`_meta.serverInfo` | `initialize` 协商版本（不认识的版本回 2025-11-25），响应头发 `Mcp-Session-Id` |
| 头 | `MCP-Protocol-Version` / `Mcp-Method` / `Mcp-Name` 与正文不一致 → 400 `-32020`；版本不支持 → 400 `-32022`，`data: {supported, requested}` | 之后的请求带 `Mcp-Session-Id`，不认识的会话 → 404；不带会话号的宽松处理（归到匿名客户端） |
| 结果 | 都带 `resultType: "complete"` 与 `_meta.serverInfo`；未知方法 HTTP 404 `-32601` | 原样；未知方法 HTTP 200 `-32601`；`ping` 回 `{}` |

- 通知一律 202。`GET /mcp` 回 405（不提供服务端推送流）；`DELETE /mcp` 带会话号时结束旧会话。
- `tools/call`：未知工具 `-32602`；执行失败、被拒绝、用户不同意都是 `isError: true` 的正常结果（文字说明原因）。
  内容是一段文字，截图等再附一个 `image` 块（base64 + mimeType）。
- 请求头 `Accept` 含 `text/event-stream` 时 `tools/call` 用 SSE 回：等待期间每 15 秒一条心跳注释，客户端给了
  `_meta.progressToken` 时改发 `notifications/progress`（提示可能在等用户批准）；最后一条是响应。
  **工具调用在 SSE 流里执行，客户端断开（新版规范里的取消方式）即取消**：挂着的确认被撤销，界面卡片标成「调用方已取消」。
- 需要批准的步骤一直挂着直到用户在 LemonGalaxy 里作答（不用 MRTR 把批准交给客户端）。

### 2.1 安全

- 鉴权：`Authorization: Bearer <令牌>`，常量时间比较；令牌 `lgm_` + 32 字节随机数（base64url），可在 MCP 页重新生成。
- 防 DNS 重绑定：`Host` 必须是 `127.0.0.1` / `localhost` / `[::1]`；带 `Origin` 的请求只接受本机页面，否则 403。
- 端点文件：数据目录 `mcp-endpoint.json`（`{url, token, pid}`，Unix 上 0600），stdio 桥接从这里读；关闭服务不删（令牌保留）。
  令牌对同一系统用户的其他程序可见，这与把令牌写进外部智能体配置文件是同一个信任边界。
- 输出与内置 AI 一样遮蔽私钥、云厂商密钥、令牌等，单次最多 64 KB。

### 2.2 客户端与智能体

- 按客户端名（新版 `_meta.clientInfo.name`，旧版 `initialize.clientInfo.name`，规整成小写字母数字与 `-`）归到一个智能体：
  确认过的设备、应用、「本客户端同类都允许」在它后续的调用里继续有效。
- 空闲 2 小时（没在执行、没有待批准）回收：设备会话断开，下次调用重新确认。
- 退出登录时所有外部客户端的智能体作废；服务继续开着，新调用回「还没有登录账号」。

## 3. 工具

与内置 AI 同名同参数（docs/ai-agent.md 第 4 节），外部格式见 `galaxy-mcp/src/tools.rs`（英文说明、`reason` 可选、
`readOnlyHint` / `destructiveHint` 标注）。外部智能体的使用说明（initialize / server/discover 的 `instructions`）在同一文件。

账号级：`list_devices`、`wake_device`、`list_snippets`、`list_port_forwards`、`recent_activity`、`wait`。
设备级（都带 `target`：list_devices 里的 id 或名称，本机为 `local`）：`system_info`、`run_command`、
`terminal_open/send/read/close`、`read_file`、`write_file`、`edit_file`、`list_dir`、`search_files`、
`upload_file` / `download_file`（本机绝对路径 ↔ 目标，单个最多 256 MB）、`list_displays`、`screenshot`、`ui_tree`、
`mouse`、`keyboard`、`request_secret`、`app_list/app_state/app_click/app_set_value/app_type/app_key/app_scroll/app_action/app_open`。

不开放：`update_plan`、`ask_user`、`web_fetch`（外部智能体有自己的）。

新增给外部智能体的三类（内置 AI 的 handle_tool 也认，但没放进它的工具清单）：
- `list_displays`：只读。
- `upload_file`：按「写入目标路径」判定（已存在算覆盖）；`download_file`：按「读目标路径」判定，本机文件已存在时升为危险。
  本机路径只收绝对路径或 `~` 开头。
- `wake_device`：只读档拒绝，其余直接发（唤醒包不改数据）。

## 4. FFI（galaxy-ffi `mcp.rs`）

接口在所有平台导出（各端绑定一致），手机上返回「不支持」（`McpStatus.supported = false`）。

```
mcp_default_port() -> u16                      // 47821
mcp_stdio_bridge(data_dir) -> i32              // --mcp-stdio 时调用，读标准输入写标准输出直到结束，返回退出码
GalaxyClient:
  async mcp_start(McpConfig { port, mode, trust_minutes }, McpListener) -> McpStatus   // 开启或按新设置重启
  mcp_stop()
  mcp_status() -> McpStatus
  async mcp_regenerate_token() -> McpStatus    // 开着时用新令牌重启，已配置的外部客户端要更新
  mcp_set_mode(mode, trust_minutes)            // 立即作用于所有外部客户端
  mcp_decide(client_key, call_id, ApprovalDecision)
  mcp_provide_secret(client_key, request_id, secret)
  mcp_stop_client(client_key)                  // 拒绝它等批准的步骤、停止正在跑的命令
  mcp_history(client_key) -> [AgentMessage]    // 最近 500 条操作记录（界面重开时补齐）
McpListener:
  on_status(McpStatus { supported, running, url, port, token, endpoint_file, mode, error, clients: [McpClientInfo] })
  on_event(client_key, AgentEvent)             // 与 AI 对话相同的事件：ToolProposed(needs_approval) / ToolStarted /
                                               // ToolOutput / ToolFinished / SecretRequested / Status / ModeChanged
McpClientInfo { key, name, version, last_active_ms, running, pending_approvals }
```

`ToolProposed { needs_approval: true }` 的 `call.tool` 可能是 `use_target`（第一次用某台设备）、`use_app`
（第一次操作某个应用），其余是工具名；`call_id` 用于 `mcp_decide`。

## 5. 界面（Mac、Windows 主控端）

侧栏「MCP」一项（在「AI 智能体」后面），有待批准时显示数字角标。页面自上而下：

1. **服务**：开关、状态（运行中 + 地址 / 未开启 / 错误原因，如端口被占用）、端口（默认 47821，改了点应用即重启）。
2. **待批准**（有才显示，置顶醒目）：每张卡片显示客户端名、目标设备、操作摘要、详情（完整命令、文件改动对比）、
   风险级别与原因、外部智能体给的理由；按钮「允许一次」「本客户端同类都允许」「拒绝」（可填理由回给智能体）。
   `request_secret` 显示安全输入框，提交走 `mcp_provide_secret`。
3. **连接方式**：地址、令牌（默认遮住，可显示 / 复制 / 重新生成，重新生成先确认），以及可一键复制的配置：
   - Claude Code：`claude mcp add --transport http --scope user lemongalaxy <url> --header "Authorization: Bearer <token>"`
   - Codex（`~/.codex/config.toml`）：
     ```toml
     [mcp_servers.lemongalaxy]
     url = "<url>"
     http_headers = { "Authorization" = "Bearer <token>" }
     ```
   - Cursor 等（JSON）：`{"mcpServers":{"lemongalaxy":{"url":"<url>","headers":{"Authorization":"Bearer <token>"}}}}`
   - Claude Desktop 等只支持 stdio 的：`{"mcpServers":{"lemongalaxy":{"command":"<本应用可执行文件绝对路径>","args":["--mcp-stdio"]}}}`
4. **权限**：档位（四档，完全信任可选时限），i 图标说明「被控端机主的上限与绝对禁止清单始终生效」。
5. **已连接的客户端**：名称、版本、最近活动时间、正在执行、待批准数，「停止」按钮（`mcp_stop_client`）。
6. **操作记录**：所有客户端最近的工具卡片（时间、客户端、目标、摘要、结果：成功 / 失败 / 已拒绝 / 已取消），最新在上，最多 200 条。

- 设置（开关、端口、档位、时限）存在客户端设置里；登录后若开着就自动 `mcp_start`；关闭开关 `mcp_stop`。
- 有新的待批准而窗口不在前台或不在 MCP 页：发系统通知（点开跳到 MCP 页），Mac 另外 `requestUserAttention`，
  Windows 闪任务栏按钮。
- 可执行文件带 `--mcp-stdio` 参数启动时不建界面，直接 `mcp_stdio_bridge(<数据目录>)` 后退出（数据目录与界面版相同）。

## 6. 测试

- `cargo test -p galaxy-mcp`：起真的本机 HTTP 服务（假执行器 + 假账号目录），覆盖两代协议、鉴权 401、Host / Origin 403、
  头不一致 / 版本不支持 / 未知方法 / 未知工具、设备首次使用批准、危险命令被拒、SSE 进度与断开即取消、文件下载与远程开机、stdio 桥接。
- 真实客户端：`cargo run -p galaxy-ffi --example mcp_serve -- <数据目录> <服务器地址> <用户> <密码> [--new-pass 新密码] [--port 端口] [--yes]`
  走真内核登录并开服务（打印每一步，`--yes` 自动同意、默认自动拒绝；`--stdio <数据目录>` 是 stdio 桥接）。
  再用临时配置跑 Claude Code，不改全局配置：
  `claude -p "…" --mcp-config <json> --strict-mcp-config --allowedTools mcp__lemongalaxy`
  （HTTP：`{"mcpServers":{"lemongalaxy":{"type":"http","url":…,"headers":{"Authorization":"Bearer …"}}}}`；
  stdio：`{"mcpServers":{"lemongalaxy":{"command":"…/mcp_serve","args":["--stdio","<数据目录>"]}}}`）。
  2026-11-25 实测（Claude Code 2.1.330）：HTTP 与 stdio 都通；列设备、本机首次使用批准、命令、系统概况、
  危险命令批准后执行、拒绝原因原样回到 Claude Code 且不重试。
