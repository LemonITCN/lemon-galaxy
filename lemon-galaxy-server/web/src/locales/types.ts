import type { ConfigProviderProps } from 'antdv-next'

/** 一种界面语言：语言包文件导出的就是它 */
export interface LocaleDefinition<M> {
  /** 语言代码，同时作为请求头 Accept-Language 的值，例如 zh-CN、en */
  code: string
  /** 在语言菜单里显示的名字（用该语言自己的写法） */
  name: string
  /** 浏览器语言前缀匹配，例如 ['zh'] 让 zh-TW、zh-HK 也落到这个语言 */
  match: string[]
  /** antdv-next 自身的语言包 */
  antd: ConfigProviderProps['locale']
  /** dayjs 语言名（语言文件里要先 import 对应的 dayjs/locale/xxx） */
  dayjs: string
  messages: M
}

/** 把嵌套的文案对象展开成 'a.b.c' 形式的键 */
export type MessageKeys<T, P extends string = ''> = {
  [K in keyof T & string]: T[K] extends string ? `${P}${K}` : MessageKeys<T[K], `${P}${K}.`>
}[keyof T & string]
