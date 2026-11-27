# macOS 纯被控端无人值守（登录界面控制）调研与测试计划

- 日期：2026-11-22
- 范围：lemon-galaxy-worker-macos（LemonGalaxyWorker）开机即在线、没人登录时可远程控制登录界面、注销与快速切换用户不掉线。契约见 [spec.md 3.0.1](../spec.md)。
- 方法：Apple 开发者论坛（含 DTS 回复）与官方文档；本机只做不登记任何系统服务、不弹窗的实验（LaunchServices 登记、代码签名核对、跨进程通信、包签名检查）。**没有在本机登记后台服务或会话代理，没有用 sudo。**
- 标记约定：【已核实】= 一手来源或本机实验直接看到；【推测】= 按机制推断；【待验证】= 必须上真机测。

---

## 0. 结论速览

1. 结构与 Windows 一致：root 的后台服务（LaunchDaemon）联网、跑内核；每个图形会话（登录界面 + 每个已登录用户）一个会话代理（LaunchAgent）负责屏幕与键鼠；后台服务把平台请求转给当前在屏幕上的那个代理。Apple DTS 推荐的就是这个结构（守护进程不该碰窗口服务器）。【已核实】
2. 后台服务用 `SMAppService.daemon` 登记（macOS 13 起，配置放包内 `Contents/Library/LaunchDaemons`，用户在登录项里批准）。会话代理**不能**用 `SMAppService.agent`：它只登记到当前用户，而且 LoginWindow 类型登记时报 “Service cannot load in requested session”，DTS 说这个 API 不是为此设计的。所以由后台服务（root）把全局配置写到 `/Library/LaunchAgents`，与 TeamViewer、AnyDesk、RustDesk 的做法相同。【已核实（论坛）；第三方做法为推测】
3. 登录界面截屏：macOS 14.4 修好了 ScreenCaptureKit 在登录界面前不能用的问题（DTS，r. 121253782）；更早的系统用 CGDisplayStream。实现为「14.4 起先试 ScreenCaptureKit，不行退 CGDisplayStream」。【已核实（论坛）；在本工程上待验证】
4. 同一个 Bundle ID 的辅助进程只要碰了 `NSApplication.shared`（`NSScreen.screens` 也会隐式创建）就会登记到 LaunchServices，之后双击应用、登录项启动都只会激活那个辅助进程、不再打开界面。所以会话代理平时不建 NSApplication，只在开隐私屏时建，空闲后退出由 launchd 重启。【已核实（本机实验，见 2.5）】
5. 系统授权（屏幕录制、辅助功能）按代码身份记在系统级 TCC 库；三种角色是同一个签名的可执行文件，授权共用。【推测，待验证】macOS 15 起用户会话里的屏幕录制会定期要求「继续允许」，必须有人点；VNC 类软件可申请 `com.apple.developer.persistent-content-capture` 托管授权。【已核实（文档、新闻）；对本工程的影响待验证】
6. 需要内核改一处安全问题：临时协助会话以宿主进程身份执行终端与文件，后台服务里就是 root（Windows 服务里是 SYSTEM）。现阶段在后台服务里限制为只有管理员能开启或允许协助。见第 4 节。

---

## 1. 进程结构

```
                    ┌────────────── 系统域（launchd，root）──────────────┐
                    │  LemonGalaxyWorker --daemon                         │
                    │   WorkerHost + HostService（Rust 内核，联网、账号）  │
                    │   DaemonPlatform：HostPlatform → 当前屏幕上的代理    │
                    │   /var/run/cn.lemonit.galaxy.worker.sock（0666）     │
                    └───────▲───────────────▲───────────────▲─────────────┘
                            │ 平台请求 / 画面帧 │               │ 界面请求 / 状态推送
        ┌───────────────────┴──┐   ┌─────────┴────────────┐  ┌─┴──────────────────────┐
        │ 登录界面会话（root）  │   │ 用户 A 的图形会话      │  │ 菜单栏应用（用户 A）     │
        │ --session-agent      │   │ --session-agent       │  │ RemoteWorkerHost        │
        │ SCK / CGDisplayStream│   │ MacHostPlatform（SCK）│  │ 同样的页面，不跑内核     │
        └──────────────────────┘   └───────────────────────┘  └────────────────────────┘
```

- 后台服务：包内 `Contents/Library/LaunchDaemons/cn.lemonit.galaxy.worker.daemon.plist`（`BundleProgram` = `Contents/MacOS/LemonGalaxyWorker`，参数 `--daemon`，`KeepAlive`，`AssociatedBundleIdentifiers`），`SMAppService.daemon(plistName:)` 登记。
- 会话代理：后台服务启动时生成 `/Library/LaunchAgents/cn.lemonit.galaxy.worker.agent.plist`（`ProgramArguments` = 包内主程序绝对路径 + `--session-agent`，`LimitLoadToSessionType` = LoginWindow + Aqua，`KeepAlive` = `{SuccessfulExit: false}`），内容变了才改写，并在已登录用户的 `gui/<uid>` 域里重新加载；包内 `Contents/Library/LaunchAgents/` 里放同样内容的模板。
- 菜单栏应用：后台服务已登记且启用时，启动即改当界面（`RemoteWorkerHost` / `RemoteHostService` 是 `WorkerHost` / `HostService` 的子类，界面代码不变）。

## 2. 关键问题与依据

### 2.1 SMAppService 能做什么

| 需求 | SMAppService | 结论 |
|---|---|---|
| root 守护进程，开机启动 | `SMAppService.daemon`，配置放包内，用户在「登录项与扩展 → 允许在后台」批准（要管理员密码） | 用它【已核实（文档）】 |
| 登录界面里的代理 | `SMAppService.agent` 登记 LoginWindow 类型报 “Service cannot load in requested session”，状态却显示已启用；DTS：「我怀疑 SMAppService API 不是为此设计的」 | 不能用【已核实（论坛 726864）】 |
| 每个用户的代理 | `SMAppService.agent` / `.loginItem` 只绑定当前用户，不会扩散到之后登录的其他用户；系统级 agent 目前做不到（r. 92457638） | 不能用【已核实（论坛 737038）】 |
| 推荐做法 | DTS：守护进程（KeepAlive）+ 代理（KeepAlive，`LimitLoadToSessionType` = Aqua + LoginWindow），IPC 连接 | 照做【已核实（论坛 814152）】 |

本机看到：构建目录里那份 .app（`.noindex`，已从 LaunchServices 注销）调用 `SMAppService.daemon(...).status` 得到 `.notFound`。登记要在 `/Applications` 里的正式签名应用上做。【已核实（本机）；登记流程待验证】

### 2.2 截屏

- 守护进程「不该连窗口服务器」，ScreenCaptureKit 也不例外；在守护进程里截登录界面，14、15 上都有人报拿到空帧。必须放在 pre-login agent 里。【已核实（论坛 814152、768146）】
- DTS 用 ScreenCaptureKit 做过原型，在 macOS 14.4 及以后可以截登录界面；14.4 修了阻止 pre-login 使用 ScreenCaptureKit 的问题（r. 121253782），之前要用 CGDisplayStream 这类老接口。【已核实（论坛 814152）】
- macOS 15.0 / 15.1 beta：登录界面里以 root 运行、由服务用 NSTask 拉起的进程调 `CGDisplayStreamCreateWithDispatchQueue` 卡 30 秒后返回 NULL，改用 NSWorkspace 启动后正常。本工程的代理由 launchd 直接启动，不经 NSTask。【已核实（论坛 765055）；对本工程待验证】
- 有人报 Sequoia 登录界面前 CGDisplayStream、AVCaptureScreenInput、ScreenCaptureKit 全部失败，但他是在守护进程里截；DTS 要他改成 pre-login agent，之后的结果帖子里没有。【已核实（论坛 768146）】
- `CGDisplayCreateImage`、`CGWindowListCreateImage` 在新 SDK 里标为过时（部署目标 15 起不可用）；本工程部署目标 13，仍能编译（本仓库已编过）。【已核实】
- 实现：用户会话里与主控端相同（ScreenCaptureKit，授权在但启动失败时退 CGDisplayStream）；登录界面里 14.4 起先 ScreenCaptureKit、10 秒内起不来退 CGDisplayStream，更早直接 CGDisplayStream；截图退 `CGDisplayCreateImage`。环境变量 `LEMONGALAXY_CAPTURE=displaystream` 强制走 CGDisplayStream 便于对比。

### 2.3 登录界面的键鼠

- 代理用 `CGEvent.post(tap: .cghidEventTap)`，事件源 `hidSystemState`（与主控端相同）。登录界面的代理以 root 运行，root 不受「辅助功能」授权限制。【推测】
- 有人报 Intel Mac 上注销后登录界面键盘事件发不出去（鼠标可以），约十次成功一次，Apple 芯片正常，帖子没有答案。【已核实（论坛 724740）；需要在 Intel 上专门测】
- 密码框处于安全输入（Secure Event Input）状态，挡的是读取键盘的事件监听，不是 HID 层注入；两种输入（按键与输入法文字）都要在登录界面密码框里测。【推测，待验证】

### 2.4 系统授权（TCC）与签名

- 屏幕录制（kTCCServiceScreenCapture）、辅助功能（kTCCServiceAccessibility）记在系统级 TCC 库，按客户端代码身份（Bundle ID + 指定要求）。后台服务、会话代理、菜单栏应用是同一个可执行文件换参数运行，身份相同，用户在菜单栏应用里授权后代理直接可用；其他用户快速切换进来时也不用重新授权。【推测，待验证】
- 签名必须稳定：临时签名每次编译 cdhash 变，授权全部失效（scripts/build.sh 里已有说明）；SMAppService 登记守护进程也需要正式签名。【后半句推测】
- 本机进程间核对：对端审计令牌 → `SecCodeCopyGuestWithAttributes` → `SecCodeCheckValidity(对端, 本进程的指定要求)`。用开发证书签的包，指定要求是「标识 cn.lemonit.galaxy.worker + Apple 证书链 + 叶证书名」；用系统自带的 nc 冒充会话代理会被拒绝。【已核实（本机自检）】
- macOS 15 起，用 ScreenCaptureKit 的应用会定期（15.0 每月，15.1 起对常用应用减少）弹「继续允许录屏」。无人值守时这个提示出现在用户会话里，没人点就截不到。Apple 给 VNC 类软件准备了托管授权 `com.apple.developer.persistent-content-capture`（要提交申请），有开发者认为它能免掉这个提示，Apple 没有明说。登录界面里会不会弹没有资料。【前半已核实（新闻、论坛 761641）；对本工程待验证】建议：正式发布前申请这个授权。

### 2.5 LaunchServices 陷阱（本机实验）

用一个最小的 `.app`（Bundle ID `cn.lemonit.test.lsprobe`）直接运行包内可执行文件，再用 `lsappinfo find bundleid=…` 看是否登记：

| 进程里做了什么 | 是否登记到 LaunchServices |
|---|---|
| 只用 CoreGraphics（`CGSessionCopyCurrentDictionary`、`CGMainDisplayID`） | 否 |
| `NSPasteboard`、`NSWorkspace.runningApplications`、`NSWorkspace` 通知、`NSBitmapImageRep`、`CGEventSource`、`AXIsProcessTrusted` | 否 |
| `NSScreen.screens` | **是** |
| `NSApplication.shared`（无论 `.accessory` / `.prohibited`、是否 `run()`） | **是** |

登记后再 `open Probe.app`：不新开进程，只激活已登记的那个；`open -n` 才新开。【已核实（本机）】

所以：会话代理用 `MacHostPlatform(appKit: false)`（共享代码新增的参数，不碰 NSScreen，屏幕名称用 CoreGraphics 的通用名称），平时主线程只跑 RunLoop；开隐私屏时才在主线程建 NSApplication 并 `attachAppKit()`，隐私屏关掉且没有采集、声音时退出（非零退出码），launchd 立刻拉起一个干净的。用打好的包实测：代理运行时 `lsappinfo` 查不到本应用。菜单栏应用切换运行方式时用 `open -n` 重新打开自己；「只跑一份」检查跳过参数里带 `--session-agent` / `--daemon` 的进程。

### 2.6 文件保险箱

开了文件保险箱的 Mac 重启后先进预启动认证界面（在 macOS 启动之前），后台服务还没运行，远程不可达。计划内重启用 `sudo fdesetup authrestart` 跳过一次。【已核实（fdesetup 手册）】

### 2.7 终端与文件

后台服务以 root 运行，内核按账号对应的系统用户 `setgroups/setgid/setuid` 后启动终端与命令（与 Linux 被控端相同）。这些进程在系统域里，不在用户的图形会话里：效果像 SSH 会话，`open`、`pbcopy`、用户钥匙串一般用不了；访问「文稿」「桌面」等受保护文件夹时没法弹授权框，需要在「完全磁盘访问权限」里打开 LemonGalaxyWorker。【推测，待验证】

### 2.8 macOS 13–27 差异

| 版本 | 差异 | 来源 |
|---|---|---|
| 13 | SMAppService 首次提供；登录项里出现「允许在后台」列表，新加入的后台项目会发系统通知 | 文档与公开资料【推测，未逐条核对】 |
| 14.0–14.3 | 登录界面里 ScreenCaptureKit 不可用（DTS 说的 bug），只能 CGDisplayStream；`CGDisplayStream` 标为过时 | 论坛 814152【已核实】 |
| 14.4 起 | 登录界面里 ScreenCaptureKit 可用 | 论坛 814152【已核实】 |
| 15.0 / 15.1 | 屏幕录制定期确认；由 NSTask 拉起的登录界面进程 CGDisplayStream 卡 30 秒 | 新闻、论坛 765055【已核实】 |
| 15.x | 有人报 pre-login 截屏全部失败（守护进程里截） | 论坛 768146【已核实】 |
| 26、27 | 没找到与登录界面截屏、pre-login agent 相关的变化资料 | 【待验证】 |

## 3. 实现

文件（均在 `lemon-galaxy-worker-macos/`）：

| 文件 | 作用 |
|---|---|
| `Sources/LemonGalaxyWorker/Unattended/IPC.swift` | 套接字、分帧、请求 / 应答 / 通知、对端凭据与代码签名核对 |
| `Unattended/IPCCodable.swift` | 内核类型的 JSON 编码、消息正文、方法名 |
| `Unattended/UnattendedSupport.swift` | 名字与路径、控制台用户、文件型安全存储、数据移交、会话代理 launchd 配置的生成与加载 |
| `Unattended/DaemonService.swift` | 后台服务入口（`--daemon`）、界面请求处理与权限、移交、`--unattended-status` |
| `Unattended/DaemonPlatform.swift` | 后台服务的 HostPlatform：转发给负责画面的代理、换会话续上 |
| `Unattended/SessionAgent.swift` | 会话代理入口（`--session-agent`）、代理的平台层（登录界面截屏后备）、CGDisplayStream 截屏 |
| `Unattended/RemoteWorker.swift` | 菜单栏应用当界面时的 WorkerHost / HostService 子类 |
| `Unattended/UnattendedController.swift` | SMAppService 登记 / 注销、状态、移交、切换运行方式 |
| `Unattended/IPCSelfTest.swift` | `--ipc-selftest`：不用 root 的通信与配置自检 |
| `UI/UnattendedCard.swift` | 通用页卡片与关闭确认 |
| `Packaging/LaunchDaemons/…daemon.plist`、`Packaging/LaunchAgents/…agent.plist` | 打包进 `Contents/Library/` |
| `scripts/check-bundle.sh` | 包检查（build.sh 打包后自动运行） |

共享代码只改了一处：`lemon-galaxy-control-macos/…/Platform/MacHostPlatform.swift` 增加 `init(appKit: Bool = true)` 与 `attachAppKit()`，默认行为不变。

权限模型：管理员（root 或 admin 组）全部可做；坐在屏幕前的普通用户可查看、开关远程、断开会话、管理对应到自己的账号；其他本机用户被拒绝。签名对不上但对端是管理员时只允许查询与重启（应用刚更新、后台服务还是旧程序）。导入、导出、删除数据只有管理员能做。

## 4. 需要内核改动

1. **临时协助以宿主身份执行（安全）**：`galaxy-host/src/service.rs` 的 `handle_conn` 里协助模式用 `HostPolicy::default_for(&SysUser::current().name)`，后台服务里就是 root，坐在屏幕前的普通用户点「允许」就给了对方 root 终端与文件（Windows 服务里同样是 SYSTEM）。建议：`HostPlatform` 增加 `fn interactive_user(&self) -> Option<String>`（默认 None；macOS 后台服务返回当前在屏幕上的会话代理的用户，Windows 返回控制台会话用户），协助模式用它，取不到时拒绝终端与文件只留画面。FFI 的 `HostPlatform` 相应加 `interactiveUser() -> String?`。现阶段后台服务里只允许管理员开启或允许协助。
2. **终端进入用户的图形会话（可选）**：用户已登录时，终端与命令改用 `launchctl asuser <uid>` 或加入该用户的审计会话后启动，`open`、`pbcopy`、钥匙串才能用。
3. **停止被控服务（可选）**：FFI 的 `WorkerHost` 增加 `shutdown()`，菜单栏应用切换运行方式时可以不重启自己。

## 5. 真机测试计划

准备：

- 用固定证书签名构建：`cd lemon-galaxy-worker-macos && scripts/build.sh --dmg`（Developer ID 或带 LemonIT 的 Apple Development；临时签名不行）。把 `LemonGalaxyWorker.app` 放进 `/Applications`。
- 另一台设备装主控端（Mac 主控或安卓），与本机登录同一服务器上有权限的账号。
- 准备一个非管理员测试用户（下称 test）；控制中心里打开快速用户切换。
- 看日志：`log stream --info --predicate 'eventMessage CONTAINS "LemonGalaxy"'`；后台服务的输出在 `/Library/Logs/LemonGalaxyWorker-daemon.log`，内核日志在 `/Library/Application Support/LemonGalaxy/logs/core.log`（要 sudo）。
- 随时查状态：`/Applications/LemonGalaxyWorker.app/Contents/MacOS/LemonGalaxyWorker --unattended-status`。

A. 开启

1. 用管理员登录，打开应用，登录账号，在「系统授权」里给屏幕录制、辅助功能；先确认普通模式下主控端能看能控（基线），记下设备号。
2. 通用 → 远程访问 → 打开「无人值守」。预期：系统通知「已添加后台项目」，系统设置打开登录项页，卡片显示「等待批准」。
3. 在「允许在后台」里打开 LemonGalaxyWorker（输管理员密码）。预期：几秒内应用自动重新打开，卡片「正在移交」→「运行中」，「当前画面」为「用户 你的名字」；状态页标题旁有「无人值守」；设备号不变，主控端设备列表里还是同一台（没有多出一台）。
4. `--unattended-status`：登记状态已启用，全局会话代理配置存在，你的会话代理在屏幕上并负责画面；`sudo ls -la "/Library/Application Support/LemonGalaxy/worker"` 为 root、0700；`~/Library/Application Support/LemonGalaxyWorker/` 下出现 `worker.migrated`。
5. 从主控端连上：画面、键鼠、剪贴板（文字、图片、文件）、声音、隐私屏、终端（`whoami` 是你的用户名，不是 root）、文件、AI。
6. `lsappinfo find bundleid=cn.lemonit.galaxy.worker` 只有菜单栏应用一个。退出菜单栏应用，从访达双击能重新打开界面；开过隐私屏、会话结束后再试一次。

B. 重启后控制登录界面

7. 重启（开了文件保险箱：`sudo fdesetup authrestart`，或先在开机界面解锁磁盘），不要登录。
8. 主控端：设备在线；连上看到登录窗口；移动鼠标、点用户、在密码框里输入密码（按键与输入法文字两种）并回车。Intel Mac 上专门测键盘。
9. 保持远程画面不断：登录后画面从登录界面切到桌面（可能黑 1–3 秒），不用重连。
10. 日志里有「画面改由 aqua（你的名字）的会话代理负责」；记下登录界面用的是 ScreenCaptureKit 还是 CGDisplayStream，以及 macOS 版本。

C. 注销

11. 远程操作「苹果菜单 → 退出登录」：画面回到登录界面，可以继续操作；设备一直在线。

D. 快速切换用户

12. 你登录着，切到 test：中间的登录窗口可远程输入 test 的密码；登录后画面是 test 的桌面。
13. 在 test 里运行 `--unattended-status`：两个用户代理，test 在屏幕上并负责画面。
14. 在 test 里打开应用：界面可用；开临时协助提示需要管理员；不能修改对应到你的账号；可以断开会话、开关远程。
15. 切回你的用户：画面跟随。

E. 锁屏

16. 打开「远程结束后自动锁屏」，远程结束 10 秒后锁屏；再连上，在锁屏界面输入密码解锁。

F. 授权提示（macOS 15 起）

17. 观察用户会话里是否出现「继续允许 LemonGalaxyWorker 录屏」，以及登录界面会不会出现；记下版本。

G. 更新

18. 用更高构建号的包替换 `/Applications` 里的应用并打开：日志里后台服务因版本不同重启、会话代理退出后重启；`--unattended-status` 显示新版本。
18b. 自动更新（还没在真机上验证）：先 `sfltool dumpbtm | grep -B2 -A12 lemonit` 记下后台服务条目；在菜单栏菜单或关于页点「立即更新」（也从另一台主控端远程点一次）。预期：不弹任何窗口；`/Library/Application Support/LemonGalaxy/update/update.log` 依次有「开始安装」「已换成」「新版本 … 已启动」「更新完成」；`--unattended-status` 显示新版本、会话代理是新版本；主控端看到设备重新上线后能再连；登录项里仍是已允许，`sfltool dumpbtm` 里的条目没有变成需要批准；重启 Mac 后后台服务照常起来。要验证回滚可以拿一个启动即退出的测试包：日志里有「换回」，旧版本照常在线。如果登录项变成需要批准或后台服务不被 launchd 拉起，记下 macOS 版本与 `log show --predicate 'subsystem == "com.apple.backgroundtaskmanagement" OR process == "launchd"' --last 10m`。

H. 关闭

19. 通用 → 关闭「无人值守」→ 确认：应用重新打开回到普通模式；设备号、账号不变，主控端还是同一台；`/Library/LaunchAgents/cn.lemonit.galaxy.worker.agent.plist` 已删；`sudo launchctl print system/cn.lemonit.galaxy.worker.daemon` 找不到；`/Library/Application Support/LemonGalaxy/worker` 已删；登录项里没有了。
20. （可选）用 test 关闭：退回 test 自己的数据，后台服务的数据保留；之后管理员再开启时不用移交。

出问题时手动清理：

```
sudo launchctl bootout system/cn.lemonit.galaxy.worker.daemon
launchctl bootout gui/$(id -u)/cn.lemonit.galaxy.worker.agent
sudo rm -f /Library/LaunchAgents/cn.lemonit.galaxy.worker.agent.plist /var/run/cn.lemonit.galaxy.worker.sock
sudo rm -rf "/Library/Application Support/LemonGalaxy" /Library/Caches/cn.lemonit.galaxy.worker
sfltool dumpbtm | grep -B2 -A8 lemonit
```

## 6. 已知限制

- 菜单栏应用的被控提示条、设置窗口与会话代理不在同一进程，会出现在远程画面里（共享的 `ScreenCapture.filter` 只排除本进程窗口；改为按 Bundle ID 排除即可）。
- 会话代理不碰 NSScreen，屏幕名称是「Built-in Display」「Display N」这类通用名称。
- 登录界面不提供隐私屏与声音。
- 非管理员不能开启或允许临时协助（第 4 节第 1 条改好后可放开）。
- 删除应用前没关掉无人值守时，按上面的命令清理。

## 参考

- Apple 开发者论坛：[loading SMAppService LoginWindow agent](https://developer.apple.com/forums/thread/726864)、[Global LaunchAgent/LoginItem with SMAppService?](https://developer.apple.com/forums/thread/737038)、[Capturing screen buffer at macOS Login Window with ScreenCaptureKit and PrivilegedHelper](https://developer.apple.com/forums/thread/814152)、[Capture Screen pre-login contex on macOS 15 Sequoia](https://developer.apple.com/forums/thread/768146)、[CGDisplayStreamCreateWithDispatchQueue returns NULL after 30 seconds](https://developer.apple.com/forums/thread/765055)、[CGStreamDisplay and CGEvents for keyboard not working during LoginWindow in intel Mac](https://developer.apple.com/forums/thread/724740)、[ScreenCaptureKit com.apple.developer.persistent-content-capture entitlement?](https://developer.apple.com/forums/thread/761641)
- [macOS Sequoia screen recording prompt monthly（9to5Mac）](https://9to5mac.com/2024/08/14/macos-sequoia-screen-recording-prompt-monthly/)
