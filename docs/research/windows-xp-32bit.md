# Windows 32 位被控端（XP / Vista / 7）调研与实现

- 日期：2026-11-24
- 范围：lemon-galaxy-worker-windows 出 32 位版本，Windows XP SP3（重点）、Vista、Windows 7 的 32 位系统装成被控端，能被远程桌面、终端、文件控制。**只做被控，不做主控**（WPF 主控端要 .NET Framework 4.8，XP 装不了）。
- 方法：读依赖源码（tokio / mio / quinn-udp / socket2 / getrandom / windows 0.62 / Rust 标准库 win7 分支）；下载 YY-Thunks 1.2.3、VC-LTL5 5.3.1 的发布包核对（SHA-256 与 GitHub 发布页一致）；本机用 llvm-mingw 交叉编 32 位程序，列出导入表，对照 YY-Thunks 包里带的 XP / Vista / Win7 系统 DLL 导出表；CI 上用 MSVC 编正式包再核一遍，并在 Windows Server 2025 上跑完整端到端测试。**还没有在 XP / Vista / Win7 实机或虚拟机上跑过。**
- 标记：【已核实】= 源码、发布包或本机实验直接看到；【推测】= 按机制推断；【待验证】= 必须上实机测。

---

## 0. 结论

1. **能做，已做成两份 32 位发布文件**（CI 构建，导入表逐个核对）：
   - `lemon-galaxy-worker-<版本>-windows7-x86.exe`：Vista SP2 / 7 / 8 / 8.1 / 10 的 32 位系统。Rust 的 `i686-win7-windows-msvc` 目标 + YY-Thunks 的 Vista 版补 Win7 起才有的接口。
   - `lemon-galaxy-worker-<版本>-windowsxp-x86.exe`（实验）：XP SP3 / Server 2003 SP2 的 32 位系统，也能在更新的 32 位、64 位系统上跑。同一目标 + YY-Thunks 的 XP 版 + VC-LTL5 的 XP 运行库 + 子系统版本 5.01 + 自己补一个 YY-Thunks 没有的 `WSASendMsg`。
2. **QUIC 在 XP 上可行，不需要退到「只走服务器 TCP 中转」**：LemonGalaxy 的 QUIC 跑在自己的 UDP 套接字（MagicSock，tokio 的 UdpSocket）上，不用 quinn-udp 的收发；tokio 在 XP 上缺的接口 YY-Thunks 都有补，连 mio 必需的 `SIO_BASE_HANDLE` 它也专门处理了。万一 XP 上 UDP 有问题，现有的「强制 TCP 中转」照样可用（同一份代码）。【已核实（源码、YY-Thunks 说明）；能否跑通待验证】
3. **顺带发现现有 Windows 7 兼容版很可能在 Win7 上根本启动不了**：`windows` 0.62 把 `CoTaskMemFree` 指到 Windows 8 才有的 `combase.dll`，被控端的声音、硬件编码代码用了它。已改成从 `ole32.dll` 导入，CI 改成按 Win7 导出表核对。主控端兼容版压缩包里的 `galaxy_ffi.dll` 也导入了 Win8 才有的 `api-ms-win-core-winrt-error-l1-1-0.dll`、`mmdevapi.dll` 里的函数，同样可能起不来，CI 会列出来但不挡发布（主控端不在这次范围）。【导入表已核实；起不来是按系统 DLL 导出表推断，待 Win7 实机确认】
4. XP / Vista 上缺的功能：显卡编码（XP 没有 Media Foundation，32 位不带 NVENC）、伪终端（ConPTY 要 Win10 1809，老系统一直是简化终端）、远程发 Ctrl+Alt+Del（`SendSAS` 是 Win7 起）、AI 的「应用操作」（UI Automation 在 XP 上要另装）。远程桌面（GDI 截屏 + openh264）、键鼠、剪贴板、终端、文件、命令、端口映射、自动更新都在。声音、隐私屏、虚拟屏 2026-11-26 补齐（第 9 节）：XP 录声卡的「立体声混音」，隐私屏用分层黑窗 + GDI 截屏不带 CAPTUREBLT（Vista / 7 也能用），虚拟屏用自写的 XPDM 驱动。
5. 需要拍板的见第 8 节：给 ISO 实测；简化终端要不要换成 winpty；CPU 要求 SSE2。
6. 2026-11-25 起三个兼容版也发桌面端（窗口 + 托盘，自带安装；界面同一份源码按 .NET Framework 4.0 编，XP SP3 起能装），XP 兼容版的被控服务也带了本机控制接口。见 [spec.md](../spec.md) 3.0.2 末尾。发布物叫法：桌面端 / CLI。

---

## 1. Rust 在老系统上的现状

- Rust 标准目标（`*-pc-windows-msvc`）从 1.78 起要求 Windows 10；`i686-win7-windows-msvc` / `x86_64-win7-windows-msvc` 是 tier 3 目标，没有预编译的标准库，要 nightly + `-Z build-std` 自己编。win7 目标的标准库避开了 Win8 / Win10 接口（`WaitOnAddress`、`ProcessPrng`、`GetSystemTimePreciseAsFileTime` 等），锁用 SRWLock、线程停放用 keyed event、随机数用 `RtlGenRandom`。【已核实（nightly-2026-11-20 标准库源码）】
- 但 win7 目标仍然直接导入 Vista 才有的接口：SRWLock、条件变量、`GetFileInformationByHandleEx`、`CreateWaitableTimerExW`、`CompareStringOrdinal`、`InitializeProcThreadAttributeList` 等；创建套接字带 `WSA_FLAG_NO_HANDLE_INHERIT`（Win7 SP1 起才认）、列目录用 `FindExInfoBasic`（Win7 起）。所以 XP 必须补接口，Vista 也要补 Win7 的。【已核实】
- 其他路线：rust9x（改过的 rustc，能到 9x / XP）版本落后、要换整套工具链，不考虑。thunk-rs、oldwin 两个 crate 做的事与下面相同（加 VC-LTL 库路径、链 YY-Thunks 目标文件、设子系统版本），但 thunk-rs 在构建时用 `curl -k` 下载且不校验摘要，oldwin 钉死某个 nightly；我们把同样三步写进 CI 脚本，固定版本与 SHA-256，`Cargo.toml` 不加只给 XP 用的依赖。【已核实（两者源码 / 文档）】

### 1.1 YY-Thunks（补系统接口）

- 原理：链接时把它的目标文件（`objs/x86/YY_Thunks_for_WinXP.obj`）放进去。它定义了 `__imp__函数名@N`（导入表指针）这批符号，链接器就不再从系统 DLL 导入这些函数；运行时先 `GetProcAddress` 找真的，没有就用它的替代实现（比如 SRWLock / 条件变量用 XP 已有的 keyed event 实现，`BCryptGenRandom` 改调 `RtlGenRandom`，`D3D11CreateDevice` 返回失败）。【已核实（目标文件符号、说明表）】
- 版本 1.2.3（2026-10-09），`YY-Thunks-Objs.zip` SHA-256 `4dc68ec8…5b81`。XP 版补 677 个接口、Vista 版 298 个、Win7 版 225 个。包里还带各版本系统 DLL 的导出表（`Config/x86/5.1.2600.txt` 是 XP，`6.0.6000.txt` 是 Vista，`6.1.7600.txt` 是 Win7），正好拿来核对导入表。【已核实】
- 对本工程关键的几条（说明表原文要点）【已核实】：
  - `WSAIoctl`：「低于 6.0 时，`SIO_BASE_HANDLE` 代码返回 SOCKET 自身」——mio 注册套接字第一步就是它，XP 原本不支持会让 tokio 的所有套接字注册失败。
  - `WSASocketW`：「低于 6.1.7601 时自动去除 `WSA_FLAG_NO_HANDLE_INHERIT`」——标准库与 socket2 建套接字都带这个标志。
  - `GetQueuedCompletionStatusEx`：XP 上调 `GetQueuedCompletionStatus`（1.1.7 起一次取多个）；`SetFileCompletionNotificationModes`：XP 上什么都不做。
  - `NtCancelIoFileEx` / `CancelIoEx`：调 `NtCancelIoFile`，会取消这个句柄上全部 I/O（标「不推荐」）。mio 一个 AFD 句柄管多个套接字，改关注事件时会把同句柄上其他套接字的等待也取消掉；mio 收到「已取消」会重新投递，【推测】只是多一点开销，【待验证】。
  - `UpdateProcThreadAttribute`：`PROC_THREAD_ATTRIBUTE_HANDLE_LIST` 被忽略。
  - `SetWindowDisplayAffinity`、`AddClipboardFormatListener`：什么都不做、假装成功（所以隐私屏要按版本号自己挡，见 4.6）。
  - `QueryDisplayConfig`：报告没有驱动；`DwmGetWindowAttribute`：报告 DWM 已关闭。

### 1.2 VC-LTL5（C 运行库）

- 把静态 C 运行库（`libvcruntime.lib`、`libucrt.lib`）换成转调系统自带 `msvcrt.dll` 的版本，XP 上不需要装 VC++ 运行库、也不用 UCRT。XP 版 32 位库在 `TargetPlatform/5.1.2600.0/lib/Win32`，另有两个头文件目录（msvcrt 的区域设置结构与 UCRT 不同）。版本 5.3.1（2025-12-12），`VC-LTL-Binary.7z` SHA-256 `7a18799e…31ad`。5.3.1 起不再自带 YY-Thunks，XP 要自己加（本工程本来就加）。【已核实（发布包、config.cmake）】
- 用法：Rust 用 `crt-static`（相当于 /MT），`-L native=` 指到上面的库目录让链接器先找到它的 `libvcruntime` / `libucrt`；C 依赖（openh264、opus、ring）经 `CFLAGS_i686_win7_windows_msvc` 加它的头文件目录。【按 config.cmake 做法，待 CI 验证】
- Vista / Win7 32 位版不用 VC-LTL，与现有 Win7 x64 版一样用 VS 自带的静态运行库，缺的 Win7 接口由 YY-Thunks Vista 版补，导入表核对保证。

### 1.3 其他链接细节

- XP 的加载器只认子系统版本 5.x，VS 2012 起默认 6.0：XP 版链接加 `/SUBSYSTEM:CONSOLE,5.01`。【已核实（YY-Thunks、thunk-rs 都这么做）】
- 32 位 Rust 目标默认 `/SAFESEH`，openh264 的 NASM 汇编目标文件没有 SAFESEH 表，32 位版设 `OPENH264_NO_ASM=1` 不编汇编（runner 上没装 nasm 时本来也不编）。【已核实（目标规格）】
- 32 位目标默认 `target-cpu=pentium4`，ring 也要求 SSE2：**CPU 要支持 SSE2**（奔腾 4、奔腾 M、速龙 64 起；更早的奔腾 III、速龙 XP 跑不了）。【已核实】

---

## 2. 依赖逐项核对

| 项 | XP 上的情况 | 处理 |
|---|---|---|
| tokio / mio（IOCP + AFD） | `GetQueuedCompletionStatusEx`、`SetFileCompletionNotificationModes`、`NtCancelIoFileEx`、`SIO_BASE_HANDLE` 都是 Vista 起 | 全部由 YY-Thunks 补（见 1.1）。`IOCTL_AFD_POLL` 本身是 XP 的 afd.sys 就有的（Winsock 的 select 靠它）【推测】，【待验证】 |
| quinn（QUIC） | QUIC 走 MagicSock（tokio UdpSocket），不用 quinn-udp 收发；但 quinn-udp 的代码经运行时虚表被链进来，静态导入 `WSASendMsg`（Vista 起，YY-Thunks 没补） | XP 版自己补 `WSASendMsg`（`src/win/xp.rs`：有真的用真的，没有退成 `WSASendTo`）。本机验证：补了之后导入表里没有它了 |
| 双栈套接字 | XP 的 IPv6 是独立协议栈，不支持 `IPV6_V6ONLY=0`（多数 XP 没装 IPv6） | MagicSock 建双栈失败自动退 IPv4（已有代码） |
| rustls + ring（TLS） | 纯 Rust / 汇编，不用系统 SChannel，TLS 1.3 照用；根证书编译在程序里（webpki-roots），XP 过期的根证书库不影响 | 无需处理 |
| 随机数 | getrandom 0.2（ring、rand 0.8 用）导入 `BCryptGenRandom`（bcrypt.dll，Vista 起）；getrandom 0.3 与标准库在 win7 目标下用 `RtlGenRandom`（XP 有） | YY-Thunks 把 `BCryptGenRandom` 转到 `RtlGenRandom` |
| SRWLock / 条件变量 / InitOnce | Vista 起 | YY-Thunks 用 keyed event 实现；parking_lot 运行时探测 `WaitOnAddress`，没有就用 keyed event |
| 系统服务（windows-service） | XP 有服务管理器；「非崩溃退出也重启」、服务 SID 等是 Vista 起 | 安装时这几项失败本来就忽略 |
| 管理员判断 | `TokenElevation` 是 Vista 起，XP 上查不到会当成「不是管理员」，装不了服务 | XP 改用 `IsUserAnAdmin`（`galaxy-host` win.rs） |
| 画面 | DXGI 桌面复制是 Win8 起；XP 没有 d3d11 / dxgi | YY-Thunks 让 `CreateDXGIFactory1`、`D3D11CreateDevice` 返回失败，走已有的 GDI 截屏退路 |
| 编码 | Media Foundation 是 Win7 起（动态加载，失败即跳过）；NVENC 的结构体按 64 位写、NVIDIA 2018 年起不出 32 位系统驱动 | 32 位不编 NVENC；XP 上用 openh264 软件编码 |
| 键鼠 SendInput、剪贴板 | XP 都有 | 无需处理 |
| Ctrl+Alt+Del | `SendSAS` 是 Win7 起（动态加载，失败返回 false） | XP 经典登录界面需要按 Ctrl+Alt+Del 时远程发不了；XP 默认的欢迎屏幕不需要。可考虑 XP 上给 Winlogon 的「SAS window」发 `WM_HOTKEY`（后续） |
| 终端 | 没有 ConPTY（Win10 1809 起），Win7 / Vista / XP 一直是「简化终端」：管道、按行发送、不支持全屏程序 | 这次补上代码页转换：输出按控制台代码页（中文系统 GBK）转 UTF-8，输入反过来，退格按整个字符删；之前中文 Windows 7 的输出是乱码 |
| winpty | 支持 XP 到 Win10（要用 XP 工具集编的发布包，winpty 0.4.x 的 `ia32_xp`）【已核实（winpty README / 发布说明）】；但现有代码**没有接 winpty**（spec 写的是 winpty，实际是简化终端） | 要接的话：随程序带 `winpty.dll` + `winpty-agent.exe`（或内嵌后释放），且 winpty 只能以调用者身份起进程，要以指定账号运行就得先起一个该账号身份的辅助进程再在里面开 winpty。列为后续可选，见第 8 节 |
| PowerShell | XP、Vista 默认没装 | 默认 shell 改成「有 PowerShell 用 PowerShell，没有用 cmd」；命令执行（AI 的 run_command 等）本来就支持 cmd |
| 以指定账号起进程 | 进程属性表（只继承指定句柄、伪终端）是 Vista 起；XP 的 `CreateProcess` 不认 `EXTENDED_STARTUPINFO_PRESENT`【推测】 | XP 上不用属性表、用普通 STARTUPINFO。代价：子进程会继承同一时刻别的会话建的可继承管道端（只有管道，影响很小） |
| 未登录账号的 S4U 登录 | 本机账号走 MSV1_0 的 S4U，XP 是否支持没查到确切资料 | 【待验证】；已登录账号取会话令牌不受影响 |
| 声音 | WASAPI 是 Vista 起 | XP 改录声卡的「立体声混音」（waveIn + 混音器选录音源，`wavein.rs`）；声卡驱动没提供立体声混音就不报能传声音（第 9 节） |
| 隐私屏 | WDA_EXCLUDEFROMCAPTURE 要 Win10 2004（YY-Thunks 会让设置「假装成功」，不能靠返回值） | 按版本号选做法：XP / Vista / 7 用分层黑窗、开着时 GDI 截屏不带 CAPTUREBLT（Vista / 7 先关 Aero），Win8 到 10 1909 不支持（`privacy.rs`，第 9 节） |
| 虚拟屏 | IddCx 驱动要 Win10 1607 起，且只有 x64 驱动 | 显示配置接口（Win7 起）改成运行时加载；XP 用自写的 XPDM 驱动（`driver/xp`，第 9 节），Vista / 7 仍不支持（XPDM 驱动与 WDDM 显卡驱动不能共存） |
| AI 应用操作（UI Automation） | XP 要另装 UIAutomationCore（KB971513） | 失败报错 |
| 文件管理 | 标准库的 `FindFirstFileExW(FindExInfoBasic)`、`GetFinalPathNameByHandleW` 等由 YY-Thunks 补；符号链接 XP 不支持 | 无需处理 |

## 3. 导入表核对（保证程序能启动）

程序导入了目标系统没有的函数，Windows 加载时就报「无法定位程序输入点」或「找不到 xxx.dll」，一行代码都跑不了。所以：

- `scripts/check-legacy-imports.py`：把 `dumpbin /imports` 的结果与 YY-Thunks 包里的系统导出表逐个对照，CI 的三份兼容版每次都核（Win7 x64 版对 Win7 x64 导出表，Vista / 7 32 位版对 Vista，XP 版对 XP）。YY-Thunks 补上的函数链接后就不在导入表里了，所以只看最终导入表。
- 本机用 llvm-mingw 编 32 位被控端（标准库与 C 运行库是 MinGW 的，与正式包不同，只看本工程与依赖库的导入）核对结果【已核实】：
  - 改之前，Win7 缺 `combase.dll!CoTaskMemFree`；Vista 另缺 `QueryDisplayConfig`、`GetDisplayConfigBufferSizes`、`SetDisplayConfig`、`DisplayConfigGetDeviceInfo`、`newdev.dll!DiUninstallDevice`；XP 另缺 `SetDisplayConfig`、`DiUninstallDevice`、`WSASendMsg`。
  - 改之后（`CoTaskMemFree` 从 ole32 导入，显示配置与设备卸载运行时加载，XP 补 `WSASendMsg`），XP、Vista、Win7 三个导出表全部覆盖。
- 正式包的标准库（win7 目标）与 C 运行库（XP 用 VC-LTL）由 CI 核对。

### 3.1 CI 结果（2026-11-24，构建 0.1.0.42）【已核实】

| 版本 | PE 头 | 导入表 | Server 2025 上 |
|---|---|---|---|
| XP 兼容版（windowsxp-x86） | x86，子系统版本 5.01 | 376 个函数、20 个 DLL（含系统自带的 msvcrt.dll），XP 全有 | 截屏（DXGI，YY-Thunks 转给真接口）+ openh264 编码自检通过；端到端全部通过 |
| Vista / 7 32 位版（windows7-x86） | x86，6.00 | 352 个函数、20 个 DLL，Vista 全有 | 端到端全部通过 |
| Win7 x64 版（windows7-x86_64） | x64，6.00 | 354 个函数、21 个 DLL，Win7 全有 | 端到端全部通过 |
| 主控端兼容包的 galaxy_ffi.dll | x64 | Win7 没有：`combase.dll`（`CoTaskMemFree`、`CoCreateFreeThreadedMarshaler`）、`mmdevapi.dll!ActivateAudioInterfaceAsync`、`api-ms-win-core-winrt-error-l1-1-0.dll!RoOriginateErrorW` | 不挡发布，主控端 Win7 兼容包要另修 |

端到端测试包括：登录、自动选路执行命令、终端断线继承、远端声音测试音、8 MB 文件上传下载、复制粘贴文件、强制 UDP / TCP 中转、远程开机、装成 SYSTEM 服务后以指定账号执行命令与写文件。

## 4. 代码改动

1. `lemon-galaxy-worker-windows/src/win/compat.rs`：`co_task_mem_free`（ole32）；`win7` 模块运行时加载 4 个显示配置接口与 `DiUninstallDevice`。
2. `src/win/xp.rs`（只在 `--cfg lemon_galaxy_xp` 时编）：`WSASendMsg` 替身，用 YY-Thunks 同样的办法定义 `__imp__WSASendMsg@24`。
3. `src/win/encode/mod.rs`：32 位不编 NVENC。
4. `src/win/privacy.rs`：按版本号选做法（第 9 节）。
5. `galaxy-host/src/win.rs`：`os_version()`（`RtlGetVersion`）；XP 不用进程属性表；XP 用 `IsUserAnAdmin` 判断管理员；没有 PowerShell 时默认 shell 用 cmd；简化终端的代码页转换与按字符退格。`win_cu.rs`：32 位上 `SHELLEXECUTEINFOW` 是 1 字节对齐的，字段先拷出再用。
6. 自动更新平台名（第 5 节）。

## 5. 自动更新

| 发布文件 | 平台名 | 架构 |
|---|---|---|
| `lemon-galaxy-worker-<版本>-windows-x86_64.exe` / `-aarch64.exe` | `windows` | `x86_64` / `aarch64` |
| `lemon-galaxy-worker-<版本>-windows7-x86_64.exe` | `windows7` | `x86_64` |
| `lemon-galaxy-worker-<版本>-windows7-x86.exe` | `windows7` | `x86` |
| `lemon-galaxy-worker-<版本>-windowsxp-x86.exe` | `windowsxp` | `x86` |

- 被控端按「编译时」决定平台名（`galaxy-worker` 的 `platform_name`）：XP 版编译时加 `--cfg lemon_galaxy_xp` 得 `windowsxp`，win7 目标得 `windows7`；架构取 `std::env::consts::ARCH`，32 位是 `x86`。所以 XP 版装在 Win7 上也只会更新成 XP 版，32 位 Win7 版不会被换成 64 位。
- 服务器按 `galaxy_proto::update::classify_asset` 认发布文件，新文件名有单元测试。

## 6. 构建与 CI

- `scripts/build-windows-legacy.sh <win7-x64|win7-x86|xp>`：固定 nightly（CI 里 `nightly-2026-11-20`）+ `-Z build-std`；下载并校验 YY-Thunks / VC-LTL；编译；`dumpbin` 看 PE 头（XP 版子系统版本应为 5.01）并核对导入表。Win7 x64 版另出主控端兼容版用的 `galaxy_ffi.dll`（导入表也核，但不挡发布）。
- CI：`windows7` 任务改成三份矩阵（不挡发布）；新增「老系统兼容版端到端测试」：在 Windows Server 2025 上把兼容版被控端换进 x64 的测试目录，跑完整 `scripts/e2e.sh`（登录、直连 / UDP / TCP 中转、命令、终端、文件、声音测试音、远程开机、装成 SYSTEM 服务）。这验证的是功能链路（新系统上 YY-Thunks 直接转给真接口），**不能代替老系统实机**。
- 发布说明的下载表加两行；XP 版标「实验」，直到实机测过。

## 7. 实机测试计划（Ubuntu 测试机上的 VMware Workstation）

需要：XP SP3 32 位、Vista SP2 32 位、Win7 SP1 32 位的安装 ISO 与激活方式（微软已不提供下载，要用户给或同意来源）。每个系统：

1. 双击 `lemon-galaxy-worker.exe version`：能启动（验证导入表、子系统版本、C 运行库）。
2. `login` → `install-service`：服务以 SYSTEM 起来，开机自启，主控端设备列表里在线。
3. 连接路径：同一局域网直连；强制 UDP 中转、强制 TCP 中转（`LEMON_GALAXY_FORCE_RELAY`）。
4. 远程桌面：GDI 截屏 + openh264，键鼠、剪贴板（中文）、多显示器；锁屏 / 欢迎屏幕 / 注销后能看到登录界面；画面帧率与 CPU 占用（老机器的软件编码是瓶颈）。
5. 终端：cmd 简化终端，中文输出不乱码，退格、Ctrl+C；断线重连接续。
6. 文件：列目录（含中文名）、上传下载大文件、断点续传、删除目录。
7. 自动更新：从旧版本更新到新版本，平台名不串。
8. 长跑：挂一夜看内存与句柄（YY-Thunks 的 SRWLock / 取消 I/O 替代实现）。

## 8. 风险与需要拍板

1. **XP 版目前只有「能编、导入表对得上、新系统上功能通」，还没有在 XP 上跑过**。最大的不确定：mio 的 AFD 轮询在 XP 的 afd.sys 上是否正常、YY-Thunks 的取消 I/O 语义、VC-LTL 与 Rust 标准库的配合。需要用户给 ISO 后实测。
2. 万一 XP 上 tokio 网络不通（AFD 不可用）：退路是给 mio 打补丁改用 `select` / `WSAEventSelect` 轮询（XP 专用，构建脚本里用 `--config patch` 只替换 XP 版的依赖），工作量中等；或者 XP 版只走 TCP（同样依赖 tokio，帮助不大）。先实测再定。
3. 终端要不要换 winpty（全屏程序、Tab 补全、颜色都能用，但要多带两个文件、以指定账号运行要多一层辅助进程）。现在的简化终端在 XP / Vista / 7 上都能用。
4. CPU 要求 SSE2（2001 年以后的大多数机器都有），更老的不支持。
5. Ctrl+Alt+Del：XP 经典登录界面远程发不了，要不要做 `WM_HOTKEY` 方案。

## 9. 补齐声音、隐私屏、虚拟屏，以及第一次在 XP 上跑（2026-11-26）

测试环境：用户 Ubuntu 测试机（192.168.50.197）上 VMware Workstation 里的 Windows XP Professional SP3 32 位虚拟机（VMware SVGA II 显卡、Sound Blaster PCI 声卡、VMware Tools 10.0.12），用 `vmrun` 拷文件、在登录用户的桌面里运行程序。装驱动前拍了快照。

### 9.1 先用不带 C 运行库的小程序验证可行性（llvm-mingw 编，子系统 5.01）

- **隐私屏**【已核实】：黑色分层置顶窗盖住整屏后，GDI `BitBlt` 带 CAPTUREBLT 截到全黑（非黑像素 0%），不带 CAPTUREBLT 截到的和开之前一样（81%），在黑窗下面新开一个记事本再截，记事本也在（99%）。XP 没有桌面合成，分层窗口由 GDI 另外叠在屏幕上，不在屏幕表面里。Vista / 7 关掉 Aero 后是同一套机制（【推测】，没有实测）。
- **声音**【已核实】：XP 没有 WASAPI，但声卡驱动的录音混音器里有「立体声混音」（类型 SRC_ANALOG，名字按系统语言是「立体声混音」）。用混音器的 MUX 控件把录音源切到它，waveIn 48 kHz 双声道录，同时 waveOut 放 440 Hz，录到的信号过零率折合约 435 Hz；录完把录音源切回原来的（麦克风）。
- **虚拟屏**【已核实】：XP 显示驱动模型（XPDM）可以做不带硬件的帧缓冲显示驱动（`driver/xp`，见驱动 README）。根枚举设备装上约 1 秒、不用重启，系统里多出 \\.\DISPLAY4、\\.\DISPLAY5 两块不接入桌面的「LemonGalaxy Virtual Display」；把 1470x956 写进设备键后 `ChangeDisplaySettingsEx` 接入约 0.16 秒，挪过去的记事本、桌面背景都正常画出，GDI 截那块屏正常；断开正常。DualView 主视图（\Device\Video5）在注册表里有，但系统没列成显示设备，所以目前可用 2 块。卸载设备后显示设备要重启才消失；装着驱动重启，开机正常。

### 9.2 XP 兼容版被控端第一次在 XP 上跑

- 能启动（`version`），`selftest-capture`：GDI 截屏 48 ms，openh264 编码 10 帧 518 ms，JPEG 截图正常；**隐私屏开着时截屏非黑像素 81.5%**（与开之前 81.3% 一样）；声音采集走立体声混音，48 kHz 双声道有数据。【已核实】
- **网络不通**【已核实】：登录时 TCP 连上、TLS 1.3 握手完成、HTTP/2 请求发出，之后一直收不到响应，60 秒超时。根因：XP 没有 NtCancelIoFileEx，YY-Thunks 换成的 NtCancelIoFile 只取消调用线程自己发出的 I/O；mio 会在任意线程（例如 `block_on` 的主线程读到 WouldBlock 后重新登记事件）取消另一个线程发出的 AFD 轮询，XP 上取消不掉，套接字从此等不到「可读」。第 8 节第 1 条担心的「mio 在 XP 上」就是这里，AFD 轮询本身是好的。
- 用不带运行库的 C 小程序在 XP 上复现 mio 的 AFD 轮询【已核实】：AFD 轮询本身正常（可写立即完成；先轮询可读再发数据，数据一到就完成；套接字句柄直接当基础句柄用可以）；**别的线程调 NtCancelIoFile 取消不掉**，发出的那个线程才能取消，取消后完成包状态 STATUS_CANCELLED，GetQueuedCompletionStatus 对这种包返回失败（但带着 overlapped）。
- 抓包【已核实】：请求发出去了，服务器的响应也到了 XP、TCP 已确认，只是程序一直没去读。
- 根因【已核实，读 YY-Thunks 1.2.3 源码 `src/Thunks/ntdll.hpp` 与 `api-ms-win-core-io.hpp`】有两处：
  1. XP 上 NtCancelIoFileEx 的替代实现开头写明「NtCancelIoFile 无法模拟特定 I/O 的取消」，**传了要取消的那次 I/O 就直接返回 STATUS_NOT_FOUND、什么也不取消**（只在不指定 I/O 时才调 NtCancelIoFile 并给其他线程排 APC）。mio 每次取消 AFD 轮询都指定 I/O，又把 STATUS_NOT_FOUND 当成「已经完成」，于是记成取消中、等一个永远不来的完成包，套接字再也收不到事件。CancelIoEx 的替代实现同理（命名管道上取消读操作也不生效）。
  2. XP 上 GetQueuedCompletionStatusEx 的替代实现在 GetQueuedCompletionStatus 返回失败时直接返回失败（第一个包）或跳出循环（后面的包），这时已经取出来的失败完成包（比如 STATUS_CANCELLED 的轮询）被丢掉。
- 修法：`scripts/xp-mio.patch`，XP 版构建时打在 mio 上：① 系统没有 NtCancelIoFileEx 时，AFD 轮询的发出与取消都交给同一个专用线程，取消直接调 XP 自带的 NtCancelIoFile（取消这个 AFD 句柄上专用线程发出的全部轮询，被连带取消的套接字完成后 mio 照常重新发出）；② 系统没有 GetQueuedCompletionStatusEx 时，`CompletionPort::get_many` 自己循环调 GetQueuedCompletionStatus，失败但带 overlapped 的包也交给 mio。Vista 起两处都不生效。只做 ①（且取消仍走替代实现）的 0.1.0.71 / 0.1.0.73 在 XP 上仍然收不到响应；0.1.0.74 起登录、连服务器正常。
- **有声音时服务崩溃**【已核实】：远端听声音时被控服务 3 秒内崩掉重启，前台运行看到 "Rust cannot catch foreign exceptions"，静音时不崩。libopus 的 `x86cpu.c` 判断 AVX2 只看 CPUID（AVX、FMA、AVX2 位），不看 OSXSAVE / XGETBV；XP、Vista、没打 SP1 的 Win7 不支持保存 AVX 寄存器，新 CPU（这台虚拟机的宿主）上一执行 AVX2 就是非法指令。修法：老系统兼容版的目标 `*-win7-windows-msvc` 的 `target_vendor` 是 `win7`，galaxy-host、galaxy-client 对它单独给 opusic-sys 打开 `no-runtime-feature-detection`，只用编译时确定的 SSE / SSE2。
- **exec 的中文输出乱码**【已核实】：exec 用 cmd 时先 `chcp 65001`，XP 上不起作用，输出仍是 GBK。XP 上不切代码页，标准输出、错误按控制台代码页转 UTF-8（与简化终端同一套，双字节字符被拆开时留到下次）。
- 0.1.0.74 在 XP 虚拟机里端到端（Mac 上的临时服务器与 `galaxy-dev`）【已核实】：登录、装成服务并连上服务器；能力列表有声音、隐私屏、虚拟屏（上限 2）；远程截图、看画面（静止时 13 帧 / 秒）；文件上传下载一致；终端断线接续；加 1470x956 虚拟屏 0.19 秒、5 秒收到 62 帧、删屏 0.18 秒；**隐私屏开着时远程截图是正常桌面，同时 XP 本地带 CAPTUREBLT 截屏非黑像素 0%（本地屏幕全黑），关掉后恢复**。声音与 exec 中文的修正在 0.1.0.76 起，还没在 XP 上复测。
- 老系统桌面端：XP 虚拟机里没装任何 .NET，原来的 WPF 界面（.NET 4.0）打不开。改成原生界面（`lemon-galaxy-worker-windows/gui-native`，Rust + Win32），0.1.0.77 起随兼容版发布，导入表核对通过；还没在 XP 上运行过。
- 装服务时顺带装驱动报 0xE000022B（ERROR_DI_DONT_INSTALL）：当时刚卸过一次驱动、显示设备还等着重启，【推测】是显示类安装程序因此拒绝；重启后再测。

### 9.3 0.1.0.79 复测与子进程输出（2026-11-27）

- 声音【已核实】：XP 桌面上放 440 Hz 测试音，远端收到 7.9 秒、估算频率 445 Hz、均方根 0.053；停掉后均方根 0；服务没有崩。exec 中文输出正常（`ver`、`echo 中文测试`、`dir` 的「卷」都对）。
- **原生桌面端安装失败**【已核实】：`LemonGalaxyWorker.exe --install` 复制好文件后调 `lemon-galaxy-worker.exe install-service` 取输出，报 ERROR_INVALID_NAME（123）。根因【已核实，读标准库源码 `library/std/src/sys/process/windows/child_pipe.rs`】：Rust 标准库给子进程建管道（`Command::output()`、`Stdio::piped()`，tokio 的 process 也一样）是打开 `\Device\NamedPipe\` 后用 NtCreateNamedPipeFile 建**空名字**的无名管道，XP 的命名管道文件系统不支持，返回 STATUS_OBJECT_NAME_INVALID，进程根本没启动。不取输出（`Stdio::null()`、继承、文件）的 `spawn` / `status` 不受影响；被控端自己的终端与 exec 用 CreatePipe 建管道，也不受影响。
- 同一原因还会让 XP 上：自动更新与远程升级「先试跑新程序 version」永远判成起不来（升级装不上）；账号用户目录（以前调 `reg query`）查不到；「开机自启」开关（`sc config`）报错。另外收紧数据目录权限用的 `icacls` 是 Vista 起才有的命令，XP 上数据目录（账号令牌在里面）一直没收紧。
- 修法：`galaxy_host::win::output_no_pipe`——标准输出、错误先写进临时目录里独占新建的文件，进程结束后读回删掉，要取子进程输出的一次性命令都改用它（原生界面的 `run_hidden` 同样做法）；用户目录直接读注册表；数据目录用 SetNamedSecurityInfoW（SDDL `D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)`），各版本一致。
- 0.1.0.83（含上面的修法）在 XP 上【已核实】：原生桌面端 `--install` 成功（复制到 Program Files、装服务、顺带更新虚拟屏驱动，徽标提示点「仍然继续」后 3 秒装完）；界面打开正常，概览页显示在线、设备号、服务器与被控服务版本；数据目录 `C:\ProgramData\LemonGalaxy\worker` 的权限只剩 SYSTEM 与 Administrators 完全控制、不继承；服务上报更新通道 `worker/windowsxp/x86`，能力列表有 `remote_update`。
- XP 没有 ProgramData 环境变量，被控服务的数据目录一直是 `C:\ProgramData\LemonGalaxy\worker`；原生界面原来按公共应用数据目录（All Users\Application Data）算，卸载确认框里写的保留位置不对，改成与被控服务同一套算法。
- 远程升级 0.1.0.83 → 0.1.0.84【已核实】：Mac 上 `galaxy-dev remote-update` 发起，15 秒后以 0.1.0.84 重新上线；XP 上桌面端整套（核心、原生界面、许可证）换好，约 75 秒后试运行确认（committed），「添加或删除程序」里的版本改成 0.1.0.84。
- **界面没有自己换成新版本**【已核实根因】：界面每 10 秒对安装位置上的程序调 GetFileVersionInfo 看版本，但它自己就是从这个路径启动的，系统读版本时直接用本进程已加载的映像，读到的永远是自己的版本（不带运行库的小程序在 XP 上复现：进程把自己改名挪开、同一位置放上别的程序后，本进程里读到的仍是自己——它没有版本资源，返回 0、错误 1813；换个进程读同一位置才是新文件的 0.1.0.83）。WPF 版界面（`FileVersionInfo.GetVersionInfo`）同一写法。修法：文件的修改时间或大小变了才复制到临时目录、对副本读版本；已安装版本的显示也走这里。0.1.0.85 起生效（从 0.1.0.84 及更早升级上来时，旧界面仍不会自己换，要重开一次）。
