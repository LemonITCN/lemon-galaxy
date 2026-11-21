// 仅开发用：内存假数据（npm run dev:mock）。生产构建不会打包这个目录。
import type {
  AdminUser,
  AiGrant,
  AiProvider,
  AuditEntry,
  Connection,
  Device,
  DeviceKind,
  Platform,
  Settings,
  UpdatePackage,
  User,
} from '@/api/types'
import { AUDIT_ACTIONS } from '@/api/types'

// 固定种子的伪随机数，保证每次刷新数据一致
let seed = 20261121
export function rnd(): number {
  seed = (seed * 1664525 + 1013904223) % 4294967296
  return seed / 4294967296
}
export const pick = <T>(arr: readonly T[]): T => arr[Math.floor(rnd() * arr.length)]
export const int = (min: number, max: number) => Math.floor(min + rnd() * (max - min + 1))

const NOW = Date.now()
const DAY = 86_400_000
const HOUR = 3_600_000

export const hex = (n: number) => Array.from({ length: n }, () => '0123456789abcdef'[int(0, 15)]).join('')

export interface MockAdmin extends AdminUser {
  password: string
}

export const db = {
  needsSetup: false,
  admins: [] as MockAdmin[],
  users: [] as User[],
  grants: new Map<number, AiGrant[]>(),
  devices: [] as Device[],
  connections: [] as Connection[],
  providers: [] as AiProvider[],
  audit: [] as AuditEntry[],
  settings: {} as Settings,
  updates: [] as UpdatePackage[],
  updatesLastCheck: NOW - 3 * HOUR,
  updatesLastError: '',
  startedAt: NOW - 3 * DAY - 5 * HOUR - 17 * 60_000,
  relayBytesTotal: 0,
  nextId: 1000,
}

export const MOCK_ADMIN_PASSWORD = 'galaxy-admin-123'
export const MOCK_SETUP_TOKEN = 'mock-setup-token'
export const MOCK_TOTP = '123456'

// ---------- 管理员 ----------
db.admins = [
  { id: 1, username: 'admin', password: MOCK_ADMIN_PASSWORD, totp_enabled: true, created_at: NOW - 40 * DAY, last_login_at: NOW - 2 * HOUR },
  { id: 2, username: 'ops', password: MOCK_ADMIN_PASSWORD, totp_enabled: true, created_at: NOW - 20 * DAY, last_login_at: NOW - 3 * DAY },
  { id: 3, username: 'newbie', password: MOCK_ADMIN_PASSWORD, totp_enabled: false, created_at: NOW - 1 * DAY, last_login_at: 0 },
]

// ---------- 用户 ----------
const NAMES: [string, string][] = [
  ['alice', 'Alice Chen'],
  ['bob', 'Bob'],
  ['carol', '王卡萝'],
  ['dave', 'Dave Li'],
  ['eve', 'Eve'],
  ['frank', '张峰'],
  ['grace', 'Grace Wu'],
  ['heidi', '李海迪'],
  ['ivan', 'Ivan'],
  ['judy', 'Judy Zhao'],
  ['mallory', ''],
  ['niaj', 'Niaj'],
  ['olivia', '刘奥丽'],
  ['peggy', 'Peggy'],
  ['rupert', 'Rupert Sun'],
  ['sybil', 'Sybil'],
  ['trent', '陈特伦'],
  ['victor', 'Victor'],
  ['walter', 'Walter Ma'],
  ['zoe', '周佐伊'],
  ['ops.bot', 'Ops Bot'],
  ['build-runner', 'CI 构建'],
  ['lab_01', '实验室 01'],
  ['lab_02', '实验室 02'],
]
const MODES = ['off', 'read_only', 'confirm_all', 'confirm_dangerous', 'confirm_dangerous', 'full_trust'] as const

db.users = NAMES.map(([username, display], i) => {
  const initialized = i % 7 !== 3
  return {
    id: i + 1,
    username,
    display_name: display,
    note: i % 5 === 0 ? '研发部' : i === 7 ? '外包，项目结束后禁用' : '',
    disabled: i === 10 || i === 17,
    must_change_password: !initialized,
    totp_enabled: i % 3 === 0,
    require_totp: i % 4 === 0,
    account_initialized: initialized,
    account_epoch: i % 6 === 1 ? 2 : 1,
    ai_enabled: i % 6 !== 5,
    ai_max_mode: MODES[i % MODES.length],
    ai_daily_token_limit: i % 4 === 0 ? 200_000 : 0,
    ai_monthly_token_limit: i % 3 === 0 ? 5_000_000 : 0,
    device_count: 0,
    online_device_count: 0,
    created_at: NOW - (60 - i * 2) * DAY,
    last_login_at: initialized ? NOW - int(1, 72) * HOUR : 0,
  }
})

// ---------- 大模型供应商 ----------
db.providers = [
  {
    id: 1,
    name: 'OpenCode Go',
    kind: 'opencode_go',
    base_url: 'https://opencode.ai/zen/go/v1',
    has_api_key: true,
    enabled: true,
    models: [
      { id: 'glm-5.2', name: 'GLM-5.2', vision: false, tools: true, context_length: 200000, enabled: true },
      { id: 'kimi-k2.6', name: 'Kimi K2.6', vision: true, tools: true, context_length: 256000, enabled: true },
      { id: 'qwen3.6-coder', name: 'Qwen3.6 Coder', vision: false, tools: true, context_length: 1000000, enabled: true },
      { id: 'deepseek-v4', name: 'DeepSeek V4', vision: false, tools: true, context_length: 128000, enabled: true },
      { id: 'minimax-m3', name: 'MiniMax M3', vision: true, tools: true, context_length: 1000000, enabled: false },
    ],
    models_updated_at: NOW - 5 * HOUR,
    created_at: NOW - 30 * DAY,
  },
  {
    id: 2,
    name: '内网 vLLM',
    kind: 'openai_compatible',
    base_url: 'http://10.0.8.20:8000/v1',
    has_api_key: true,
    enabled: true,
    models: [
      { id: 'qwen3-32b-instruct', name: 'qwen3-32b-instruct', vision: false, tools: true, context_length: 131072, enabled: true },
      { id: 'qwen3-vl-8b', name: 'qwen3-vl-8b', vision: true, tools: false, context_length: 32768, enabled: true },
    ],
    models_updated_at: NOW - 2 * DAY,
    created_at: NOW - 12 * DAY,
  },
]

db.users.forEach((u, i) => {
  if (!u.ai_enabled) return
  if (i % 3 === 0) db.grants.set(u.id, [{ provider_id: 1, models: ['*'] }, { provider_id: 2, models: ['*'] }])
  else if (i % 3 === 1) db.grants.set(u.id, [{ provider_id: 1, models: ['glm-5.2', 'kimi-k2.6'] }])
  else db.grants.set(u.id, [{ provider_id: 1, models: ['*'] }])
})

// ---------- 设备 ----------
const HOSTS: { kind: DeviceKind; platform: Platform; os: string; arch: string; host: string }[] = [
  { kind: 'worker', platform: 'linux', os: 'Ubuntu 24.04 LTS', arch: 'x86_64', host: 'web-01' },
  { kind: 'worker', platform: 'linux', os: 'Ubuntu 24.04 LTS', arch: 'x86_64', host: 'web-02' },
  { kind: 'worker', platform: 'linux', os: 'Debian 13', arch: 'aarch64', host: 'db-primary' },
  { kind: 'worker', platform: 'linux', os: 'Rocky Linux 9.6', arch: 'x86_64', host: 'build-04' },
  { kind: 'worker', platform: 'linux', os: 'Alpine 3.22', arch: 'aarch64', host: 'edge-gw' },
  { kind: 'control', platform: 'macos', os: 'macOS 27.2', arch: 'arm64', host: 'Alice-MacBook-Pro' },
  { kind: 'control', platform: 'macos', os: 'macOS 26.4', arch: 'arm64', host: 'design-studio' },
  { kind: 'control', platform: 'windows', os: 'Windows 11 24H2', arch: 'x86_64', host: 'DESKTOP-7QK2M1' },
  { kind: 'worker', platform: 'windows', os: 'Windows Server 2022', arch: 'x86_64', host: 'WIN-FILESRV' },
  { kind: 'worker', platform: 'windows', os: 'Windows 7 SP1', arch: 'x86', host: 'LEGACY-POS-03' },
  { kind: 'worker', platform: 'macos', os: 'macOS 15.6', arch: 'x86_64', host: 'mac-mini-ci' },
  { kind: 'mobile', platform: 'android', os: 'Android 16', arch: 'arm64-v8a', host: 'Xiaomi 17 Pro' },
  { kind: 'mobile', platform: 'android', os: 'Android 15', arch: 'arm64-v8a', host: 'Pixel 10' },
  { kind: 'mobile', platform: 'ios', os: 'iOS 27.1', arch: 'arm64', host: 'iPhone' },
  { kind: 'mobile', platform: 'ios', os: 'iPadOS 27.1', arch: 'arm64', host: 'iPad Pro' },
  { kind: 'mobile', platform: 'harmonyos', os: 'HarmonyOS 6.0', arch: 'arm64', host: 'Mate 80' },
]

for (let i = 0; i < 46; i++) {
  const h = HOSTS[i % HOSTS.length]
  const n = Math.floor(i / HOSTS.length)
  const online = rnd() < 0.55
  const owners = new Set<number>()
  owners.add(int(1, 12))
  if (h.kind === 'worker' && rnd() < 0.4) owners.add(int(1, 24))
  if (h.kind === 'worker' && rnd() < 0.2) owners.add(int(1, 24))
  const assist = h.kind === 'worker' && i === 9
  db.devices.push({
    id: hex(32),
    kind: h.kind,
    platform: h.platform,
    os_version: h.os,
    arch: h.arch,
    hostname: n ? `${h.host}-${n + 1}` : h.host,
    app_version: pick(['0.1.0.38', '0.1.0.41', '0.1.0.42']),
    online,
    last_seen_at: online ? NOW - int(5, 50) * 1000 : NOW - int(1, 240) * HOUR,
    last_ip: h.kind === 'mobile' ? `223.104.${int(1, 254)}.${int(1, 254)}` : `10.0.${int(1, 9)}.${int(2, 250)}`,
    assist_mode: assist,
    accounts: assist
      ? []
      : [...owners].map((uid, k) => ({
          user_id: uid,
          username: db.users[uid - 1].username,
          status: db.users[uid - 1].account_epoch > 1 && k === 0 && i % 4 === 0 ? 'needs_relogin' : i % 13 === 0 && k === 1 ? 'revoked' : 'active',
          added_at: NOW - int(2, 50) * DAY,
        })),
    created_at: NOW - int(3, 55) * DAY,
  })
}

export function recountDevices() {
  for (const u of db.users) {
    const mine = db.devices.filter((d) => d.accounts.some((a) => a.user_id === u.id && a.status !== 'revoked'))
    u.device_count = mine.length
    u.online_device_count = mine.filter((d) => d.online).length
  }
}
recountDevices()

// ---------- 当前连接 ----------
const PATHS = ['lan', 'direct', 'direct', 'relay_udp', 'relay_tcp', 'connecting'] as const
const controllers = db.devices.filter((d) => d.online && d.kind !== 'worker')
const targets = db.devices.filter((d) => d.online && d.kind !== 'mobile')
for (let i = 0; i < 7; i++) {
  const from = controllers[i % controllers.length]
  const to = targets[(i * 3 + 1) % targets.length]
  if (!from || !to) break
  const acc = from.accounts[0]
  const path = PATHS[i % PATHS.length]
  db.connections.push({
    id: hex(16),
    kind: i === 5 ? 'assist' : 'control',
    user_id: acc?.user_id ?? 0,
    username: acc?.username ?? '',
    from_device_id: from.id,
    from_device_name: from.hostname,
    to_device_id: to.id,
    to_device_name: to.hostname,
    path,
    started_at: NOW - int(1, 300) * 60_000,
    relay_bytes: path.startsWith('relay') ? int(2, 900) * 1024 * 1024 : 0,
  })
}

// ---------- 审计日志 ----------
const IPS = ['203.0.113.7', '198.51.100.24', '10.0.3.15', '192.0.2.66', '10.0.1.8']
let auditAt = NOW - 4 * 60_000
for (let i = 0; i < 260; i++) {
  auditAt -= int(3, 160) * 60_000
  const action = AUDIT_ACTIONS[int(0, AUDIT_ACTIONS.length - 1)]
  const [group] = action.split('.')
  const u = pick(db.users)
  const d = pick(db.devices)
  const admin = pick(db.admins.slice(0, 2))
  let actor_type: AuditEntry['actor_type'] = 'admin'
  let actor_id = String(admin.id)
  let actor_name = admin.username
  let target_type = group
  let target_id = ''
  let target_name = ''
  let detail: Record<string, unknown> | null = null
  if (action.startsWith('user.login') || action === 'user.password_change' || action === 'user.locked') {
    actor_type = action === 'user.locked' ? 'system' : 'user'
    actor_id = String(u.id)
    actor_name = action === 'user.locked' ? 'system' : u.username
    target_type = 'user'
    target_id = String(u.id)
    target_name = u.username
    if (action === 'user.login') detail = { device_id: d.id, platform: d.platform }
    if (action === 'user.login_failed') detail = { reason: 'invalid_credentials', attempts: int(1, 5) }
    if (action === 'user.locked') detail = { minutes: 15 }
  } else if (group === 'user') {
    target_type = 'user'
    target_id = String(u.id)
    target_name = u.username
    if (action === 'user.update') detail = { fields: ['ai_max_mode', 'ai_daily_token_limit'] }
  } else if (group === 'device') {
    actor_type = action === 'device.register' ? 'device' : 'admin'
    if (actor_type === 'device') {
      actor_id = d.id
      actor_name = d.hostname
    }
    target_type = 'device'
    target_id = d.id
    target_name = d.hostname
    if (action === 'device.revoke') detail = { user_id: u.id }
  } else if (group === 'connection') {
    actor_type = 'user'
    actor_id = String(u.id)
    actor_name = u.username
    target_type = 'connection'
    target_id = hex(16)
    target_name = `${pick(db.devices).hostname} → ${d.hostname}`
    detail = action === 'connection.end' ? { path: pick(PATHS), duration_ms: int(1, 7200) * 1000, relay_bytes: int(0, 500) * 1048576 } : { path: 'connecting' }
  } else if (group === 'ai') {
    target_type = action === 'ai.grants_update' ? 'user' : 'ai_provider'
    target_id = action === 'ai.grants_update' ? String(u.id) : '1'
    target_name = action === 'ai.grants_update' ? u.username : 'OpenCode Go'
  } else if (group === 'settings') {
    target_type = 'settings'
    target_name = ''
    detail = { fields: ['relay_session_kbps'] }
  } else if (group === 'update') {
    target_type = 'update'
    target_id = String(int(1, 9))
    target_name = 'worker-linux 0.1.0.42'
  } else if (group === 'admin') {
    target_type = 'admin'
    target_id = String(admin.id)
    target_name = admin.username
    if (action === 'admin.login_failed') detail = { reason: pick(['invalid_credentials', 'totp_invalid']) }
  }
  db.audit.push({
    id: 5000 - i,
    at: auditAt,
    actor_type,
    actor_id,
    actor_name,
    action,
    target_type,
    target_id,
    target_name,
    ip: actor_type === 'system' ? '' : pick(IPS),
    detail,
  })
}

// ---------- 系统设置 ----------
db.settings = {
  server_name: 'LemonGalaxy',
  public_host: 'galaxy.example.com',
  relay_enabled: true,
  relay_session_kbps: 0,
  relay_total_mbps: 200,
  login_lockout_threshold: 5,
  login_lockout_minutes: 15,
  session_idle_days: 30,
  audit_retention_days: 180,
  admin_ip_allowlist: [],
  update_source: 'github',
  update_github_repo: 'LemonITCN/lemon-galaxy',
  update_check_hours: 6,
}

// ---------- 更新包 ----------
const COMPONENTS: [string, string, string][] = [
  ['worker-linux', 'linux', 'x86_64'],
  ['worker-linux', 'linux', 'aarch64'],
  ['control-macos', 'macos', 'universal'],
  ['control-windows', 'windows', 'x86_64'],
  ['worker-windows', 'windows', 'x86_64'],
  ['mobile-android', 'android', 'arm64-v8a'],
]
db.updates = COMPONENTS.flatMap(([component, platform, arch], i) =>
  ['0.1.0.41', '0.1.0.42'].map((version, k) => ({
    id: i * 2 + k + 1,
    component,
    platform,
    arch,
    version,
    file_name: `lemon-galaxy-${component}-${version}-${arch}${platform === 'macos' ? '.dmg' : platform === 'windows' ? '.exe' : platform === 'android' ? '.apk' : '.tar.gz'}`,
    size: int(6, 90) * 1024 * 1024 + int(0, 1_000_000),
    sha256: hex(64),
    signature_ok: !(i === 4 && k === 1),
    source: 'github' as const,
    created_at: NOW - (k ? 2 : 9) * DAY - i * HOUR,
  })),
)

db.relayBytesTotal = 186 * 1024 ** 3 + 734 * 1024 ** 2

export function nextId() {
  db.nextId += 1
  return db.nextId
}

export { NOW, DAY, HOUR }
