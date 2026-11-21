# 服务器接口

服务器只监听一个端口号（默认 7443）：TCP 上是 HTTPS（管理后台、客户端接口、WebSocket、TCP 中转），UDP 上是打洞探测与 UDP 中转（见 protocol.md）。

通用约定：

- 路径前缀 `/api/v1`，JSON，字段 snake_case。
- 时间一律 Unix **毫秒**（整数）。
- 二进制数据（密钥、密文、OPAQUE 消息）用 base64url 无填充字符串。
- 分页：`?page=1&page_size=20`，返回 `{ "items": [...], "total": 123 }`。
- 错误：HTTP 状态码 + `{ "error": { "code": "invalid_credentials", "message": "人类可读说明" } }`。
  常见 code：`bad_request`、`unauthorized`、`forbidden`、`not_found`、`conflict`、`rate_limited`、`locked`、`need_totp`、`totp_invalid`、`must_change_password`、`setup_required`、`internal`。
- 后台的 `message` 按请求头 `Accept-Language`（`zh-CN` / `en`）返回对应语言。

---

## 一、管理后台接口 `/api/v1/admin/*`

认证：登录成功后服务器下发 `lg_admin` Cookie（HttpOnly、Secure、SameSite=Strict），并在响应体里给出 `csrf_token`。
除 `GET` 外的所有请求都必须带请求头 `X-CSRF-Token: <csrf_token>`。刷新页面后用 `GET /admin/me` 重新拿 `csrf_token`。

### 1. 初始化（服务器第一次运行，还没有管理员）

服务器启动时如果没有管理员，会在日志里打印一次性初始化令牌和初始化网址 `https://<地址>/setup?token=<令牌>`。

- `GET /admin/setup/status` → `{ "needs_setup": true }`
- `POST /admin/setup` `{ "token": "...", "username": "admin", "password": "..." }`
  → `{ "ok": true }`。创建第一个管理员。之后必须登录并绑定两步验证。

### 2. 登录

- `POST /admin/login` `{ "username": "...", "password": "...", "totp_code": "123456"? }`
  - 密码正确但还没绑定两步验证：`200 { "stage": "totp_setup", "csrf_token": "..." }`（此时 Cookie 处于「待绑定」状态，只能调用 totp 绑定接口）
  - 需要两步验证码：`401 { "error": { "code": "need_totp" } }`，带上 `totp_code` 重试
  - 成功：`200 { "stage": "done", "csrf_token": "...", "admin": AdminUser }`
- `POST /admin/logout` → `{ "ok": true }`
- `GET /admin/me` → `{ "admin": AdminUser, "csrf_token": "...", "stage": "done" | "totp_setup" }`；未登录 401
- `POST /admin/totp/setup` → `{ "secret": "BASE32...", "otpauth_uri": "otpauth://totp/LemonGalaxy:admin?secret=...&issuer=LemonGalaxy" }`
- `POST /admin/totp/verify` `{ "code": "123456" }` → `{ "ok": true }`（绑定完成，Cookie 转为正式状态）
- `POST /admin/password` `{ "old_password": "...", "new_password": "..." }` → `{ "ok": true }`

`AdminUser`：
```json
{ "id": 1, "username": "admin", "totp_enabled": true, "created_at": 0, "last_login_at": 0 }
```

### 3. 管理员管理

- `GET /admin/admins` → `{ "items": [AdminUser] }`
- `POST /admin/admins` `{ "username", "password" }` → `AdminUser`（新管理员首次登录时必须绑定两步验证）
- `DELETE /admin/admins/{id}` → `{ "ok": true }`（不能删除自己，不能删除最后一个管理员）
- `POST /admin/admins/{id}/reset-totp` → `{ "ok": true }`

### 4. 概览

- `GET /admin/server-info` →
```json
{
  "version": "0.1.0", "started_at": 0,
  "address": "galaxy.example.com:7443#sha256-AbC...",   // 客户端填写的完整服务器地址（含指纹）
  "fingerprint": "sha256-AbC...",
  "public_host": "galaxy.example.com", "port": 7443,
  "users": 10, "devices": 25, "online_devices": 12,
  "active_connections": 3,
  "relay_bytes_total": 123456789,
  "db_size_bytes": 1048576
}
```
- `GET /admin/stats/timeline?days=7` → `{ "items": [{ "day": "2026-11-21", "connections": 10, "relay_bytes": 0, "ai_tokens": 0, "logins": 3 }] }`

### 5. 用户

`User`：
```json
{
  "id": 3, "username": "alice", "display_name": "Alice", "note": "",
  "disabled": false, "must_change_password": true,
  "totp_enabled": false, "require_totp": false,
  "account_initialized": false,      // 是否已在客户端完成首次登录（生成账号密钥）
  "account_epoch": 1,
  "ai_enabled": true,
  "ai_max_mode": "confirm_dangerous", // off | read_only | confirm_all | confirm_dangerous | full_trust
  "ai_daily_token_limit": 0,          // 0 = 不限
  "ai_monthly_token_limit": 0,
  "device_count": 2, "online_device_count": 1,
  "created_at": 0, "last_login_at": 0
}
```

- `GET /admin/users?q=&page=&page_size=` → `{ "items": [User], "total": n }`
- `POST /admin/users` `{ "username", "display_name"?, "initial_password", "note"?, "require_totp"?, "ai_enabled"?, "ai_max_mode"? }` → `User`
  - 用户名：3–32 位，字母数字和 `._-`，不区分大小写唯一。
- `GET /admin/users/{id}` → `User`
- `PATCH /admin/users/{id}` 任意可改字段：`display_name`、`note`、`disabled`、`require_totp`、`ai_enabled`、`ai_max_mode`、`ai_daily_token_limit`、`ai_monthly_token_limit` → `User`
- `POST /admin/users/{id}/reset-password` `{ "new_password" }` → `User`
  - 后台要明确提示：重置后该用户必须在所有设备上重新登录，被控端也要重新登录该账号。
- `POST /admin/users/{id}/reset-totp` → `{ "ok": true }`
- `POST /admin/users/{id}/revoke-sessions` → `{ "ok": true }`（注销该用户所有设备的登录会话）
- `DELETE /admin/users/{id}` → `{ "ok": true }`
- `GET /admin/users/{id}/ai-grants` → `{ "items": [AiGrant] }`
- `PUT /admin/users/{id}/ai-grants` `{ "items": [AiGrant] }` → `{ "items": [AiGrant] }`

`AiGrant`：`{ "provider_id": 1, "models": ["*"] }`（`"*"` 表示该供应商全部模型）

### 6. 设备

`Device`：
```json
{
  "id": "9f2c...（32 位十六进制）",
  "kind": "worker",                 // control | worker | mobile
  "platform": "linux",              // linux | macos | windows | android | ios | harmonyos
  "os_version": "Ubuntu 24.04", "arch": "x86_64",
  "hostname": "web-01", "app_version": "0.1.0",
  "online": true, "last_seen_at": 0, "last_ip": "1.2.3.4",
  "assist_mode": false,             // 正处于临时协助模式（未登录账号）
  "accounts": [ { "user_id": 3, "username": "alice", "status": "active", "added_at": 0 } ],
  "created_at": 0
}
```
`accounts[].status`：`active` | `needs_relogin`（账号密码被管理员重置后）| `revoked`

- `GET /admin/devices?q=&user_id=&online=true|false&kind=&page=&page_size=` → `{ "items": [Device], "total": n }`
- `GET /admin/devices/{id}` → `Device`
- `POST /admin/devices/{id}/revoke` `{ "user_id": 3 }` → `Device`（从某账号吊销该设备；不传 user_id 则从所有账号吊销）
- `DELETE /admin/devices/{id}` → `{ "ok": true }`（删除设备记录，同时吊销）

### 7. 当前连接

`Connection`：
```json
{
  "id": "c0ffee...", "kind": "control",     // control | assist
  "user_id": 3, "username": "alice",
  "from_device_id": "...", "from_device_name": "MacBook",
  "to_device_id": "...", "to_device_name": "web-01",
  "path": "lan",                           // connecting | lan | direct | relay_udp | relay_tcp | closed
  "started_at": 0, "relay_bytes": 0
}
```
- `GET /admin/connections` → `{ "items": [Connection] }`
- `POST /admin/connections/{id}/terminate` → `{ "ok": true }`

### 8. 大模型供应商

`AiProvider`：
```json
{
  "id": 1, "name": "OpenCode Go", "kind": "opencode_go",   // opencode_go | openai_compatible
  "base_url": "https://opencode.ai/zen/go/v1",
  "has_api_key": true, "enabled": true,
  "models": [ { "id": "glm-5.2", "name": "GLM-5.2", "vision": false, "tools": true, "context_length": 200000, "enabled": true } ],
  "models_updated_at": 0, "created_at": 0
}
```
- `GET /admin/ai/providers` → `{ "items": [AiProvider] }`
- `POST /admin/ai/providers` `{ "name", "kind", "base_url"?, "api_key", "enabled"? }` → `AiProvider`（创建后自动拉一次模型列表）
- `PATCH /admin/ai/providers/{id}` `{ "name"?, "base_url"?, "api_key"?, "enabled"? }` → `AiProvider`（`api_key` 只写不读）
- `PUT /admin/ai/providers/{id}/models` `{ "items": [{ "id": "...", "enabled": true }] }` → `AiProvider`（单独停用某些模型）
- `POST /admin/ai/providers/{id}/refresh-models` → `AiProvider`
- `POST /admin/ai/providers/{id}/test` `{ "model": "..." }` → `{ "ok": true, "latency_ms": 1234, "reply": "..." }`
- `DELETE /admin/ai/providers/{id}` → `{ "ok": true }`

### 9. AI 用量

- `GET /admin/ai/usage?from=&to=&user_id=&group_by=user|model|day` →
```json
{ "items": [ { "key": "alice", "requests": 10, "prompt_tokens": 1000, "completion_tokens": 200, "total_tokens": 1200 } ] }
```

### 10. 审计日志

`AuditEntry`：
```json
{
  "id": 1, "at": 0,
  "actor_type": "admin",          // admin | user | device | system
  "actor_id": "1", "actor_name": "admin",
  "action": "user.create",        // 见下
  "target_type": "user", "target_id": "3", "target_name": "alice",
  "ip": "1.2.3.4", "detail": { }
}
```
action 取值：`admin.login`、`admin.login_failed`、`admin.logout`、`admin.create`、`admin.delete`、`admin.password`、`admin.totp_bind`、
`user.create`、`user.update`、`user.delete`、`user.reset_password`、`user.reset_totp`、`user.revoke_sessions`、`user.login`、`user.login_failed`、`user.locked`、`user.password_change`、
`device.register`、`device.revoke`、`device.delete`、`connection.start`、`connection.end`、`ai.provider_create`、`ai.provider_update`、`ai.provider_delete`、`ai.grants_update`、`settings.update`、`update.upload`。

- `GET /admin/audit?from=&to=&actor_type=&action=&q=&page=&page_size=` → `{ "items": [AuditEntry], "total": n }`

### 11. 系统设置

`Settings`：
```json
{
  "server_name": "LemonGalaxy",
  "public_host": "galaxy.example.com",       // 写进客户端地址里的主机名或 IP
  "relay_enabled": true,
  "relay_session_kbps": 0,                    // 每个连接中转限速，0 = 不限
  "relay_total_mbps": 0,                      // 中转总带宽上限，0 = 不限
  "login_lockout_threshold": 5,
  "login_lockout_minutes": 15,
  "session_idle_days": 30,                    // 客户端多少天不上线自动注销
  "audit_retention_days": 180,
  "admin_ip_allowlist": [],                   // 为空不限制，例如 ["10.0.0.0/8"]
  "update_source": "github",                  // github | manual
  "update_github_repo": "LemonITCN/lemon-galaxy",
  "update_check_hours": 6
}
```
- `GET /admin/settings` → `Settings`
- `PATCH /admin/settings` 任意字段 → `Settings`

### 12. 更新包

`UpdatePackage`：
```json
{ "id": 1, "component": "worker-linux", "platform": "linux", "arch": "x86_64", "version": "0.1.0.42",
  "file_name": "...", "size": 0, "sha256": "...", "signature_ok": true, "source": "github", "created_at": 0 }
```
- `GET /admin/updates` → `{ "items": [UpdatePackage], "last_check_at": 0, "last_error": "" }`
- `POST /admin/updates/check` → 同上（立即从 GitHub 检查一次）
- `POST /admin/updates/upload`（multipart：`file`、`signature`）→ `UpdatePackage`
- `DELETE /admin/updates/{id}` → `{ "ok": true }`

---

## 二、客户端接口

认证：`Authorization: Bearer <session_token>`。

### 1. 服务器信息

- `GET /api/v1/info` → `{ "server_name", "version", "server_sign_pubkey": "b64", "min_client_version": "0.1.0", "features": ["ai","relay","assist"] }`

### 2. 登录（OPAQUE）

- `POST /api/v1/auth/login/start` `{ "username", "opaque_request": "b64" }`
  → `{ "login_id": "...", "opaque_response": "b64" }`
- `POST /api/v1/auth/login/finish`
  ```json
  { "login_id": "...", "opaque_finalization": "b64", "totp_code": "123456"?,
    "device": { "device_id": "hex32", "device_pubkey": "b64", "kind": "worker", "platform": "linux",
                "os_version": "...", "arch": "...", "hostname": "...", "app_version": "..." } }
  ```
  → 成功：
  ```json
  { "session_token": "...", "user": { "id": 3, "username": "alice", "display_name": "Alice", "ai_enabled": true },
    "must_change_password": false,
    "account": { "epoch": 1, "pubkey": "b64" | null, "wrapped_secrets": "b64" | null, "nonce": "b64" | null } }
  ```
  `account.wrapped_secrets` 为空表示账号还没初始化（首次登录），客户端必须走改密码流程并初始化账号密钥。
  失败：`401 invalid_credentials` / `401 need_totp` / `423 locked` / `403 disabled`。
- `POST /api/v1/auth/password/start`（Bearer）`{ "registration_request": "b64" }` → `{ "registration_response": "b64" }`
- `POST /api/v1/auth/password/finish`（Bearer）
  `{ "registration_upload": "b64", "account": { "epoch": 1, "pubkey": "b64", "wrapped_secrets": "b64", "nonce": "b64" } }` → `{ "ok": true }`
- `POST /api/v1/auth/totp/setup`、`/auth/totp/verify`、`/auth/totp/disable`（Bearer，用户自己管理两步验证）
- `POST /api/v1/auth/logout`（Bearer）

### 3. 设备

- `POST /api/v1/devices/certificate`（Bearer）`{ "cert": "b64(postcard DeviceCert)", "signature": "b64" }` → `{ "ok": true }`
  登录后上传本设备在本账号下的证书。服务器校验签名与账号公钥一致。
- `GET /api/v1/devices`（Bearer）→ `{ "items": [ClientDevice] }`，本账号下所有设备（含在线状态、证书）
  ```json
  { "id": "hex32", "kind": "worker", "platform": "linux", "os_version": "", "arch": "", "hostname": "web-01",
    "app_version": "", "online": true, "last_seen_at": 0, "cert": "b64", "cert_signature": "b64",
    "public_ip": "1.2.3.4", "last_ip": "1.2.3.4", "macs": ["AA:BB:CC:DD:EE:FF"] }
  ```
- `DELETE /api/v1/devices/{id}`（Bearer）→ 从本账号移除该设备（远程注销）
- `GET /api/v1/account/revocations`（Bearer）→ `{ "list": "b64", "signature": "b64" }`；`PUT` 同结构（由主控端用账号签名密钥签）

### 4. 加密库 vault

- `GET /api/v1/vault?since=<version>`（Bearer）→ `{ "items": [VaultItem], "version": 123 }`
- `PUT /api/v1/vault/{item_id}`（Bearer）`{ "kind": "ssh_host", "ciphertext": "b64", "nonce": "b64", "base_version": 12 }`
  → `{ "version": 13 }`；`base_version` 与服务器不一致时 `409 conflict`（客户端合并后重试）
- `DELETE /api/v1/vault/{item_id}`（Bearer）`?base_version=` → `{ "version": 14 }`

`VaultItem`：`{ "id": "uuid", "kind": "...", "ciphertext": "b64", "nonce": "b64", "version": 13, "deleted": false, "updated_at": 0 }`
kind：`folder`、`device_meta`、`ssh_host`、`rdp_host`、`credential`、`snippet`、`known_host`、`preferences`。明文结构见 ffi.md。

### 5. AI 网关

- `GET /api/v1/ai/models`（Bearer）→ `{ "enabled": true, "max_mode": "confirm_dangerous", "items": [{ "provider_id": 1, "provider_name": "OpenCode Go", "id": "glm-5.2", "name": "GLM-5.2", "vision": false, "tools": true, "context_length": 200000 }] }`
- `POST /api/v1/ai/chat/completions`（Bearer）OpenAI Chat Completions 兼容请求体，额外字段：
  `"lg_provider_id": 1`、`"lg_conversation_id": "ascii-id"`、`"lg_device_ids": ["..."]`（用于用量统计）。
  服务器校验授权与额度、注入 API Key 与供应商要求的请求头后原样转发；`stream: true` 时以 SSE 原样回传，并在流里读取 usage 计量。
  超额：`429 { "error": { "code": "quota_exceeded" } }`；未开通：`403 ai_disabled` / `403 model_not_granted`。

### 6. 自动更新

- `GET /api/v1/updates/latest?component=&platform=&arch=&current=` → `{ "available": true, "version": "...", "url": "/api/v1/updates/files/{id}", "size": 0, "sha256": "...", "signature": "b64" }`
- `GET /api/v1/updates/files/{id}` → 文件

### 7. WebSocket 信令 `GET /api/v1/ws`

请求头 `Authorization: Bearer <session_token>`；临时协助模式（未登录）用 `?assist=1&device_id=...`。
消息为 JSON 文本帧，字段 `t` 表示类型，细节见 protocol.md。

### 8. TCP 中转 `GET /api/v1/relay?token=<relay_token>`

WebSocket 二进制帧，每帧一个 UDP 数据报的内容。详见 protocol.md。
