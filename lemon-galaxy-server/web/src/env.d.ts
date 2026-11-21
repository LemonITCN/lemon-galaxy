/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** 仅开发用：为 '1' 时请求走 src/mock 内存假数据（npm run dev:mock 用 --mode mock 达到同样效果） */
  readonly VITE_MOCK?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
