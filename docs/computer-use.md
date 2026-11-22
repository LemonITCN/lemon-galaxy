# AI 应用操作（computer use）

让 AI 像 Codex 的 computer use 一样操作电脑上的某个应用：**不抢用户的鼠标和键盘、不把目标应用拉到前台**，
用户可以同时继续用电脑；AI 有自己专属的光标，画在被操作的窗口上，用户看得见它在点哪里。

它和已有的全屏 `screenshot / mouse / keyboard` 工具并存：后者会动真实的鼠标键盘（接管前台），只在应用级操作做不到时才用。

## 1. 分层

```
智能体（galaxy-agent）      app_* 工具、按应用授权、风险判定
  │  Executor::computer_use(target, 请求 JSON, origin)
  ├─ 本机目标（galaxy-ffi LocalAi）──→ HostPlatform::computer_use（进程内）
  └─ 远程设备（PeerSession）── MgmtReq::ComputerUse ──→ 被控端 serve ──→ HostPlatform::computer_use
                                                         （Windows 被控端经会话代理 IPC 到用户会话里执行）
平台实现：macOS = Swift（MacHostPlatform，主控端与被控端共用）；Windows = Rust（galaxy-host::win_cu，
         被控端会话代理与 Windows 主控端进程内共用）
```

- 支持的被控端在能力列表里报 `"computer_use"`；不支持的（老版本、Linux、手机）不给模型这组工具。
- 协议：`MgmtReq::ComputerUse { request: String, origin: Origin }`，应答 `MgmtResp::ComputerUse { text: String, image: Option<ImageData> }`
  （两个变体都加在各自枚举的最后）。被控端要求会话有控制权限，并按 AI 上限判定（读类动作按只读算，其余按普通操作）。
- 平台接口：`fn computer_use(&self, request: String) -> Option<ComputerUseReply>`，`ComputerUseReply { text, image }`；
  返回 None 表示平台不支持。调用在后台线程上，可以阻塞（单次最多约 10 秒）。

## 2. 请求与应答（JSON）

请求都带 `action`；除 `list_apps` 外都带 `app`（应用名、Bundle ID / 可执行文件名，或 `pid:1234`，大小写不敏感，
先精确再前缀匹配）。可选 `window`（第几个窗口，默认最前面的那个）。

| action | 参数 | 说明 |
|---|---|---|
| `list_apps` | 无 | 运行中的有窗口应用：名称、Bundle ID / 进程名、pid、窗口数、是否前台 |
| `get_app_state` | `app`, `window?`, `max_nodes?`（默认 400） | 窗口标题与位置 + 编号的控件树 + 窗口截图（见下） |
| `click` | `app`, `element` 或 `x`,`y`；`count?`(1/2)；`button?`(left/right) | 优先按控件执行（AXPress / InvokePattern）；给坐标时按最近一次截图的像素 |
| `set_value` | `app`, `element`, `value` | 直接设置输入框等控件的值 |
| `type_text` | `app`, `text`, `element?` | 给了 element 先让它获得焦点；输入到应用当前焦点控件 |
| `press_key` | `app`, `key` | 如 `Return`、`Tab`、`Escape`、`cmd+s`、`ctrl+shift+t`；带 cmd/ctrl 的快捷键在 macOS 上优先找菜单项执行 |
| `scroll` | `app`, `element?` 或 `x`,`y`；`direction`(up/down/left/right)；`pages?`(默认 1，可小数) | |
| `perform_action` | `app`, `element`, `name` | 执行控件的其他动作（如 AXShowMenu、AXIncrement、ExpandCollapse） |
| `open_app` | `app` | 启动应用但不抢前台（已在运行就什么也不做） |
| `end` | 无 | 本轮结束：收起 AI 光标 |

**控件树格式**（`get_app_state` 的 text，模型直接读）：

```
应用 Safari（com.apple.Safari，pid 812），窗口 0/2「Apple」 位置 (120,80) 大小 1280×800，截图 1280×800
[0] AXWindow "Apple"
  [1] AXToolbar
    [2] AXButton "后退" actions=AXPress
    [3] AXTextField "地址和搜索" value="apple.com" settable focused
  [4] AXWebArea "Apple"
    [5] AXLink "Mac" actions=AXPress (412,96 48×20)
```

- 每行：编号、角色、标题或说明（引号内）、`value=`（截断到 200 字）、`settable`、`focused`、`disabled`、
  `actions=`（除 AXPress 外的动作也列出）、括号里是控件在**截图**里的位置与大小。
- 编号在下一次 `get_app_state` 之前有效（平台缓存控件句柄）；对同一应用重新 `get_app_state` 会重编号。
- 只列可见、有意义的控件（跳过无标题的纯布局容器，但保留它的子节点缩进），最多 `max_nodes` 个。
- 截图：只截目标窗口（被遮挡也能截），长边不超过 1280，JPEG 或 PNG；坐标系 = 这张截图的像素。

**动作的应答**：一行结果（如「已点击 [5] AXLink "Mac"」），并附动作后约 300 ms 的窗口截图（模型能看图时用得上）。
出错时 text 以 `错误：` 开头，说明原因和建议（如「控件编号已失效，请先 get_app_state」）。

## 3. 不抢前台的做法

### macOS
1. 观察：AX 树（`AXUIElementCreateApplication(pid)`）；窗口截图 `SCScreenshotManager` + `SCContentFilter(desktopIndependentWindow:)`。
   Electron 应用先设 `AXManualAccessibility = true`；Chrome 的网页树懒加载，首次取不到时等一下再取。
2. 点击：有控件编号时 `AXUIElementPerformAction(kAXPressAction)`；只有坐标时用进程定向事件：
   `NSEvent.mouseEvent(windowNumber:)` → `cgEvent`，设字段 91/92 为目标窗口号、`CGEventSetWindowLocation`（私有，dlsym 探测），
   `CGEvent.postToPid(pid)`。Chromium 系需要 `SLEventPostToPid`（SkyLight 私有，dlsym 探测；没有就退回 postToPid）。
3. 文字：有焦点控件且 settable 时优先改 `AXSelectedText`（插入到光标处）；否则 `CGEvent(keyboardEventSource:)` +
   `keyboardSetUnicodeString` 再 `postToPid`。按键同样 `postToPid`；带 cmd 的快捷键先在 AX 菜单栏里找对应菜单项 `AXPress`。
4. 滚动：控件支持就用 AXScrollArea 的滚动条值；否则 `CGEvent(scrollWheelEvent2Source:)` + `postToPid`。
5. 私有接口一律 `dlopen/dlsym` 运行时探测，缺了就退到公开接口，不崩溃。
6. 做不到后台的（游戏、只认 HID 事件的画布类应用）：返回错误说明，提示模型改用全屏 `mouse / keyboard`（会接管前台，需确认）。
7. 实现备注（`lemon-galaxy-control-macos/Sources/LemonGalaxy/Platform/ComputerUse.swift`，被控端经符号链接共用）：
   - Chromium / Electron 系（按 Bundle ID 与包内 Frameworks 判断）在坐标点击、键盘输入前，先用 `SLPSPostEventRecordTo` 给**目标进程**发一条
     只对它生效的「激活」记录（键盘另加 yabai 的 make-key-window 两条记录），动作后撤销；不给真实前台应用发失活记录。
     鼠标事件同时经 `SLEventPostToPid` 与公开 `postToPid` 投递。缺任一私有符号就只走公开 `postToPid`。
   - `type_text` 用 `AXSelectedText` 插入后会读回值确认真的变了，没变再退回 Unicode 键盘事件；含换行的文字直接走键盘事件（换行按 Return）。
   - 没有「屏幕录制」授权时照样返回控件树，坐标系按「长边不超过 1280、不放大」虚拟一张截图换算。
   - 无人值守被控端：后台服务（root）经会话代理 IPC（`p.computerUse`）转发，由用户图形会话里的代理执行；登录界面不支持。
     代理平时不建 NSApplication，执行会动 AI 光标的动作时才接上 AppKit（与隐私屏同一机制，空闲时回收）。
   - AI 光标目前不跟随目标窗口移动，只在每次动作时移到新位置，`end` 或 30 秒无动作淡出。
   - 只读自检：`LemonGalaxy --cu-selftest [应用名] [--out 截图路径]`（list_apps + get_app_state + 自我保护判定，不点击不输入）。

### Windows
1. 观察：UI Automation（`IUIAutomation::ElementFromHandle` + `RawViewWalker`/`ControlViewWalker`）；
   窗口截图 `PrintWindow(hwnd, PW_RENDERFULLCONTENT)`（失败再 BitBlt 窗口区域）。
2. 点击：InvokePattern / SelectionItemPattern / TogglePattern / ExpandCollapsePattern；只有坐标时 `PostMessage(WM_LBUTTONDOWN/UP)`
   到坐标下的子窗口（客户区坐标）。
3. 文字：ValuePattern.SetValue（会抢前台的控件改用 `EM_REPLACESEL` / `WM_CHAR`）。按键 `PostMessage(WM_KEYDOWN/UP)`。
4. 滚动：ScrollPattern，否则 `WM_MOUSEWHEEL`（屏幕坐标）。
5. 目标进程权限更高（管理员运行）时窗口消息会被系统丢弃：返回错误说明。

## 4. AI 光标（专属光标）

- 一个透明、不接收鼠标、不抢焦点的小窗口，画一个与系统光标明显不同的箭头（品牌强调色描边 + 白色内填充，旁边小标签「AI」），
  位于目标窗口之上（macOS：`order(.above, relativeTo: 目标窗口号)`；Windows：置顶的分层窗口 `WS_EX_LAYERED|WS_EX_TRANSPARENT|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW`）。
- 每次动作前，光标沿平滑曲线移到动作位置（约 250–400 ms），点击时有一个小的按下动画；输入文字时停在焦点控件处。
- 目标窗口移动或被关闭时跟着走或隐藏；`end` 或 30 秒没有动作后淡出。
- 被控端远程使用时，这个光标画在被控电脑上（被控电脑前的人也能看到 AI 在操作哪里），截屏画面里也看得到。

## 5. 智能体侧（galaxy-agent）

- 工具（都带 `target`、`reason`）：`app_list`、`app_state`、`app_click`、`app_set_value`、`app_type`、`app_key`、
  `app_scroll`、`app_action`、`app_open`。参数与第 2 节一一对应。只有目标支持 `computer_use` 时才给模型。
- **按应用授权**（与 Codex 一致）：每个对话里第一次操作某个应用（含读取它的界面）先弹确认「AI 想操作 某应用」，只有允许 / 拒绝；
  允许后本对话内这个应用不再问。完全信任档不问。
- 绝对禁止操作的应用：LemonGalaxy 自身、密码管理器（钥匙串访问、1Password、Bitwarden、LastPass 等）、系统的安全与隐私设置。
- 风险：`app_list` / `app_state` 只读；`app_click` 按控件文字判定（删除、发送、付款、确认等是危险操作）；其余为普通操作。
- 一轮任务结束时对用过 app 工具的目标发一次 `end`。
- 系统提示词：操作某个应用时优先用 app_* 工具（不打扰用户）；先 `app_state` 看清楚再动手，每次动作后看返回的截图确认效果；
  做不到时再说明并改用全屏工具。
