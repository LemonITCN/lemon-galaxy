// 管理后台接口，一个函数对应 docs/api.md 第一部分的一个接口。
import { del, get, patch, post, put, upload } from './http'
import type {
  AdminUser,
  AiGrant,
  AiModel,
  AiProvider,
  AiProviderCreate,
  AiProviderPatch,
  AiTestResult,
  AuditEntry,
  AuditQuery,
  Connection,
  Device,
  DeviceQuery,
  ItemsOf,
  LoginResult,
  MeResult,
  OkResult,
  Page,
  ServerInfo,
  Settings,
  SetupStatus,
  TimelinePoint,
  TotpSetupResult,
  UpdateList,
  UpdatePackage,
  UsageQuery,
  UsageRow,
  User,
  UserCreate,
  UserPatch,
} from './types'

export * from './types'
export { ApiError, errorText, onAuthError, setCsrfToken } from './http'

// 1. 初始化
export const setupStatus = () => get<SetupStatus>('/setup/status', undefined, { silent: true, noAuthRedirect: true })
export const setup = (token: string, username: string, password: string) =>
  post<OkResult>('/setup', { token, username, password }, { silent: true, noAuthRedirect: true })

// 2. 登录
export const login = (username: string, password: string, totp_code?: string) =>
  post<LoginResult>('/login', { username, password, totp_code }, { silent: true, noAuthRedirect: true })
export const logout = () => post<OkResult>('/logout', {}, { silent: true, noAuthRedirect: true })
export const me = () => get<MeResult>('/me', undefined, { silent: true, noAuthRedirect: true })
export const totpSetup = () => post<TotpSetupResult>('/totp/setup', {}, { silent: true, noAuthRedirect: true })
export const totpVerify = (code: string) => post<OkResult>('/totp/verify', { code }, { silent: true, noAuthRedirect: true })
export const changePassword = (old_password: string, new_password: string) =>
  post<OkResult>('/password', { old_password, new_password }, { silent: true })

// 3. 管理员
export const listAdmins = () => get<ItemsOf<AdminUser>>('/admins')
export const createAdmin = (username: string, password: string) => post<AdminUser>('/admins', { username, password })
export const deleteAdmin = (id: number) => del<OkResult>(`/admins/${id}`)
export const resetAdminTotp = (id: number) => post<OkResult>(`/admins/${id}/reset-totp`)

// 4. 概览
export const serverInfo = () => get<ServerInfo>('/server-info')
export const timeline = (days = 7) => get<ItemsOf<TimelinePoint>>('/stats/timeline', { days })

// 5. 用户
export const listUsers = (q: { q?: string; page?: number; page_size?: number }) => get<Page<User>>('/users', q)
export const createUser = (body: UserCreate) => post<User>('/users', body)
export const getUser = (id: number, silent = false) => get<User>(`/users/${id}`, undefined, { silent })
export const updateUser = (id: number, body: UserPatch) => patch<User>(`/users/${id}`, body)
export const resetUserPassword = (id: number, new_password: string) => post<User>(`/users/${id}/reset-password`, { new_password })
export const resetUserTotp = (id: number) => post<OkResult>(`/users/${id}/reset-totp`)
export const revokeUserSessions = (id: number) => post<OkResult>(`/users/${id}/revoke-sessions`)
export const deleteUser = (id: number) => del<OkResult>(`/users/${id}`)
export const getUserGrants = (id: number) => get<ItemsOf<AiGrant>>(`/users/${id}/ai-grants`)
export const putUserGrants = (id: number, items: AiGrant[]) => put<ItemsOf<AiGrant>>(`/users/${id}/ai-grants`, { items })

// 6. 设备
export const listDevices = (q: DeviceQuery) => get<Page<Device>>('/devices', q as Record<string, unknown>)
export const getDevice = (id: string) => get<Device>(`/devices/${id}`)
export const revokeDevice = (id: string, user_id?: number) =>
  post<Device>(`/devices/${id}/revoke`, user_id === undefined ? {} : { user_id })
export const deleteDevice = (id: string) => del<OkResult>(`/devices/${id}`)

// 7. 当前连接
export const listConnections = (silent = false) => get<ItemsOf<Connection>>('/connections', undefined, { silent })
export const terminateConnection = (id: string) => post<OkResult>(`/connections/${id}/terminate`)

// 8. 大模型供应商
export const listProviders = () => get<ItemsOf<AiProvider>>('/ai/providers')
export const createProvider = (body: AiProviderCreate) => post<AiProvider>('/ai/providers', body)
export const updateProvider = (id: number, body: AiProviderPatch) => patch<AiProvider>(`/ai/providers/${id}`, body)
export const setProviderModels = (id: number, items: Pick<AiModel, 'id' | 'enabled'>[]) =>
  put<AiProvider>(`/ai/providers/${id}/models`, { items })
export const refreshProviderModels = (id: number) => post<AiProvider>(`/ai/providers/${id}/refresh-models`)
export const testProvider = (id: number, model: string) =>
  post<AiTestResult>(`/ai/providers/${id}/test`, { model }, { silent: true })
export const deleteProvider = (id: number) => del<OkResult>(`/ai/providers/${id}`)

// 9. AI 用量
export const aiUsage = (q: UsageQuery) => get<ItemsOf<UsageRow>>('/ai/usage', q as unknown as Record<string, unknown>)

// 10. 审计日志
export const listAudit = (q: AuditQuery) => get<Page<AuditEntry>>('/audit', q as Record<string, unknown>)

// 11. 系统设置
export const getSettings = () => get<Settings>('/settings')
export const updateSettings = (body: Partial<Settings>) => patch<Settings>('/settings', body)

// 12. 更新包
export const listUpdates = () => get<UpdateList>('/updates')
export const checkUpdates = () => post<UpdateList>('/updates/check')
export const uploadUpdate = (file: File, signature: File, onProgress?: (p: number) => void) =>
  upload<UpdatePackage>('/updates/upload', { file, signature }, onProgress)
export const deleteUpdate = (id: number) => del<OkResult>(`/updates/${id}`)
