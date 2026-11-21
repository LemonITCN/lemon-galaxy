// 全局提示：由 AppBridge 组件在 <a-app> 内部取到带主题的 message / modal 实例后绑定到这里，
// 请求封装等非组件代码也能用同一套提示。
import type { App } from 'antdv-next'

type AppApi = ReturnType<typeof App.useApp>

let bound: AppApi | null = null

export function bindFeedback(api: AppApi) {
  bound = api
}

export const toast = {
  success(text: string) {
    bound?.message.success(text)
  },
  error(text: string) {
    bound?.message.error(text)
  },
  warning(text: string) {
    bound?.message.warning(text)
  },
  info(text: string) {
    bound?.message.info(text)
  },
}

export interface ConfirmOptions {
  title: string
  content?: string
  okText?: string
  danger?: boolean
}

/** 确认对话框；用户点确定返回 true */
export function confirm(opts: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    if (!bound) {
      resolve(window.confirm(opts.title))
      return
    }
    bound.modal.confirm({
      title: opts.title,
      content: opts.content,
      okText: opts.okText,
      okButtonProps: opts.danger ? { danger: true } : undefined,
      onOk: () => resolve(true),
      onCancel: () => resolve(false),
    })
  })
}
