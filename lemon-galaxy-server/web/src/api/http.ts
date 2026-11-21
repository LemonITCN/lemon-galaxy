// 请求封装：同源 /api/v1/admin，Cookie（lg_admin）登录。
// - 非 GET 请求自动带 X-CSRF-Token（登录或 GET /admin/me 时拿到，只存在内存里）
// - 自动带 Accept-Language，服务器据此返回对应语言的错误说明
// - 出错时默认用 message 提示服务器返回的 error.message；401 交给路由跳登录页
import { localeCode, t } from '@/locales'
import { toast } from '@/utils/feedback'

export const API_BASE = '/api/v1/admin'

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message)
  }
}

export interface RequestOptions {
  query?: Record<string, unknown>
  body?: unknown
  /** 不自动弹出错误提示（调用方自己处理） */
  silent?: boolean
  /** 401 时不触发全局跳登录（登录、初始化等接口自己处理） */
  noAuthRedirect?: boolean
}

let csrfToken = ''

export function setCsrfToken(token: string) {
  csrfToken = token || ''
}

type AuthListener = (err: ApiError) => void
const authListeners: AuthListener[] = []

/** 登录失效（401）或服务器要求先初始化（setup_required）时通知路由 */
export function onAuthError(fn: AuthListener) {
  authListeners.push(fn)
}

// ---------- 开发用假数据：只在 dev 且（VITE_MOCK=1 或 --mode mock）时动态加载，生产构建会整段剔除 ----------

export interface MockRequest {
  method: string
  path: string
  query: URLSearchParams
  body: unknown
  headers: Record<string, string>
}
export interface MockResponse {
  status: number
  body: unknown
}
type MockHandler = (req: MockRequest) => Promise<MockResponse>

let mockLoader: Promise<MockHandler> | null = null
function mockHandler(): Promise<MockHandler> | null {
  if (import.meta.env.DEV && (import.meta.env.VITE_MOCK === '1' || import.meta.env.MODE === 'mock')) {
    mockLoader ??= import('@/mock').then((m) => m.handle)
    return mockLoader
  }
  return null
}

// ---------- 核心 ----------

function buildQuery(query?: Record<string, unknown>): string {
  if (!query) return ''
  const q = new URLSearchParams()
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined || v === null || v === '') continue
    q.set(k, String(v))
  }
  const s = q.toString()
  return s ? '?' + s : ''
}

function headers(method: string, json: boolean): Record<string, string> {
  const h: Record<string, string> = {
    Accept: 'application/json',
    'Accept-Language': localeCode.value,
  }
  if (method !== 'GET' && method !== 'HEAD') h['X-CSRF-Token'] = csrfToken
  if (json) h['Content-Type'] = 'application/json'
  return h
}

function toError(status: number, body: any): ApiError {
  const e = body?.error ?? {}
  return new ApiError(status, e.code ?? 'http_' + status, e.message || t('errors.http', { status }))
}

function handleError(err: ApiError, opts: RequestOptions) {
  const authFailure = err.status === 401 || err.code === 'setup_required'
  if (authFailure && !opts.noAuthRedirect) {
    authListeners.forEach((l) => l(err))
    return
  }
  if (!opts.silent) toast.error(err.message)
}

export async function request<T>(method: string, path: string, opts: RequestOptions = {}): Promise<T> {
  const url = API_BASE + path + buildQuery(opts.query)
  const hasBody = opts.body !== undefined

  const mock = mockHandler()
  if (mock) {
    const handle = await mock
    const res = await handle({
      method,
      path,
      query: new URLSearchParams(buildQuery(opts.query).slice(1)),
      body: hasBody ? JSON.parse(JSON.stringify(opts.body)) : undefined,
      headers: headers(method, hasBody),
    })
    if (res.status >= 200 && res.status < 300) return res.body as T
    const err = toError(res.status, res.body)
    handleError(err, opts)
    throw err
  }

  let res: Response
  try {
    res = await fetch(url, {
      method,
      credentials: 'same-origin',
      headers: headers(method, hasBody),
      body: hasBody ? JSON.stringify(opts.body) : undefined,
    })
  } catch {
    const err = new ApiError(0, 'network', t('errors.network'))
    if (!opts.silent) toast.error(err.message)
    throw err
  }

  let body: any = null
  const text = await res.text()
  if (text) {
    try {
      body = JSON.parse(text)
    } catch {
      body = null
    }
  }
  if (!res.ok) {
    const err = toError(res.status, body)
    handleError(err, opts)
    throw err
  }
  return body as T
}

export const get = <T>(path: string, query?: Record<string, unknown>, opts: RequestOptions = {}) =>
  request<T>('GET', path, { ...opts, query })
export const post = <T>(path: string, body: unknown = {}, opts: RequestOptions = {}) => request<T>('POST', path, { ...opts, body })
export const patch = <T>(path: string, body: unknown, opts: RequestOptions = {}) => request<T>('PATCH', path, { ...opts, body })
export const put = <T>(path: string, body: unknown, opts: RequestOptions = {}) => request<T>('PUT', path, { ...opts, body })
export const del = <T>(path: string, opts: RequestOptions = {}) => request<T>('DELETE', path, opts)

/** multipart 上传，带进度；返回可取消的句柄 */
export function upload<T>(
  path: string,
  fields: Record<string, File>,
  onProgress?: (percent: number) => void,
): { promise: Promise<T>; abort: () => void } {
  const mock = mockHandler()
  if (mock) {
    let aborted = false
    const promise = (async () => {
      const handle = await mock
      for (let p = 10; p <= 100; p += 30) {
        if (aborted) throw new ApiError(0, 'aborted', t('errors.uploadAborted'))
        onProgress?.(Math.min(p, 100))
        await new Promise((r) => setTimeout(r, 120))
      }
      const res = await handle({ method: 'POST', path, query: new URLSearchParams(), body: fields, headers: headers('POST', false) })
      if (res.status >= 200 && res.status < 300) return res.body as T
      const err = toError(res.status, res.body)
      handleError(err, {})
      throw err
    })()
    return { promise, abort: () => (aborted = true) }
  }

  const xhr = new XMLHttpRequest()
  const promise = new Promise<T>((resolve, reject) => {
    xhr.open('POST', API_BASE + path)
    xhr.withCredentials = true
    for (const [k, v] of Object.entries(headers('POST', false))) xhr.setRequestHeader(k, v)
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && onProgress) onProgress(Math.round((e.loaded / e.total) * 100))
    }
    xhr.onload = () => {
      let body: any = null
      try {
        body = JSON.parse(xhr.responseText)
      } catch {
        body = null
      }
      if (xhr.status >= 200 && xhr.status < 300) {
        resolve(body as T)
        return
      }
      const err = toError(xhr.status, body)
      handleError(err, {})
      reject(err)
    }
    xhr.onerror = () => {
      const err = new ApiError(0, 'network', t('errors.uploadFailed'))
      toast.error(err.message)
      reject(err)
    }
    xhr.onabort = () => reject(new ApiError(0, 'aborted', t('errors.uploadAborted')))
    const fd = new FormData()
    for (const [k, v] of Object.entries(fields)) fd.append(k, v, v.name)
    xhr.send(fd)
  })
  return { promise, abort: () => xhr.abort() }
}

export function errorText(e: unknown): string {
  if (e instanceof ApiError) return e.message
  if (e instanceof Error) return e.message
  return String(e)
}
