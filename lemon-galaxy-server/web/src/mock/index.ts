// 仅开发用：按 docs/api.md 第一部分模拟管理后台接口（npm run dev:mock）。
// 只在开发模式且 VITE_MOCK=1（或 vite --mode mock）时由请求封装动态加载，生产构建不会打包。
//
// 账号：admin / galaxy-admin-123，两步验证码 123456
//       newbie / galaxy-admin-123（还没绑定两步验证，用来看强制绑定页）
// 想看初始化页：在浏览器控制台执行 sessionStorage.setItem('lg-mock-setup', '1') 后刷新，令牌 mock-setup-token
import type { MockRequest, MockResponse } from '@/api/http'
import type {
  AiGrant,
  AiProvider,
  AiProviderCreate,
  AiProviderPatch,
  Settings,
  UpdatePackage,
  UsageRow,
  User,
  UserCreate,
  UserPatch,
} from '@/api/types'
import { DAY, MOCK_SETUP_TOKEN, MOCK_TOTP, db, hex, int, nextId, recountDevices } from './data'

interface MockSession {
  adminId: number
  stage: 'done' | 'totp_setup'
  csrf: string
}

const SESSION_KEY = 'lg-mock-session'

function loadSession(): MockSession | null {
  try {
    const s = sessionStorage.getItem(SESSION_KEY)
    return s ? (JSON.parse(s) as MockSession) : null
  } catch {
    return null
  }
}
function saveSession(s: MockSession | null) {
  try {
    if (s) sessionStorage.setItem(SESSION_KEY, JSON.stringify(s))
    else sessionStorage.removeItem(SESSION_KEY)
  } catch {
    /* 忽略 */
  }
}

try {
  if (sessionStorage.getItem('lg-mock-setup') === '1') db.needsSetup = true
} catch {
  /* 忽略 */
}

let lang = 'zh-CN'
const L = (zh: string, en: string) => (lang.startsWith('zh') ? zh : en)

class HttpError {
  constructor(
    public status: number,
    public code: string,
    public message: string,
  ) {}
}
const fail = (status: number, code: string, zh: string, en: string): never => {
  throw new HttpError(status, code, L(zh, en))
}

const ok = (body: unknown = { ok: true }): MockResponse => ({ status: 200, body })
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

function page<T>(list: T[], q: URLSearchParams) {
  const p = Math.max(1, Number(q.get('page') ?? 1))
  const size = Math.max(1, Math.min(200, Number(q.get('page_size') ?? 20)))
  return { items: list.slice((p - 1) * size, p * size), total: list.length }
}

function adminPublic(a: (typeof db.admins)[number]) {
  const { password: _pw, ...rest } = a
  void _pw
  return rest
}

function findUser(id: number): User {
  const u = db.users.find((x) => x.id === id)
  if (!u) fail(404, 'not_found', '用户不存在', 'User not found')
  return u!
}

function findProvider(id: number): AiProvider {
  const p = db.providers.find((x) => x.id === id)
  if (!p) fail(404, 'not_found', '供应商不存在', 'Provider not found')
  return p!
}

function audit(action: string, target_type: string, target_id: string, target_name: string, adminId: number, detail: Record<string, unknown> | null = null) {
  const a = db.admins.find((x) => x.id === adminId)
  db.audit.unshift({
    id: nextId(),
    at: Date.now(),
    actor_type: 'admin',
    actor_id: String(adminId),
    actor_name: a?.username ?? '',
    action,
    target_type,
    target_id,
    target_name,
    ip: '127.0.0.1',
    detail,
  })
}

// 连接随时间变化：中转流量增长，偶尔切换路径
function tickConnections() {
  for (const c of db.connections) {
    if (c.path === 'relay_udp' || c.path === 'relay_tcp') c.relay_bytes += int(50, 900) * 1024
    if (c.path === 'connecting' && Math.random() < 0.3) c.path = Math.random() < 0.5 ? 'direct' : 'relay_udp'
  }
}

function usage(q: URLSearchParams): UsageRow[] {
  const from = Number(q.get('from') ?? Date.now() - 30 * DAY)
  const to = Number(q.get('to') ?? Date.now())
  const userId = q.get('user_id') ? Number(q.get('user_id')) : undefined
  const groupBy = q.get('group_by') ?? 'day'
  const models = db.providers.flatMap((p) => p.models.map((m) => m.id))
  const users = db.users.filter((u) => u.ai_enabled && (!userId || u.id === userId))
  const map = new Map<string, UsageRow>()
  // 用确定性的「伪事件」生成用量
  for (let t = from; t <= to; t += DAY) {
    const day = new Date(t)
    const key = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, '0')}-${String(day.getDate()).padStart(2, '0')}`
    const dseed = day.getDate() * 31 + day.getMonth() * 7
    users.forEach((u, ui) => {
      const reqs = (dseed + u.id * 13) % 9 === 0 ? 0 : ((dseed * (ui + 3) + u.id * 17) % 23) + 1
      if (!reqs) return
      const model = models[(dseed + u.id) % models.length]
      const prompt = reqs * (((dseed + u.id * 7) % 40) + 8) * 230
      const completion = Math.round(prompt * (0.12 + ((u.id % 5) * 0.03)))
      const k = groupBy === 'user' ? u.username : groupBy === 'model' ? model : key
      const row = map.get(k) ?? { key: k, requests: 0, prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 }
      row.requests += reqs
      row.prompt_tokens += prompt
      row.completion_tokens += completion
      row.total_tokens += prompt + completion
      map.set(k, row)
    })
  }
  return [...map.values()]
}

async function route(req: MockRequest): Promise<MockResponse> {
  const { method, path, query: q } = req
  const body = (req.body ?? {}) as Record<string, any>
  const seg = path.split('/').filter(Boolean)
  const M = (m: string, pattern: string) => {
    if (m !== method) return null
    const ps = pattern.split('/').filter(Boolean)
    if (ps.length !== seg.length) return null
    const params: Record<string, string> = {}
    for (let i = 0; i < ps.length; i++) {
      if (ps[i].startsWith(':')) params[ps[i].slice(1)] = decodeURIComponent(seg[i])
      else if (ps[i] !== seg[i]) return null
    }
    return params
  }

  // ---------- 不需要登录 ----------
  if (M('GET', '/setup/status')) return ok({ needs_setup: db.needsSetup })
  if (M('POST', '/setup')) {
    if (!db.needsSetup) fail(409, 'conflict', '服务器已经初始化过了', 'The server is already set up')
    if (body.token !== MOCK_SETUP_TOKEN) fail(401, 'unauthorized', '初始化令牌无效或已过期', 'Invalid or expired setup token')
    db.admins = [{ id: 1, username: body.username, password: body.password, totp_enabled: false, created_at: Date.now(), last_login_at: 0 }]
    db.needsSetup = false
    try {
      sessionStorage.removeItem('lg-mock-setup')
    } catch {
      /* 忽略 */
    }
    return ok()
  }
  if (db.needsSetup) fail(401, 'setup_required', '服务器还没有初始化', 'The server has not been set up yet')

  if (M('POST', '/login')) {
    const a = db.admins.find((x) => x.username.toLowerCase() === String(body.username ?? '').toLowerCase())
    if (!a || a.password !== body.password) fail(401, 'invalid_credentials', '用户名或密码错误', 'Incorrect username or password')
    if (a!.totp_enabled) {
      if (!body.totp_code) fail(401, 'need_totp', '需要两步验证码', 'Two-factor code required')
      if (body.totp_code !== MOCK_TOTP) fail(401, 'totp_invalid', '验证码错误，请重新输入', 'Incorrect code, try again')
    }
    const s: MockSession = { adminId: a!.id, stage: a!.totp_enabled ? 'done' : 'totp_setup', csrf: hex(32) }
    saveSession(s)
    a!.last_login_at = Date.now()
    if (s.stage === 'done') audit('admin.login', 'admin', String(a!.id), a!.username, a!.id)
    return ok({ stage: s.stage, csrf_token: s.csrf, admin: s.stage === 'done' ? adminPublic(a!) : undefined })
  }

  // ---------- 需要登录 ----------
  const session = loadSession()
  if (!session) fail(401, 'unauthorized', '请先登录', 'Please sign in')
  const s = session!
  const me = db.admins.find((x) => x.id === s.adminId)
  if (!me) {
    saveSession(null)
    fail(401, 'unauthorized', '请先登录', 'Please sign in')
  }
  if (method !== 'GET' && req.headers['X-CSRF-Token'] !== s.csrf) fail(403, 'forbidden', 'CSRF 校验失败', 'CSRF check failed')

  if (M('GET', '/me')) return ok({ admin: adminPublic(me!), csrf_token: s.csrf, stage: s.stage })
  if (M('POST', '/logout')) {
    saveSession(null)
    audit('admin.logout', 'admin', String(me!.id), me!.username, me!.id)
    return ok()
  }
  if (M('POST', '/totp/setup')) {
    const secret = Array.from({ length: 32 }, () => 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'[int(0, 31)]).join('')
    return ok({ secret, otpauth_uri: `otpauth://totp/LemonGalaxy:${me!.username}?secret=${secret}&issuer=LemonGalaxy` })
  }
  if (M('POST', '/totp/verify')) {
    if (!/^\d{6}$/.test(String(body.code ?? ''))) fail(400, 'totp_invalid', '验证码错误', 'Incorrect code')
    me!.totp_enabled = true
    s.stage = 'done'
    saveSession(s)
    audit('admin.totp_bind', 'admin', String(me!.id), me!.username, me!.id)
    return ok()
  }
  if (s.stage !== 'done') fail(403, 'forbidden', '请先绑定两步验证', 'Set up two-factor authentication first')

  if (M('POST', '/password')) {
    if (body.old_password !== me!.password) fail(400, 'invalid_credentials', '当前密码不正确', 'Current password is incorrect')
    me!.password = body.new_password
    audit('admin.password', 'admin', String(me!.id), me!.username, me!.id)
    return ok()
  }

  // 管理员
  if (M('GET', '/admins')) return ok({ items: db.admins.map(adminPublic) })
  if (M('POST', '/admins')) {
    if (db.admins.some((a) => a.username.toLowerCase() === String(body.username).toLowerCase()))
      fail(409, 'conflict', '用户名已存在', 'Username already exists')
    const a = { id: nextId(), username: body.username, password: body.password, totp_enabled: false, created_at: Date.now(), last_login_at: 0 }
    db.admins.push(a)
    audit('admin.create', 'admin', String(a.id), a.username, me!.id)
    return ok(adminPublic(a))
  }
  let p = M('DELETE', '/admins/:id')
  if (p) {
    const id = Number(p.id)
    if (id === me!.id) fail(400, 'bad_request', '不能删除自己', 'You cannot delete yourself')
    if (db.admins.length <= 1) fail(400, 'bad_request', '不能删除最后一个管理员', 'Cannot delete the last administrator')
    const a = db.admins.find((x) => x.id === id)
    db.admins = db.admins.filter((x) => x.id !== id)
    if (a) audit('admin.delete', 'admin', String(id), a.username, me!.id)
    return ok()
  }
  p = M('POST', '/admins/:id/reset-totp')
  if (p) {
    const a = db.admins.find((x) => x.id === Number(p!.id))
    if (a) a.totp_enabled = false
    return ok()
  }

  // 概览
  if (M('GET', '/server-info')) {
    const host = db.settings.public_host
    const fp = 'sha256-Qm9vT3JiaXRMZW1vbkdhbGF4eUZpbmdlcnByaW50MA'
    return ok({
      version: '0.1.0',
      started_at: db.startedAt,
      address: `${host || '0.0.0.0'}:7443#${fp}`,
      fingerprint: fp,
      public_host: host,
      port: 7443,
      users: db.users.length,
      devices: db.devices.length,
      online_devices: db.devices.filter((d) => d.online).length,
      active_connections: db.connections.length,
      relay_bytes_total: db.relayBytesTotal,
      db_size_bytes: 37_748_736,
    })
  }
  if (M('GET', '/stats/timeline')) {
    const days = Number(q.get('days') ?? 7)
    const items = Array.from({ length: days }, (_, i) => {
      const d = new Date(Date.now() - (days - 1 - i) * DAY)
      const day = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
      const w = d.getDay() === 0 || d.getDay() === 6 ? 0.45 : 1
      return {
        day,
        connections: Math.round((38 + ((i * 17) % 23)) * w),
        relay_bytes: Math.round((1.2 + ((i * 7) % 5) * 0.6) * w * 1024 ** 3),
        ai_tokens: Math.round((420 + ((i * 131) % 380)) * w * 1000),
        logins: Math.round((24 + ((i * 11) % 15)) * w),
      }
    })
    return ok({ items })
  }

  // 用户
  if (M('GET', '/users')) {
    const kw = (q.get('q') ?? '').toLowerCase()
    const list = db.users.filter((u) => !kw || u.username.includes(kw) || u.display_name.toLowerCase().includes(kw) || u.note.toLowerCase().includes(kw))
    return ok(page(list, q))
  }
  if (M('POST', '/users')) {
    const b = body as UserCreate
    if (!/^[A-Za-z0-9._-]{3,32}$/.test(b.username)) fail(400, 'bad_request', '用户名格式不正确', 'Invalid username')
    if (db.users.some((u) => u.username.toLowerCase() === b.username.toLowerCase())) fail(409, 'conflict', '用户名已存在', 'Username already exists')
    const u: User = {
      id: nextId(),
      username: b.username,
      display_name: b.display_name ?? '',
      note: b.note ?? '',
      disabled: false,
      must_change_password: true,
      totp_enabled: false,
      require_totp: !!b.require_totp,
      account_initialized: false,
      account_epoch: 1,
      ai_enabled: b.ai_enabled ?? true,
      ai_max_mode: b.ai_max_mode ?? 'confirm_dangerous',
      ai_daily_token_limit: 0,
      ai_monthly_token_limit: 0,
      device_count: 0,
      online_device_count: 0,
      created_at: Date.now(),
      last_login_at: 0,
    }
    db.users.unshift(u)
    audit('user.create', 'user', String(u.id), u.username, me!.id)
    return ok(u)
  }
  p = M('GET', '/users/:id')
  if (p) return ok(findUser(Number(p.id)))
  p = M('PATCH', '/users/:id')
  if (p) {
    const u = findUser(Number(p.id))
    const b = body as UserPatch
    Object.assign(u, b)
    audit('user.update', 'user', String(u.id), u.username, me!.id, { fields: Object.keys(b) })
    return ok(u)
  }
  p = M('POST', '/users/:id/reset-password')
  if (p) {
    const u = findUser(Number(p.id))
    u.must_change_password = true
    u.account_initialized = false
    u.account_epoch += 1
    for (const d of db.devices) for (const a of d.accounts) if (a.user_id === u.id && a.status === 'active') a.status = 'needs_relogin'
    audit('user.reset_password', 'user', String(u.id), u.username, me!.id)
    return ok(u)
  }
  p = M('POST', '/users/:id/reset-totp')
  if (p) {
    const u = findUser(Number(p.id))
    u.totp_enabled = false
    audit('user.reset_totp', 'user', String(u.id), u.username, me!.id)
    return ok()
  }
  p = M('POST', '/users/:id/revoke-sessions')
  if (p) {
    const u = findUser(Number(p.id))
    audit('user.revoke_sessions', 'user', String(u.id), u.username, me!.id)
    return ok()
  }
  p = M('DELETE', '/users/:id')
  if (p) {
    const u = findUser(Number(p.id))
    db.users = db.users.filter((x) => x.id !== u.id)
    for (const d of db.devices) d.accounts = d.accounts.filter((a) => a.user_id !== u.id)
    audit('user.delete', 'user', String(u.id), u.username, me!.id)
    return ok()
  }
  p = M('GET', '/users/:id/ai-grants')
  if (p) return ok({ items: db.grants.get(findUser(Number(p.id)).id) ?? [] })
  p = M('PUT', '/users/:id/ai-grants')
  if (p) {
    const u = findUser(Number(p.id))
    const items = (body.items ?? []) as AiGrant[]
    db.grants.set(u.id, items)
    audit('ai.grants_update', 'user', String(u.id), u.username, me!.id)
    return ok({ items })
  }

  // 设备
  if (M('GET', '/devices')) {
    const kw = (q.get('q') ?? '').toLowerCase()
    const uid = q.get('user_id') ? Number(q.get('user_id')) : undefined
    const online = q.get('online')
    const kind = q.get('kind')
    const list = db.devices
      .filter(
        (d) =>
          (!kw || d.hostname.toLowerCase().includes(kw) || d.id.includes(kw) || d.last_ip.includes(kw)) &&
          (!uid || d.accounts.some((a) => a.user_id === uid)) &&
          (online === null || String(d.online) === online) &&
          (!kind || d.kind === kind),
      )
      .sort((a, b) => Number(b.online) - Number(a.online) || b.last_seen_at - a.last_seen_at)
    return ok(page(list, q))
  }
  p = M('GET', '/devices/:id')
  if (p) {
    const d = db.devices.find((x) => x.id === p!.id)
    if (!d) fail(404, 'not_found', '设备不存在', 'Device not found')
    return ok(d)
  }
  p = M('POST', '/devices/:id/revoke')
  if (p) {
    const d = db.devices.find((x) => x.id === p!.id)
    if (!d) fail(404, 'not_found', '设备不存在', 'Device not found')
    for (const a of d!.accounts) if (body.user_id === undefined || a.user_id === body.user_id) a.status = 'revoked'
    recountDevices()
    audit('device.revoke', 'device', d!.id, d!.hostname, me!.id, body.user_id ? { user_id: body.user_id } : null)
    return ok(d)
  }
  p = M('DELETE', '/devices/:id')
  if (p) {
    const d = db.devices.find((x) => x.id === p!.id)
    db.devices = db.devices.filter((x) => x.id !== p!.id)
    recountDevices()
    if (d) audit('device.delete', 'device', d.id, d.hostname, me!.id)
    return ok()
  }

  // 当前连接
  if (M('GET', '/connections')) {
    tickConnections()
    return ok({ items: db.connections })
  }
  p = M('POST', '/connections/:id/terminate')
  if (p) {
    db.connections = db.connections.filter((c) => c.id !== p!.id)
    return ok()
  }

  // 大模型供应商
  if (M('GET', '/ai/providers')) return ok({ items: db.providers })
  if (M('POST', '/ai/providers')) {
    const b = body as AiProviderCreate
    const prov: AiProvider = {
      id: nextId(),
      name: b.name,
      kind: b.kind,
      base_url: b.base_url || (b.kind === 'opencode_go' ? 'https://opencode.ai/zen/go/v1' : ''),
      has_api_key: !!b.api_key,
      enabled: b.enabled ?? true,
      models:
        b.kind === 'opencode_go'
          ? [{ id: 'glm-5.2', name: 'GLM-5.2', vision: false, tools: true, context_length: 200000, enabled: true }]
          : [{ id: 'gpt-oss-120b', name: 'gpt-oss-120b', vision: false, tools: true, context_length: 131072, enabled: true }],
      models_updated_at: Date.now(),
      created_at: Date.now(),
    }
    db.providers.push(prov)
    audit('ai.provider_create', 'ai_provider', String(prov.id), prov.name, me!.id)
    return ok(prov)
  }
  p = M('PATCH', '/ai/providers/:id')
  if (p) {
    const prov = findProvider(Number(p.id))
    const b = body as AiProviderPatch
    if (b.name !== undefined) prov.name = b.name
    if (b.base_url !== undefined) prov.base_url = b.base_url
    if (b.enabled !== undefined) prov.enabled = b.enabled
    if (b.api_key) prov.has_api_key = true
    audit('ai.provider_update', 'ai_provider', String(prov.id), prov.name, me!.id)
    return ok(prov)
  }
  p = M('PUT', '/ai/providers/:id/models')
  if (p) {
    const prov = findProvider(Number(p.id))
    for (const it of body.items ?? []) {
      const m = prov.models.find((x) => x.id === it.id)
      if (m) m.enabled = !!it.enabled
    }
    return ok(prov)
  }
  p = M('POST', '/ai/providers/:id/refresh-models')
  if (p) {
    const prov = findProvider(Number(p.id))
    await sleep(500)
    prov.models_updated_at = Date.now()
    return ok(prov)
  }
  p = M('POST', '/ai/providers/:id/test')
  if (p) {
    const prov = findProvider(Number(p.id))
    await sleep(700)
    if (!prov.has_api_key) fail(400, 'bad_request', '供应商没有设置 API Key', 'The provider has no API key')
    if (body.model === 'qwen3-vl-8b') fail(502, 'upstream_error', '上游返回 503：模型正在加载', 'Upstream returned 503: model is loading')
    return ok({
      ok: true,
      latency_ms: int(420, 1900),
      reply: L(`你好！我是 ${body.model}，可以帮你在远程设备上执行命令、查看文件和排查问题。`, `Hi! I'm ${body.model}. I can run commands, inspect files and troubleshoot on your remote devices.`),
    })
  }
  p = M('DELETE', '/ai/providers/:id')
  if (p) {
    const prov = findProvider(Number(p.id))
    db.providers = db.providers.filter((x) => x.id !== prov.id)
    audit('ai.provider_delete', 'ai_provider', String(prov.id), prov.name, me!.id)
    return ok()
  }

  // AI 用量
  if (M('GET', '/ai/usage')) return ok({ items: usage(q) })

  // 审计
  if (M('GET', '/audit')) {
    const kw = (q.get('q') ?? '').toLowerCase()
    const from = q.get('from') ? Number(q.get('from')) : 0
    const to = q.get('to') ? Number(q.get('to')) : Infinity
    const actor = q.get('actor_type')
    const action = q.get('action')
    const list = db.audit.filter(
      (e) =>
        e.at >= from &&
        e.at <= to &&
        (!actor || e.actor_type === actor) &&
        (!action || e.action === action) &&
        (!kw || `${e.actor_name} ${e.target_name} ${e.ip}`.toLowerCase().includes(kw)),
    )
    return ok(page(list, q))
  }

  // 系统设置
  if (M('GET', '/settings')) return ok(db.settings)
  if (M('PATCH', '/settings')) {
    db.settings = { ...db.settings, ...(body as Partial<Settings>) }
    audit('settings.update', 'settings', '', '', me!.id, { fields: Object.keys(body) })
    return ok(db.settings)
  }

  // 更新包
  const updateList = () => ({ items: db.updates, last_check_at: db.updatesLastCheck, last_error: db.updatesLastError })
  if (M('GET', '/updates')) return ok(updateList())
  if (M('POST', '/updates/check')) {
    await sleep(900)
    db.updatesLastCheck = Date.now()
    db.updatesLastError = ''
    return ok(updateList())
  }
  if (M('POST', '/updates/upload')) {
    const files = req.body as Record<string, File>
    const f = files.file
    const pkg: UpdatePackage = {
      id: nextId(),
      component: 'worker-linux',
      platform: 'linux',
      arch: 'x86_64',
      version: '0.1.0.43',
      file_name: f?.name ?? 'upload.bin',
      size: f?.size ?? 0,
      sha256: hex(64),
      signature_ok: !!files.signature,
      source: 'manual',
      created_at: Date.now(),
    }
    db.updates.unshift(pkg)
    audit('update.upload', 'update', String(pkg.id), pkg.file_name, me!.id)
    return ok(pkg)
  }
  p = M('DELETE', '/updates/:id')
  if (p) {
    db.updates = db.updates.filter((x) => x.id !== Number(p!.id))
    return ok()
  }

  return fail(404, 'not_found', `模拟接口里没有 ${method} ${path}`, `No mock for ${method} ${path}`)
}

export async function handle(req: MockRequest): Promise<MockResponse> {
  lang = req.headers['Accept-Language'] ?? 'zh-CN'
  await sleep(120 + Math.random() * 200)
  try {
    return await route(req)
  } catch (e) {
    if (e instanceof HttpError) return { status: e.status, body: { error: { code: e.code, message: e.message } } }
    console.error('[mock]', e)
    return { status: 500, body: { error: { code: 'internal', message: String(e) } } }
  }
}
