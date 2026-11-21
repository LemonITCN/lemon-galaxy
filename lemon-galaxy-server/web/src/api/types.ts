// 管理后台接口的数据结构，字段与 docs/api.md 第一部分逐一对应（snake_case 原样保留）。

export interface Page<T> {
  items: T[]
  total: number
}

export interface ItemsOf<T> {
  items: T[]
}

export interface OkResult {
  ok: boolean
}

// ---------- 初始化与登录 ----------

export interface SetupStatus {
  needs_setup: boolean
}

export type AdminStage = 'done' | 'totp_setup'

export interface AdminUser {
  id: number
  username: string
  totp_enabled: boolean
  created_at: number
  last_login_at: number
}

export interface LoginResult {
  stage: AdminStage
  csrf_token: string
  admin?: AdminUser
}

export interface MeResult {
  admin: AdminUser | null
  csrf_token: string
  stage: AdminStage
}

export interface TotpSetupResult {
  secret: string
  otpauth_uri: string
}

// ---------- 概览 ----------

export interface ServerInfo {
  version: string
  started_at: number
  address: string
  fingerprint: string
  public_host: string
  port: number
  users: number
  devices: number
  online_devices: number
  active_connections: number
  relay_bytes_total: number
  db_size_bytes: number
}

export interface TimelinePoint {
  day: string
  connections: number
  relay_bytes: number
  ai_tokens: number
  logins: number
}

// ---------- 用户 ----------

export const AI_MODES = ['off', 'read_only', 'confirm_all', 'confirm_dangerous', 'full_trust'] as const
export type AiMode = (typeof AI_MODES)[number]

export interface User {
  id: number
  username: string
  display_name: string
  note: string
  disabled: boolean
  must_change_password: boolean
  totp_enabled: boolean
  require_totp: boolean
  account_initialized: boolean
  account_epoch: number
  ai_enabled: boolean
  ai_max_mode: AiMode
  ai_daily_token_limit: number
  ai_monthly_token_limit: number
  device_count: number
  online_device_count: number
  created_at: number
  last_login_at: number
}

export interface UserCreate {
  username: string
  display_name?: string
  initial_password: string
  note?: string
  require_totp?: boolean
  ai_enabled?: boolean
  ai_max_mode?: AiMode
}

export type UserPatch = Partial<
  Pick<
    User,
    | 'display_name'
    | 'note'
    | 'disabled'
    | 'require_totp'
    | 'ai_enabled'
    | 'ai_max_mode'
    | 'ai_daily_token_limit'
    | 'ai_monthly_token_limit'
  >
>

export interface AiGrant {
  provider_id: number
  /** ["*"] 表示该供应商全部模型 */
  models: string[]
}

// ---------- 设备 ----------

export const DEVICE_KINDS = ['control', 'worker', 'mobile'] as const
export type DeviceKind = (typeof DEVICE_KINDS)[number]

export const PLATFORMS = ['linux', 'macos', 'windows', 'android', 'ios', 'harmonyos'] as const
export type Platform = (typeof PLATFORMS)[number]

export type DeviceAccountStatus = 'active' | 'needs_relogin' | 'revoked'

export interface DeviceAccount {
  user_id: number
  username: string
  status: DeviceAccountStatus
  added_at: number
}

export interface Device {
  id: string
  kind: DeviceKind
  platform: Platform
  os_version: string
  arch: string
  hostname: string
  app_version: string
  online: boolean
  last_seen_at: number
  last_ip: string
  assist_mode: boolean
  accounts: DeviceAccount[]
  created_at: number
}

export interface DeviceQuery {
  q?: string
  user_id?: number
  online?: boolean
  kind?: DeviceKind
  page?: number
  page_size?: number
}

// ---------- 当前连接 ----------

export type ConnectionKind = 'control' | 'assist'
export const CONNECTION_PATHS = ['connecting', 'lan', 'direct', 'relay_udp', 'relay_tcp', 'closed'] as const
export type ConnectionPath = (typeof CONNECTION_PATHS)[number]

export interface Connection {
  id: string
  kind: ConnectionKind
  user_id: number
  username: string
  from_device_id: string
  from_device_name: string
  to_device_id: string
  to_device_name: string
  path: ConnectionPath
  started_at: number
  relay_bytes: number
}

// ---------- 大模型 ----------

export type AiProviderKind = 'opencode_go' | 'openai_compatible'

export interface AiModel {
  id: string
  name: string
  vision: boolean
  tools: boolean
  context_length: number
  enabled: boolean
}

export interface AiProvider {
  id: number
  name: string
  kind: AiProviderKind
  base_url: string
  has_api_key: boolean
  enabled: boolean
  models: AiModel[]
  models_updated_at: number
  created_at: number
}

export interface AiProviderCreate {
  name: string
  kind: AiProviderKind
  base_url?: string
  api_key: string
  enabled?: boolean
}

export interface AiProviderPatch {
  name?: string
  base_url?: string
  api_key?: string
  enabled?: boolean
}

export interface AiTestResult {
  ok: boolean
  latency_ms: number
  reply: string
}

export type UsageGroupBy = 'user' | 'model' | 'day'

export interface UsageRow {
  key: string
  requests: number
  prompt_tokens: number
  completion_tokens: number
  total_tokens: number
}

export interface UsageQuery {
  from?: number
  to?: number
  user_id?: number
  group_by: UsageGroupBy
}

// ---------- 审计日志 ----------

export const ACTOR_TYPES = ['admin', 'user', 'device', 'system'] as const
export type ActorType = (typeof ACTOR_TYPES)[number]

export const AUDIT_ACTIONS = [
  'admin.login',
  'admin.login_failed',
  'admin.logout',
  'admin.create',
  'admin.delete',
  'admin.password',
  'admin.totp_bind',
  'user.create',
  'user.update',
  'user.delete',
  'user.reset_password',
  'user.reset_totp',
  'user.revoke_sessions',
  'user.login',
  'user.login_failed',
  'user.locked',
  'user.password_change',
  'device.register',
  'device.revoke',
  'device.delete',
  'connection.start',
  'connection.end',
  'ai.provider_create',
  'ai.provider_update',
  'ai.provider_delete',
  'ai.grants_update',
  'settings.update',
  'update.upload',
] as const
export type AuditAction = (typeof AUDIT_ACTIONS)[number]

export interface AuditEntry {
  id: number
  at: number
  actor_type: ActorType
  actor_id: string
  actor_name: string
  action: string
  target_type: string
  target_id: string
  target_name: string
  ip: string
  detail: Record<string, unknown> | null
}

export interface AuditQuery {
  from?: number
  to?: number
  actor_type?: ActorType
  action?: string
  q?: string
  page?: number
  page_size?: number
}

// ---------- 系统设置 ----------

export type UpdateSource = 'github' | 'manual'

export interface Settings {
  server_name: string
  public_host: string
  relay_enabled: boolean
  relay_session_kbps: number
  relay_total_mbps: number
  login_lockout_threshold: number
  login_lockout_minutes: number
  session_idle_days: number
  audit_retention_days: number
  admin_ip_allowlist: string[]
  update_source: UpdateSource
  update_github_repo: string
  update_check_hours: number
}

// ---------- 更新包 ----------

export interface UpdatePackage {
  id: number
  component: string
  platform: string
  arch: string
  version: string
  file_name: string
  size: number
  sha256: string
  signature_ok: boolean
  source: UpdateSource
  created_at: number
}

export interface UpdateList {
  items: UpdatePackage[]
  last_check_at: number
  last_error: string
}
