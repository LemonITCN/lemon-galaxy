# 虚拟屏（虚拟显示器）调研

- 日期：2026-11-22
- 范围：被控端（worker）在无显示器主机上仍可远控，并支持主控端按需新增最多 3 块虚拟屏（分辨率、刷新率由主控指定）。Windows 走 IddCx（间接显示驱动），macOS 走 CGVirtualDisplay。
- 方法：只读各项目的仓库页面、源码、发布说明、许可证和 issue，以及微软/苹果官方文档。**没有安装、下载执行任何驱动或安装包。** 为确认签名者，从公开仓库读取了两个签名目录文件（`.cat`）和一个证书文件（`.cer`），只用 `openssl` 解析证书链，没有执行。
- 标记约定：【已核实】= 从一手来源（源码、官方文档、发布页）直接看到；【推测】= 根据间接证据推断；【待验证】= 必须上机实测才能确定。

---

## 0. 结论速览

1. **Windows 推荐自建 IddCx 驱动**：以 SudoVDA 的驱动设计为蓝本（IOCTL 增删屏 + 看门狗，自带的许可声明是 MIT / CC0），或者从微软官方 IddCx 示例（MS-PL）起步，改硬件 ID、加设备访问控制，在自家 CI 里构建 x64 和 ARM64。签名分两步走：
   - 第一步用 Authenticode 证书签名。可以自购 OV/EV 证书，也可以申请 SignPath Foundation 的免费开源签名（发布者会显示为 SignPath Foundation）。安装时把签名者证书导入本机“受信任的发布者”，就能在 x64 的 Windows 10/11 和 Server 上静默安装。这条路已经有现成先例：VDD 就是这么做的。
   - 第二步走微软 Partner Center 的 attestation 签名。要求公司主体和 EV 证书，签完是微软签名，不用导入任何证书，ARM64 也能装。
2. **现成驱动里，只有 VDD（VirtualDrivers/Virtual-Display-Driver）同时满足“已签名 + 许可允许再分发”**：MIT 许可，由 SignPath Foundation 签名，不是微软签名。但它的控制模型不适合按会话增删单块屏：调数量、换 GPU、改大部分开关都要重载整个适配器，所有虚拟屏一起掉线重建；命名管道对 Everyone 开放全部权限；配置是全局 XML；ARM64 在 24H2 及以后装不上；签名版驱动还停在 2024-12。**只适合做过渡实验，不建议作为正式方案。**
3. **不建议采用的**：
   - Parsec VDD：驱动是 Parsec 的专有二进制，没有找到再分发授权。
   - Amyuni usbmmidd_v2：2021 年后停更，许可条款不明。RustDesk 用户在 Windows 11 25H2（26200.9278）上实测加不出屏，RustDesk 官方已表态要替换它。
   - 原样打包 SudoVDA：它是自签名证书，安装时要往本机根证书库导入第三方根证书。
   - RustDeskIddDriver：没签名，已停更。
4. **Windows 7/8 没有现实可行的软件方案**：IddCx 从 Windows 10 1607 才有；Windows 8 起镜像驱动装不上；Windows 7 的镜像驱动只能镜像现有屏，造不出新屏。建议文档里写明：用 HDMI/DP 显卡诱骗器（假负载）。
5. **macOS 用 CGVirtualDisplay 私有 API**：DeskPad（MIT）、BetterDisplay、Jump Desktop、Chromium 测试代码都在用。Developer ID 签名加公证可以正常分发（公证是自动化安全扫描，不是 App Review），上不了 Mac App Store（审核指南 2.5.1）。DriverKit 没有显示类驱动族，所以没有公开替代方案。兜底同样是诱骗器。
6. **建议修改规格**：`docs/spec.md` 里写的“Windows 10 1607 起支持虚拟屏”建议改成“Windows 10 1803 起（实测下限以 1809 / LTSC 2019 为准），Windows Server 2019 起；ARM64 要等拿到微软签名才支持”。理由见 2.7 节。
7. **工作量粗估**（明细见 5.6）：Windows 约 4–6.5 人周（驱动、服务、agent、测试、签名接入），macOS 约 1–1.5 人周，协议和各端主控界面约 0.5–1 人周，合计约 6–9 人周。签名流程（买 EV、注册开发者计划、微软审核）另需 2–6 周日历时间，可以和开发并行。

---

## 1. Windows 现成 IddCx 驱动逐项评估

### 1.1 总表

| 方案 | 许可 | 签名（谁签 / 类型） | 能否随安装包再分发 | 系统 / 架构 | 运行时控制方式 | HDR / 刷新率 | 维护情况 |
|---|---|---|---|---|---|---|---|
| VDD（MttVDD，VirtualDrivers，原 itsmikethetech） | MIT | **SignPath Foundation**，由 GlobalSign GCC R45 CodeSigning CA 2020 签发，【已核实，解析 .cat】。不是微软签名 | MIT 允许；二进制不能改，改了签名就失效 | Win10/11 x64；ARM64 包签名相同，但 24H2+ 装不上 | XML 配置 + 命名管道 `\\.\pipe\MTTVirtualDisplayPipe`（Everyone 全权限）。改数量、换 GPU、改多数开关都会重载整个适配器 | HDR10 / 12bit 需 Win11 23H2+；支持自定义分辨率和小数刷新率 | 仓库活跃（最后合并 2026-06-22）；签名驱动停在 24.12.24，2025-07-23 重新打包为 25.7.23 |
| SudoVDA（SudoMaker，随 Apollo 分发） | README 写“MIT 和 CC0，任选”；仓库里没有 LICENSE 文件 | **自签名**，CN=sudovda@su.mk，有效期 2025-07-13 至 2030-07-13，EKU 仅限代码签名。Apollo 安装时把它导入 Root 和 TrustedPublisher，【已核实】 | 许可允许；但再分发等于让用户信任一张第三方自签根证书 | Apollo 包里的 INF 只有 NTamd64（x64） | **IOCTL**：增屏（宽、高、刷新率、GUID、名称、序列号）、删屏、设置渲染 GPU、查看门狗、ping、查协议版本 | 640×480 至 7680×4320，最高 500Hz；HDR 在 Win11 24H2 上完整支持 | 驱动仓库最后提交 2025-04-14；Apollo 最后正式版 0.4.6（2025-07-13） |
| Parsec VDD（控制代码来自 nomi-san/parsec-vdd） | 驱动是 Parsec 专有（Parsec 现属 Unity）；nomi-san 的控制代码是 MIT | README 标“已签名”，签名者未核实 | **没找到再分发授权，按不可再分发处理** | 0.38：1607 起（作者标“已过时，可能随机崩溃”）；0.41：19H2 起（作者标“稳定”）；0.45：21H2 起；Server 2019+ | IOCTL：增、删、ping（必须每 100ms 内一次，否则约 1 秒后所有屏被拔掉）；改模式走 Win32 显示 API | 预设分辨率最高 4K 240Hz，另可在注册表加 5 条自定义；**不支持 HDR** | 驱动 0.45（IddCx 1.5）；控制器最后发版 2024-04-30 |
| Amyuni usbmmidd_v2（RustDesk 现在的默认方案） | **没找到明确的再分发条款** | 签名者为 Amyuni Technologies（类型未核实，见第三方指南） | 不明确，需要找 Amyuni 要书面授权 | Win10+，x64 / x86 | IOCTL `2307084`，数据 `0x10` 加一块、`0x00` 减一块，最多 4 块；分辨率用系统 API 改 | 8bit，无 HDR | v2.0.0.1，2021-08 后停更；Win11 26200.9278 上实测加不出屏；RustDesk 表态要替换 |
| RustDeskIddDriver | MS-PL | **无可用签名** | 要自己签 | x64 / x86 | IOCTL：插屏、拔屏、更新模式 | 8bit | 2024-06 后停更；RustDesk 默认已改用 Amyuni |
| virtual-display-rs（Rust 写的） | MIT | 自签名，需要手动导入 Root 和 TrustedPublisher | 问题同 SudoVDA | Win10 2004+，仅 x64 | 自带控制程序（细节未细查） | 8bit | 最后发版 2023-10-22 |
| 微软 IddSampleDriver（Windows-driver-samples） | MS-PL | 无 | 只能当源码参考 | — | 示例只建一块屏；配套示例程序演示了 `SwDeviceCreate` | — | 官方示例 |

### 1.2 VDD（VirtualDrivers/Virtual-Display-Driver，驱动文件名 MttVDD）

- **许可**：MIT。代码基于微软 IddCx 示例，从 ge9/IddSampleDriver fork 而来。仓库 1.06 万星，未关闭的 issue 和 PR 共 210 个，最后推送 2026-09-19。【已核实，GitHub API】
- **发布**【已核实】：
  - 2024-12-24，`24.12.24`：首个“官方签名”版本，资产为 `Signed-Driver-v24.12.24-x64.zip` 和 `-ARM64.zip`。
  - 2025-05-03，`25.5.2`：新增安装器和控制程序，驱动仍是 24.12.24。
  - 2025-07-23，`25.7.23`：便携版控制程序 Virtual Driver Control 也签了名；发布说明写 ARM64 仍需手动安装。
  - winget 上的版本是 25.7.23。
  - issue #531（2026-10-30）的报告者说，25.7.23 驱动包里的 `MttVDD.dll` 和 24.12.24 是同一个文件。之后合入 master 的修复都没进签名版，例如 2026-04 的“HDR 位深”修复和 2026-05 的 PR #482“多屏 swapchain 线程”修复。
- **签名**【已核实】：读取安装器仓库 `VirtualDrivers/Virtual-Driver-Installer` 里的 `mttvdd.cat`（x64 和 ARM64 各一个），证书链是 `SignPath Foundation` ← `GlobalSign GCC R45 CodeSigning CA 2020` ← `GlobalSign Code Signing Root R45`。也就是公共 CA 签发的 Authenticode 证书，**不是微软 attestation 签名，也不是 WHQL**。由此带来几个现象：
  - 安装时会弹“是否信任 SignPath Foundation 发布的软件”。issue #446 里有人在没有信任这个发布者时得到 `0xE0000242`（目录文件的 Authenticode 发布者未被信任）。
  - Windows 11 ARM64 上即使把根证书、中间证书和签名证书都导进去也装不上（issue #197，2024-11；issue #465，2026-01）。维护者给的办法只有关闭驱动签名强制或开测试模式。
  - 同仓库的虚拟声卡是内核驱动（`.sys`），用同一张证书签，在 Server 2025 上报 Code 52 被拒；但报告者明确说**显示驱动在 Server 2025 上正常**（issue #463）。这条可以作为“UMDF 驱动不需要微软签名”的旁证。
- **再分发**：MIT 允许。要注意三点：
  1. 签名绑在原二进制上，改硬件 ID、管道权限之类都要重签。
  2. 如果用户自己也装了 VDD，硬件 ID 都是 `Root\MttVDD`，会冲突。
  3. 驱动依赖 VC++ 运行库，README 要求用户自装，否则缺 `vcruntime140.dll`。
- **安装**：官方安装器（Inno Setup）里的脚本走两步：`pnputil /add-driver MttVDD.inf /install`，然后用 `nefconw --create-device-node --hardware-id Root\MttVDD --class-guid {4d36e968-...}` 建根设备节点（另有 devcon 备用）。INF 同时声明了 `Root\MttVDD` 和 `MttVDD` 两个硬件 ID，后者可以配合 `SwDeviceCreate` 用。【已核实，INF 和脚本】
- **运行时控制**【已核实，Driver.cpp】：
  - 配置文件在 `C:\VirtualDisplayDriver\vdd_settings.xml`，屏数、分辨率列表、刷新率都在里面；部分开关从 `HKLM\SOFTWARE\MikeTheTech\VirtualDisplayDriver` 读。
  - 管道命令有：`RELOAD_DRIVER`、`SETDISPLAYCOUNT n`、`SETGPU`、`HDRPLUS`、`SDR10`、`CUSTOMEDID`、`HARDWARECURSOR`、`GETALLGPUS`、`GETSETTINGS`、`PING` 等。其中改数量、改 GPU、改开关一律调用 `ReloadDriver()` → `InitAdapter()`，整个适配器重新初始化，所有虚拟屏都会断开重连。
  - 管道的安全描述符是 `D:(A;;GA;;;WD)`，即 Everyone 全部权限，任何本地用户都能让驱动重载。issue #490 有人专门问过这一点。
  - **没有“新增一块屏并指定这块屏的分辨率”这种接口**。只能先设总数，再用系统显示 API 从 XML 预置的模式里挑。
- **能力**：HDR 需要 Win11 23H2+（IddCx 1.10）；SDR 支持 8/10bit；支持自定义 EDID、硬件光标、小数刷新率。
- **稳定性**（来自 issue）：
  - #531（2026-10）：Sunshine 在虚拟屏和物理屏之间切换时，驱动在 `Assign/UnassignSwapChain` 里干重活，被 IddCx 约 5 秒的看门狗杀掉（WUDFVerifierFailure）。之后物理屏一直黑屏，要手工修复。
  - #490（2026-05）：4 块虚拟屏在 26200 上反复出现 DWM / DXGI 不稳定。
  - #399：QEMU 加 SR-IOV 核显的虚拟机里，`IDXGIOutputDuplication` 持续返回 `DXGI_ERROR_ACCESS_LOST`，切换到 2K/4K 后也会出现。
  - README 自己也提醒：更新显卡或芯片组驱动前先卸载 VDD，否则可能开机黑屏、显示优先级错乱。
- **生态**：Sunshine 的维护者在 2026-06 的 PR #487 里说“正在考虑把 VDD 直接集成进 Sunshine”。

### 1.3 SudoVDA（Apollo 所用）

- **来源**：`SudoMaker/SudoVDA`，GitHub 上显示是 VDD 的 fork，但 README 说中途已经重写。只在 Apollo（GPL-3.0）安装包里以二进制形式分发，仓库本身没有 Release。【已核实】
- **许可**：README 写“MIT and CC0 or Public Domain (for changes I made, please consult Microsoft for their license)”。仓库里没有 LICENSE 文件，GitHub 识别为无许可证。底层是微软示例，许可为 MS-PL。
- **签名**【已核实】：Apollo 仓库 `src_assets/windows/drivers/sudovda/` 下有 `SudoVDA.inf`、`sudovda.cat`、`sudovda.cer`。证书为自签名（CN=sudovda@su.mk），只有代码签名用途。`install.bat` 先执行 `certutil -addstore -f root` 和 `-addstore -f TrustedPublisher`，再用 `nefconc` 建设备节点、装驱动。INF 版本是 `1.10.9.289`（2025-07-14），只有 `NTamd64` 一节，`UmdfExtensions = IddCx0102`。
- **接口**（`Common/Include/sudovda-ioctl.h`，【已核实】），这是几个方案里设计最贴合我们需求的：
  - `IOCTL_ADD_VIRTUAL_DISPLAY`：入参是宽、高、刷新率、显示器 GUID、名称（14 字节）、序列号（14 字节）；出参是**适配器 LUID 和 TargetId**，可以直接映射到系统显示路径和 DXGI 输出。
  - `IOCTL_REMOVE_VIRTUAL_DISPLAY`：按 GUID 删除。
  - `IOCTL_SET_RENDER_ADAPTER`：按 LUID 指定渲染 GPU。
  - `IOCTL_GET_WATCHDOG` 和 `IOCTL_DRIVER_PING`：看门狗。默认 3 秒内没收到 ping 就移除虚拟屏，控制进程崩溃后能自动清理。
  - `IOCTL_GET_PROTOCOL_VERSION`：查协议版本。
  - 注册表 `HKLM\SOFTWARE\SudoMaker\SudoVDA` 可设：`maxMonitors`（默认 10）、`watchdog`（秒）、`sdrBits`、`hdrBits`、`gpuName`。
- **Apollo 的用法**（`src/platform/windows/virtual_display.cpp`，GPL-3.0，只能参考思路，不能抄代码）：
  - 以看门狗超时的三分之一为间隔起一个 ping 线程。
  - 屏加出来以后，用 `QueryDisplayConfig` 加 `DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME` 找到 GDI 设备名，用 `ChangeDisplaySettingsExW` / `SetDisplayConfig` 改模式和拓扑。
  - 给每个客户端分配固定的显示器身份，让 Windows 自己记住每个客户端的布局。
- **结论**：接口设计最好，但自签名根证书这一点不适合原样塞进远控产品的安装器。**适合作为自建驱动的蓝本。**

### 1.4 Parsec VDD

- **驱动来源**：`builds.parsec.app/vdd/parsec-vdd-0.4x.0.0.exe`，Parsec 官方构建。nomi-san/parsec-vdd 只是独立控制器，许可 MIT，5.7k 星，最后发版 v0.45.1（2024-04-30），仓库最后推送 2026-05-15。【已核实】
- **再分发**：README 没有驱动二进制的再分发条款；Parsec 帮助中心对抓取返回 403，没读到。按专有软件、**未授权再分发**处理。要用只能让用户自己去 Parsec 下载安装，或者找 Parsec（Unity）谈授权。
- **接口**：硬件 ID `Root\Parsec\VDA`；IOCTL 有 ADD（返回序号）、REMOVE、UPDATE、VERSION。UPDATE 就是 ping，**必须每 100ms 内一次**，否则约 1 秒后所有屏被拔掉。每个适配器最多 16 块屏（Parsec 自家产品界面限 3 块）。显示模式只有预设，外加 `HKLM\SOFTWARE\Parsec\vdd` 下最多 5 条自定义；不支持 HDR。静默安装用 `parsec-vdd-0.45.0.0.exe /S`，或者 `nefconw`。
- **已知坑**：Windows 10 按“当前接了哪些显示器的组合”缓存布局（`HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers\Connectivity`），从中间拔掉一块，剩下的组合会回到默认布局；作者的办法是总从右往左拔。issue #91（2025-03）报告 24H2 上新加的屏被识别成内置屏。

### 1.5 Amyuni usbmmidd_v2（RustDesk 当前默认）

- RustDesk 源码里 `src/virtual_display_manager.rs` 的 `IDD_IMPL = IDD_IMPL_AMYUNI`【已核实】。CI 从 `rustdesk-org/rdev` 的 release 下载 `usbmmidd_v2.zip` 打进包里（该 release 日期 2024-07-20）。安装走 `deviceinstaller64.exe install usbmmidd.inf usbmmidd`，或者用 SetupAPI。控制方式是对接口 `{b5ffd75f-da40-4353-8ff8-b6daf6f1d8ca}` 发 IOCTL `2307084`，最多 4 块屏。分辨率没法预设，RustDesk 的做法是插上后再用 `ChangeDisplaySettings` 改成 1920×1080。
- **维护**（RustDesk issue，2026）：
  - #16001（2026-08-30）：报告者指出驱动是 2021-08-31 的 v2.0.0.1，Amyuni 下载文件的 Last-Modified 是 2021-09-09，此后没有 v3；在 Win11 26200.9278 上，驱动接受 IOCTL 但**不出屏**。RustDesk 账号回复“amyuni_idd will be replaced in the future”。
  - #15608（2026-07）：会话结束后设备节点留在系统里，每次开机都会冒出一块 1024×768 的幽灵屏，还会变成主屏。
  - #14034（2026-01）：和用户自己装的 usbmmidd 冲突。
- **许可**：Amyuni 官网 USB Mobile Monitor 页面写的是 Windows 驱动免费、广告支持，另有商业版去广告，没写第三方再分发和 OEM 条款；论坛原帖抓取 404。**按“需书面授权”处理**。【待验证】

### 1.6 RustDeskIddDriver

- `rustdesk-org/RustDeskIddDriver`，MS-PL。基于微软示例（IddCx 1.2），提供插屏、拔屏、更新模式三个 IOCTL。最后发版 2024-02-24，tag 名为 `build_github_ci_sign_off`；仓库最后推送 2024-06-30。
- parsec-vdd 和 VDD 两个项目的对比表都标它“未签名”。RustDesk 自己默认已改用 Amyuni。不建议采用。

### 1.7 其它可参考的实现

- **virtual-display-rs**（MIT，Rust 写的 UMDF IddCx 驱动）：可以参考怎么用 Rust 写 IddCx。但仓库自 2023-10 后没有发版，安装要求用户手动导入自签证书（README 说“证书是私人的事，不应该自动加”）。
- **Looking Glass 的 LGIdd**（gnif/LookingGlass，GPL-2.0）：给虚拟机里的 Looking Glass 用的专用驱动。GPL，不适合并入 MIT 产品。
- **微软 windows-drivers-rs**（Apache-2.0）：官方 Rust WDK 支持 UMDF，但在仓库里搜不到 IddCx 绑定。用 Rust 写 IddCx 驱动要自己生成绑定，**驱动本体建议仍用 C++**，控制侧用 Rust。
- **微软官方示例**（Windows-driver-samples/video/IndirectDisplay，MS-PL）：示例程序用 `SwDeviceCreate(L"IddSampleDriver", L"HTREE\\ROOT\\0", ...)` 创建软件设备，能力标志为 `Removable | SilentInstall | DriverRequired`。

### 1.8 从 SYSTEM 服务静默安装的几种做法（汇总）

| 步骤 | 做法 | 说明 |
|---|---|---|
| 驱动包进驱动库 | `pnputil /add-driver x.inf /install`；或 API `DiInstallDriverW`（newdev.dll）、`SetupCopyOEMInfW` | SYSTEM 身份没法弹窗。签名者不受信任时直接失败，错误码 `0xE0000242`（ERROR_AUTHENTICODE_PUBLISHER_NOT_TRUSTED）。所以非微软签名必须**先把签名证书导入 `LocalMachine\TrustedPublisher`**。 |
| 建根设备节点（持久，重启后还在） | devcon / nefcon 的做法：`SetupDiCreateDeviceInfoW` → 设置 `SPDRP_HARDWAREID` → `SetupDiCallClassInstaller(DIF_REGISTERDEVICE)` → `UpdateDriverForPlugAndPlayDevicesW` | 开机就有适配器，登录前也能用。RustDesk、VDD、SudoVDA 都走这条路。 |
| 建软件设备 | `SwDeviceCreate`（Windows 8+，要求管理员） | 默认句柄关闭就移除设备，进程崩溃即自动清理；要持久可以用 `SwDeviceSetLifetime`。缺点是每次都要重新加载驱动。 |
| 卸载 | 删设备节点（`DIF_REMOVE` / `pnputil /remove-device`），再从驱动库删除（`DiUninstallDriverW` / `pnputil /delete-driver oemNN.inf /uninstall`），最后删掉导入的证书 | RustDesk #15608 说明不清干净就会出幽灵屏。 |

注意：RustDesk 是 AGPL-3.0，Apollo 是 GPL-3.0，两者的 SetupAPI / IOCTL 实现**只能参考，不能复制**进 MIT 仓库。

---

## 2. 自建 IddCx 驱动需要什么签名

### 2.1 规则：内核态和用户态驱动不一样

- 微软《Driver Signing Policy》（2024-08）原文：“Starting with Windows 10, version 1607, Windows will not load any new **kernel-mode** drivers which are not signed by the Dev Portal.” 这条强制要求只针对**内核态**驱动。【已核实】
- IddCx 驱动是 **UMDF 用户态驱动**，运行在 Session 0 的 WUDFHost 进程里。实际观察到的情况：
  - x64 上，非微软签名的 IddCx 驱动可以加载，前提是签名证书链被信任，并且发布者在 TrustedPublisher 里（否则 PnP 会弹窗，或在无人值守时失败）。证据：VDD 用 SignPath Foundation 证书，在 Win10/11 和 Server 2025 上可用（issue #463）；SudoVDA 和 virtual-display-rs 用自签证书加导入根证书也能用。【已核实，多项目旁证】
  - **ARM64（Windows 11 24H2+）上非微软签名装不上**，导入整条证书链也不行（VDD #197、#465；OSR 论坛上也有 UMDF 加 EV 证书在 ARM64 失败的报告）。微软文档没有明文写这条差异。【已核实为现象，规则未见文档】
- 2026-03-26 微软宣布取消对“交叉签名根计划”所签内核驱动的默认信任：2026 年 4 月起在 Win11 24H2/25H2/26H1 和 Server 2025 上先进入评估模式，之后强制。**这只针对内核驱动**，不影响 UMDF 的 IddCx。但它说明驱动信任策略总体在收紧，用户态驱动以后可能也会被要求微软签名。【已核实公告范围；未来趋势属推测】

### 2.2 可选的签名路线

| 路线 | 能装的系统 | 门槛 / 费用 | 安装时要做什么 | 评价 |
|---|---|---|---|---|
| **A. 自有 OV/EV Authenticode 证书**（公共 CA 签发） | x64 Win10/11、Server 2019+（按 VDD 的先例推断）；ARM64 不行 | 证书费用见 2.3。从 2026-02-27 起单张证书最长 459 天（Certum 页面）；私钥必须放在硬件令牌或云 HSM | 把自家签名证书导入 `TrustedPublisher`（**不用导入根证书**，根本来就受信任） | 能落地，和 VDD 现状相同，作用范围只限自家发布者 |
| **B. SignPath Foundation 免费开源签名** | 同 A | 免费。条件：所有组件都是 OSI 开源许可、不能有商业双许可、不能含专有组件；二进制必须由 CI 从源码构建；成员全部启用 MFA；主页要有 “Code signing policy” 页面；发布者显示为 SignPath Foundation | 导入 SignPath Foundation 证书到 `TrustedPublisher` | 省钱；缺点是信任 SignPath Foundation 等于同时信任它签的所有开源驱动，而且发布者名不是我们 |
| **C. 微软 attestation 签名**（Partner Center） | **Win10/11 桌面版，x64 和 ARM64**；文档写 Server 2016+ 不接受 attestation 签名的设备驱动。UMDF 在 Server 上的情况【待验证】 | 要有公司主体、EV 证书、Entra ID 全局管理员、法律联系人；可选 D-U-N-S；要填问卷。官方文档没提注册费 | 什么都不用导入：签的是微软的 Windows Hardware Compatibility Publisher，PnP 直接信任 | **长期首选**。文档现在把它定位为“用于测试场景”，不能上 Windows Update 零售渠道，但驱动随自家安装包分发不受影响 |
| **D. WHQL / HLK 认证** | 全部，包括 Server | 要搭 HLK 测试环境，跑显示类全套测试 | 不用导入 | 成本高，以后要上 Windows Update 或企业客户强制要求时再考虑 |
| E. 测试签名（`bcdedit /set testsigning on`） | 只在用户机器开了测试模式时 | 要关安全启动、桌面有水印、降低整机安全性 | — | **不可行**，不能让用户这样做 |
| F. 自签证书加导入根证书（SudoVDA 的做法） | x64 | 零成本 | 往 Root 和 TrustedPublisher 导第三方自签根 | 技术上可行（仅 UMDF），但远控产品往用户根证书库塞根证书风险大、观感差，可能被杀软或企业策略拦。**不建议** |

### 2.3 EV 证书和 Partner Center 的费用与时间

- EV 代码签名证书。微软列出的可用 CA 有 Certum、DigiCert、GlobalSign、IdenTrust、Sectigo、SSL.com。【已核实】
  - SSL.com EV：买 1 年 349 美元/年，买 2 年 299 美元/年，3 年 249，4 年 200，5 年 149（多年一次付清）。USB 令牌 YubiKey 另加 379 美元，页面还提示它可能不满足内核驱动 EV 的要求。云签名 eSigner 是订阅制，页面没标价。标准验证 3–5 天，加急 2 个工作日另付 599 美元。
  - Certum EV 套装（证书加智能卡加读卡器）：起价 359 欧元。
- Partner Center 硬件开发者计划注册（2026-08 文档）：用 Entra ID 全局管理员登录，填公司信息，接受代码签名协议等，上传 EV 证书，等问卷和审核。文档没写费用和时长【推测：几天到几周】。2025-08 前后有多起“硬件计划账号被停用、工单被直接关闭”的论坛帖。**有账号被冻结的风险，要预留缓冲时间。**
- attestation 提交速度：OSR 论坛 2025-08 的一个案例，ARM64 包 10–15 分钟签完，x64 包进了人工复核用了 3–4 天（那是文件系统过滤驱动，未必适用于我们）。
- 流程：把 `.inf`、`.dll`、`.pdb` 按每个驱动一个子目录打成 CAB，用 EV 证书签 CAB，再提交。微软会重新生成并签名 `.cat`。

### 2.4 测试签名对普通用户是否可行

不可行。原因：要关闭安全启动（Secure Boot）；桌面右下角常驻“测试模式”水印；整机信任任意测试签名的驱动；企业环境基本禁止。VDD 维护者对 ARM64 用户也只能给出这个办法，说明它不能作为产品方案。

---

## 3. Windows 7 / 8 / 8.1

- **IddCx 从 Windows 10 1607（IddCx 1.0）才开始有**，Windows 7/8 上没有这套机制。【已核实，IddCx 版本表】
- **镜像驱动**：微软文档原文 “Starting with Windows 8, mirror drivers will not install on the system.” Windows 7 上的镜像驱动只能镜像现有桌面的一部分，加载后还会关闭 DWM（Aero），**造不出一块新的扩展屏**，解决不了无显示器的问题。【已核实】
- **自写 WDDM 内核显示驱动**：工作量极大。签名方面：交叉签名只认 2015-07-29 前签发的证书，新驱动拿不到；剩下只有 HCK 认证这一条路（微软签名矩阵：Win7/8 不支持 attestation 签名，只认 HLK/HCK 或旧交叉签名）。**不现实。**
- **现实的办法**：
  1. HDMI / DP 显卡诱骗器（EDID 模拟头），几十元人民币，任何系统都能用，分辨率由插头里的 EDID 决定。
  2. 部分显卡驱动自带 EDID 覆盖或强制输出功能（NVIDIA、Intel、AMD 各有做法，需逐个确认）。【待验证】
  3. 虚拟机里用宿主提供的虚拟显卡。
- **建议**：规格保持“Windows 7/8 无虚拟屏”。worker 检测到没有可用输出时（DXGI 枚举不到输出，或 GDI 抓到黑屏），在主控端提示“该主机无显示输出，请接显卡诱骗器”。

---

## 4. macOS

### 4.1 CGVirtualDisplay 私有 API

- **是什么**：CoreGraphics 里没有公开文档的 Objective-C 类，`CGVirtualDisplayDescriptor`、`CGVirtualDisplay`、`CGVirtualDisplaySettings`、`CGVirtualDisplayMode`。需要自己写头文件声明。【已核实，DeskPad 的 `CGVirtualDisplayPrivate.h`】
  - descriptor 可设：`name`、`maxPixelsWide/High`、`sizeInMillimeters`、`vendorID/productID/serialNum`、`queue`、`terminationHandler`。
  - settings 可设：`modes`（模式数组，每个是宽、高、刷新率）和 `hiDPI`。
  - `initWithDescriptor:` 创建屏，`applySettings:` 设置模式，`displayID` 拿到对应的 `CGDirectDisplayID`。
- **系统版本**：Chromium 文档写“Requires macOS version 11.0+”，并提示“relies on undocumented CoreGraphics APIs and risks breakage at any time”；Chromium 已在苹果 Feedback 提了公开 API 需求（FB12349099）。DeskPad 部署目标是 macOS 13。我们的 macOS worker 最低也是 13，版本上没问题。【已核实】
- **谁在用**：
  - DeskPad（MIT，约 8k 星）。
  - BetterDisplay：5.x 要求 macOS 26.3+，宣称“HDR 和高刷新率虚拟屏在兼容机型上可用”。
  - Jump Desktop：Mac 版从 8.9 开始支持虚拟屏，2026 年 10.x 版本仍在修相关问题。
  - Chromium 的多屏测试代码。
  - 苹果自家“高性能屏幕共享”（macOS 14+，两端都要 Apple 芯片）可选 1–2 块虚拟屏，单块最大 4K（3840×2160 或 1920×1080 HiDPI）。是否用的是同一套 API 未公开。【推测】
- **几条已知行为**：
  - 必须在有 WindowServer 会话的进程里创建。LaunchDaemon、没有图形会话的 CI 机器会失败（go-macos/virtualdisplay 文档，二手来源）。
  - 屏的生命周期等于这个对象的生命周期；进程退出后系统会回收。二手来源称“改过模式后再释放对象，屏要等进程退出才消失”【待验证】。移除是异步的，有测量值约 0.7 秒才从系统屏幕列表里消失。
  - macOS 14 要求不同虚拟屏用不同的 `serialNum`（Chromium 源码注释）。
  - 应用沙盒下需要 `com.apple.security.temporary-exception.mach-lookup.global-name = com.apple.VirtualDisplay`（DeskPad 的 entitlements）。我们的 worker 没开沙盒，不受影响。
  - HDR：BetterDisplay 新版宣称支持，旧讨论里作者说 CGVirtualDisplay 不支持 HDR。具体用的哪个接口没公开。【待验证】第一版不做 HDR。

### 4.2 分发与审核

- **Developer ID 加公证可以发**：苹果原文 “Notarization of macOS software is not App Review. The Apple notary service is an automated system that scans your software for malicious content, checks for code-signing issues”。公证不检查私有 API。BetterDisplay、Jump Desktop、DeskPad 都是这样分发的。【已核实】
- **Mac App Store 不行**：审核指南 2.5.1 “Apps may only use public APIs”。指南上该条带有 Notarization 标记，但那是指 iOS/iPadOS 的公证分发，不涉及 macOS 的 Developer ID。【已核实】

### 4.3 替代方案

- **DriverKit 没有显示类驱动族**：官方 DriverKit 文档列出的族只有 USB、HID、Networking、PCI、Serial、Audio 等，没有显示或帧缓冲。所以没有公开 API 能写虚拟显示器驱动。【已核实，文档列表】
- 内核扩展（kext）方向已被苹果淘汰，Apple 芯片机器上更不现实，不考虑。
- **兜底**：HDMI 诱骗器（Mac mini、Mac Studio 等无头机最常见的做法）；或者让用户启用自动登录，配合 worker 自动建屏。

### 4.4 风险

- 私有 API，任何系统版本、甚至小版本都可能改或删。必须在运行时探测，缺失时自动降级。
- ScreenCaptureKit 在多块虚拟屏增删后会抓错屏：新建的 SCStream 总在推最后接入的那块虚拟屏，即使过滤器指定的是别的屏（苹果开发者论坛，2025-06，FB17797423；发帖者说这是 SCK 出现以来就有的老问题）。应对：拓扑变化后重建 SCShareableContent 和 SCStream，并核对输出尺寸和 displayID。
- Jump Desktop 的更新日志（2026-06，10.14.5）修过“Intel Mac 用过虚拟屏后变成无显示、直到重启”的问题，所以 Intel 机型要单独测。
- 登录前（登录窗口）的情况：我们的 macOS worker 是登录后启动的菜单栏应用，登录前无法建屏。无头 Mac 要么开自动登录，要么接诱骗器。

---

## 5. 给 LemonGalaxy 的建议

### 5.1 方案选择

| 平台 | 正式方案 | 过渡 / 兜底 |
|---|---|---|
| Windows 10 1803+ / 11 x64，Server 2019+ | 自建 IddCx 驱动 `LemonGalaxyVDD`（C++ UMDF），先用 Authenticode 签名（路线 A 或 B），之后加 attestation（路线 C） | 主机上已经装了 VDD、SudoVDA 或 Parsec 时，只识别不接管；需要时提示接诱骗器 |
| Windows 11 ARM64 | 拿到 attestation 签名后开启 | 在此之前，ARM64 上的虚拟屏能力报“不支持” |
| Windows 7 / 8 / 8.1 | 不支持 | 诱骗器提示 |
| macOS 13+ | worker 进程内用 CGVirtualDisplay | 探测失败就降级并提示诱骗器 |

为什么不直接打包 VDD：它的控制粒度是“全局屏数加重载适配器”，无法单独增删一块屏，而增删任何一块都会让其它正在被抓的虚拟屏断开重连；管道对 Everyone 开放；配置放在 `C:\` 根下的全局目录；签名版驱动 2024-12 之后没有更新；ARM64 不可用；还会和用户自装的 VDD 冲突。如果想先给内部测试用，可以做成“检测到用户已装 VDD 就通过管道和 XML 用它”的适配层，而不是由我们来打包。

### 5.2 驱动设计要点

- **起点**：fork SudoVDA 的驱动，按其 README 以 MIT/CC0 许可使用，同时保留微软示例的 MS-PL 声明。也可以直接从微软示例起步，参考 SudoVDA 的 IOCTL 设计。
- **身份**：硬件 ID 用 `Root\LemonGalaxyVDD`；设备接口 GUID 和厂商 ID 全部自定义，避免和其它虚拟屏驱动冲突。
- **IOCTL**：
  - `ADD{w,h,hz,guid,name,serial}` 返回 `{adapterLuid,targetId}`
  - `REMOVE{guid}`
  - `LIST` 返回当前所有屏（SudoVDA 没有，建议加上，服务重启后对账用）
  - `SET_RENDER_ADAPTER{luid}`
  - `PING` 和 `GET_WATCHDOG`
  - `GET_VERSION`
  - 屏数上限：3 块“扩展屏”加 1 块“无头兜底屏”。
- **安全**：设备对象和设备接口设置 DACL，只允许 SYSTEM 和 Administrators 打开（例如 `D:P(A;;GA;;;SY)(A;;GA;;;BA)`，可写在 INF 里，或用 `WdfDeviceInitAssignSDDLString`）。不开命名管道。所有输入严格校验尺寸。
- **看门狗**：默认 5–10 秒没收到 ping 就自动移除我们建的屏，保证服务崩溃或升级失败后不留幽灵屏。“断开后保留”由服务记下期望状态，重启后重新加回来，不靠驱动持久化。
- **显示器身份**：每块屏的 EDID 序列号和 GUID 按“主控设备 + 槽位”固定，Windows 就能记住每台主控对应的布局（Apollo 的做法）。不要每次随机生成，否则会触发前面提到的 Connectivity 缓存问题。
- **IddCx 版本**：针对新版 WDK 构建，运行时用 `IddCxGetVersion` 判断有哪些功能可用。微软原话：“A single IDD binary built against IddCx 1.8 can run on Windows 10, version 1803 and above using runtime checks”；按 1.10 构建是否同样适用【待验证】。`IddCxAdapterSetRenderAdapter` 要 1.4（Windows 10 1903+）才有；HDR 要 1.10（Win11 23H2+），第一版只做 SDR 8bit，和现在的 BGRA 编码链路一致。
- **运行库**：CRT 静态链接，避免 VDD 那样依赖 VC++ 运行库。
- **构建**：CI 用 windows-2022 runner 加 WDK，产出 x64 和 ARM64 两种。签名私钥放在 CI Secrets 或云 HSM（需要拍板，见 5.7）。
- **在 swapchain 回调里不做重活**：吸取 VDD #531 的教训，`AssignSwapChain` / `UnassignSwapChain` 里只发信号，不阻塞，不在里面等线程退出。

### 5.3 和现有 SYSTEM 服务、会话 agent 的集成

```
主控 ──(Mgmt: VirtualDisplayAdd/Remove/List)──▶ worker 服务 (SYSTEM, Session 0)
                                                 │ 1. 安装/修复驱动（首次需要时）
                                                 │ 2. 打开设备接口，发 IOCTL ADD → {luid,targetId}
                                                 │ 3. ping 线程（看门狗）
                                                 ▼
                                    会话 agent（SYSTEM，跑在控制台会话的输入桌面上）
                                                 │ 4. QueryDisplayConfig: targetId → \\.\DISPLAYn
                                                 │ 5. SetDisplayConfig 扩展摆放 / ChangeDisplaySettingsExW 设模式
                                                 │ 6. 现有 DXGI 采集：按 DeviceName 匹配输出 → DuplicateOutput
                                                 ▼
                                   DisplayInfo{is_virtual:true} 上报，Display 流照常
```

1. **安装**：安装器只把驱动文件放进安装目录，不立刻装驱动。服务第一次收到虚拟屏请求、或者开机发现无头时，按顺序做三件事：
   - 非微软签名时，把签名证书导入 `TrustedPublisher`；
   - `DiInstallDriverW` 把驱动包放进驱动库；
   - 用 SetupAPI 建根设备节点 `Root\LemonGalaxyVDD`，并等待接口出现。
   安装失败要上报具体错误码（例如 `0xE0000242`），主控端显示“虚拟屏不可用：原因”。这样不需要虚拟屏的用户就完全不碰驱动。
2. **枚举**：服务启动时先打开接口，用 `LIST` 和期望状态对账，把多余的屏删掉、缺的屏补上。agent 用 `QueryDisplayConfig(QDC_ALL_PATHS)` 加 `DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME`（显示器设备路径里能看到我们的硬件 ID 和 EDID）区分哪些是我们的屏，在 `DisplayInfo` 里把 `is_virtual` 置为 true（协议里已有这个字段）。
3. **增屏**：服务发 `ADD`；agent 等系统显示变化（`WM_DISPLAYCHANGE`，或轮询 QueryDisplayConfig，加超时），然后把新屏扩展摆放到现有屏的右侧，设好主控要求的模式，再回 `display_id`。
   - 改模式和拓扑要放在会话里的 agent 上做，不要在 Session 0 的服务里调。Sunshine、Apollo、RustDesk 都是这么做的；Session 0 下的具体限制【推测，待验证】。
4. **采集**：现有 `capture.rs` 遍历所有适配器的输出、按 `DeviceName` 匹配，可以直接复用。IDD 输出能被 DXGI 枚举并做桌面复制（Apollo 就是按 LUID 找适配器再按 GDI 名找输出）。要做好 `DXGI_ERROR_ACCESS_LOST`：现在的实现是报错退出，需要改成自动重建复制会话，因为改模式、增删屏都会触发它（VDD #399）。光标沿用现有的 GDI 叠加方式。
5. **无头兜底**：服务开机和显示变化时检查。如果没有任何活动的显示路径，就自动加一块 1920×1080@60 的“无头屏”，这样登录界面（Winlogon 桌面）和 UAC 也能抓到。接上物理屏后，可以按配置移除。
6. **删屏和清理**：主控断开时，按“断开后保留”的设置决定删或不删，删的时候从右往左删。服务停止时停掉 ping，驱动看门狗兜底。卸载时依次删屏、删设备节点、删驱动包、删导入的证书。
7. **多 GPU**：用 `SET_RENDER_ADAPTER` 指向编码所用的 GPU（笔记本双显卡要特别注意），让 DXGI 采集和编码在同一块 GPU 上。没有 GPU 的虚拟机会落到 WARP 或 Basic Render（IddCx 1.6 文档提到 WARP 做渲染适配器时 swapchain 在系统内存里）。【待验证】
8. **和隐私屏联动**：规格里的“关闭物理屏改走虚拟屏”可以用 `SetDisplayConfig` 只保留虚拟路径来实现（RustDesk 的隐私模式 2 就是这个思路）。必须有恢复看门狗：会话断开、超时或进程崩溃时，恢复原来的拓扑，避免物理屏一直黑屏（VDD #531）。
9. **协议增量**：在 `Mgmt` 里增加以下请求，并在 `capabilities` 里加一项 `virtual_display`：
   - `VirtualDisplayCaps`：返回 `{supported, reason, max:3, modes_hint}`
   - `VirtualDisplayAdd{width,height,refresh,keep_after_disconnect}`：返回 `{display_id}`
   - `VirtualDisplayRemove{display_id}`
   - `ListDisplays` 继续沿用，带 `is_virtual`

### 5.4 macOS 集成

- 在 worker 的 Swift 层用桥接头文件声明 CGVirtualDisplay（可参考 DeskPad 的 MIT 头文件），封装成 `VirtualDisplayManager`：
  - `create(w,h,hz,hiDPI,slot)` 返回 `CGDirectDisplayID`
  - `remove(id)`
  - 持有对象引用；进程退出时系统自动回收
- 启动时用 `NSClassFromString("CGVirtualDisplay")` 探测，类不存在就把能力报为“不支持”。另外加一个远程开关，出问题时可以一键关掉。
- 设置 descriptor 的 `maxPixelsWide/High` 时要不小于请求的尺寸；`modes` 里包含请求的模式（要 HiDPI 时同时给 2 倍尺寸）；`serialNum` 按槽位固定且互不相同。建好后刷新 `SCShareableContent`，按 displayID 找到 `SCDisplay`，交给现有采集链路。
- 无头 Mac：会话建立时如果没有在线的物理屏，自动建一块。登录前不支持，在文档里写明“开启自动登录，或者接诱骗器”。

### 5.5 风险清单

| 风险 | 影响 | 缓解 |
|---|---|---|
| ARM64 需要微软签名 | ARM64 用户暂时没有虚拟屏 | 尽早启动 EV 证书和 Partner Center 注册 |
| 微软继续收紧驱动信任策略（例如将来 UMDF 也要求微软签名） | Authenticode 路线失效 | 把 attestation 作为长期目标；能力探测失败时降级 |
| Partner Center 账号审核或冻结 | 拿不到 attestation 签名 | 预留时间；保留 Authenticode 签名包作为后路 |
| 驱动缺陷：看门狗杀进程、DXGI ACCESS_LOST、显卡驱动更新后黑屏 | 画面中断，甚至物理屏黑屏 | 回调里不做重活；采集自动重建；拓扑恢复看门狗；显卡驱动更新后自检 |
| 和用户已装的 VDD、SudoVDA、Parsec、Amyuni 冲突 | 屏数和布局混乱 | 用独立硬件 ID；只管理自己建的屏；检测到其它 IDD 时在日志里提示 |
| 企业 WDAC 策略或杀软拦截非微软签名驱动，或拦截证书导入 | 安装失败 | 上报明确错误；提供 attestation 签名的版本 |
| Windows Connectivity 布局缓存 | 布局错乱 | 固定显示器身份；从右往左删 |
| macOS 私有 API 变动 | 新系统上能力丢失 | 运行时探测、远程开关、每个系统 beta 都回归测试 |
| SCK 抓错虚拟屏（FB17797423） | 主控看到别的屏 | 拓扑变化后重建 stream，并核对尺寸和 ID |
| 许可合规 | 法律风险 | 只参考 RustDesk（AGPL）、Apollo（GPL）、Looking Glass（GPL）的代码，不复制；保留 MS-PL 和 MIT 声明 |

### 5.6 工作量估算（单人，按人日）

| 项 | 估计 |
|---|---|
| 驱动 fork 和改造（硬件 ID、IOCTL 加 LIST、访问控制、看门狗、静态 CRT、x64/ARM64 CI 构建） | 4–6 |
| 服务侧 Rust：安装、卸载、修复（SetupAPI、newdev、证书导入），IOCTL 客户端，ping，对账，无头兜底 | 4–6 |
| agent 侧：路径映射、拓扑和模式、采集适配（ACCESS_LOST 自动重建）、隐私屏联动和恢复 | 5–8 |
| 协议、主控 UI（Windows WPF、macOS、iOS、Android：最多加 3 块屏，选分辨率和刷新率） | 3–5 |
| Windows 测试矩阵：10 1809/21H2/22H2、11 23H2/24H2/25H2、Server 2019/2022/2025、ARM64、NVIDIA/AMD/Intel、无 GPU 虚拟机、双显卡笔记本、锁屏和 UAC、升级和卸载 | 5–8 |
| macOS：VirtualDisplayManager、采集和 UI 接入、13/14/15/26/27 加 Intel 机型回归 | 5–7 |
| 签名接入：Authenticode CI 接入 1–2；attestation 流程打通 1–2 | 2–4 |
| **合计** | **约 28–44 人日（约 6–9 人周）**；签名的日历时间另算 2–6 周，可并行 |

### 5.7 需要拍板的事项

1. 是否以公司主体购买 EV 证书（约 250–360 美元/年起，外加硬件令牌或云 HSM），并注册微软硬件开发者计划，走 attestation 签名。
2. 第一阶段签名用哪个：SignPath Foundation（免费，但发布者显示为 SignPath Foundation，还要满足“全开源、CI 构建、MFA、签名策略页面”等条件），还是自购 OV/EV 证书（直接用第 1 条的 EV 证书即可）。
3. 规格下限是否改成“Windows 10 1803+（以实测 1809 为准）/ Server 2019+，ARM64 以拿到微软签名为准”。
4. 是否在正式驱动之前，先做“兼容用户已装的 VDD”的适配层，供内部测试。

---

## 6. 来源（访问日期均为 2026-11-22）

**Windows 驱动项目**
- VDD 仓库与 README：https://github.com/VirtualDrivers/Virtual-Display-Driver
- VDD 发布（24.12.24 / 25.5.2 / 25.7.23）：https://github.com/VirtualDrivers/Virtual-Display-Driver/releases
- VDD 驱动源码（管道、重载、SDDL）：https://github.com/VirtualDrivers/Virtual-Display-Driver/blob/master/Virtual%20Display%20Driver%20(HDR)/MttVDD/Driver.cpp
- VDD 安装器（脚本、签名目录文件）：https://github.com/VirtualDrivers/Virtual-Driver-Installer
- VDD issue：#197 https://github.com/VirtualDrivers/Virtual-Display-Driver/issues/197 ，#446 …/issues/446 ，#463 …/issues/463 ，#465 …/issues/465 ，#399 …/issues/399 ，#490 …/issues/490 ，#531 …/issues/531 ；Sunshine 集成意向 PR #487 …/pull/487
- SudoVDA：https://github.com/SudoMaker/SudoVDA （IOCTL 头文件 `Common/Include/sudovda-ioctl.h`）
- Apollo（SudoVDA 安装脚本、证书、集成代码）：https://github.com/ClassicOldSong/Apollo （`src_assets/windows/drivers/sudovda/`，`src/platform/windows/virtual_display.cpp`）
- parsec-vdd：https://github.com/nomi-san/parsec-vdd （`core/parsec-vdd.h`）
- RustDesk 虚拟屏管理：https://github.com/rustdesk/rustdesk/blob/master/src/virtual_display_manager.rs ；issue #16001 https://github.com/rustdesk/rustdesk/issues/16001 ，#15608 …/issues/15608 ，#14034 …/issues/14034
- RustDeskIddDriver：https://github.com/rustdesk-org/RustDeskIddDriver
- Amyuni USB Mobile Monitor：https://www.amyuni.com/en/desktop-editions/usb-mobile-monitor/learn-more
- virtual-display-rs：https://github.com/MolotovCherry/virtual-display-rs
- 微软 IddCx 示例：https://github.com/microsoft/Windows-driver-samples/tree/main/video/IndirectDisplay
- windows-drivers-rs：https://github.com/microsoft/windows-drivers-rs
- Looking Glass：https://github.com/gnif/LookingGlass

**微软文档**
- IddCx 概览：https://learn.microsoft.com/en-us/windows-hardware/drivers/display/indirect-display-driver-model-overview
- IddCx 版本表（2026-04-01）：https://learn.microsoft.com/en-us/windows-hardware/drivers/display/iddcx-versions
- IddCx 1.8 说明（单一二进制可跑 1803+）：https://learn.microsoft.com/en-us/windows-hardware/drivers/display/iddcx1.8-updates
- 驱动签名策略：https://learn.microsoft.com/en-us/windows-hardware/drivers/install/kernel-mode-code-signing-policy--windows-vista-and-later-
- 签名方式对比（2026-03-23）：https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings
- attestation 签名流程：https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-attestation
- 代码签名证书要求（EV、可用 CA）：https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-reqs
- 硬件开发者计划注册（2026-08-11）：https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/hardware-program-register
- SwDeviceCreate：https://learn.microsoft.com/en-us/windows/win32/api/swdevice/nf-swdevice-swdevicecreate
- 镜像驱动（存档）：https://learn.microsoft.com/en-us/windows-hardware/drivers/display/mirror-drivers
- 取消交叉签名内核驱动信任（2026-03-26，2026-10-22 更新）：https://techcommunity.microsoft.com/blog/windows-itpro-blog/advancing-windows-driver-security-removing-trust-for-the-cross-signed-driver-pro/4504818

**签名服务与证书价格**
- SignPath Foundation 条款：https://signpath.org/terms
- SSL.com EV 代码签名：https://www.ssl.com/products/software-integrity/code-signing/ev/
- Certum EV 代码签名：https://shop.certum.eu/certum-ev-code-sigining.html
- OSR 论坛 attestation 耗时案例：https://community.osr.com/t/driver-signing-manual-review-for-x64-build-while-arm64-build-gets-signed-immediately/59812
- 硬件计划账号被停用的帖子（2025-08）：https://techcommunity.microsoft.com/discussions/partnershelpingpartnerstechtopics/windows-hardware-developer-program-deactivated-in-partner-center---support-cases/4445333

**macOS**
- DeskPad（私有头文件、用法、沙盒例外）：https://github.com/Stengo/DeskPad
- Chromium 多屏测试文档（macOS 11+、私有 API 风险、FB12349099）：https://chromium.googlesource.com/chromium/src/+/a7f3967921b2960c9e901b027cbefd2536715615/docs/ui/display/multiscreen_testing.md
- go-macos/virtualdisplay（生命周期等行为，二手来源）：https://pkg.go.dev/github.com/go-macos/virtualdisplay
- BetterDisplay：https://github.com/waydabber/BetterDisplay
- SCK 抓错虚拟屏（FB17797423）：https://developer.apple.com/forums/thread/786829
- Jump Desktop 更新日志：https://changelog.jumpdesktop.com/
- 苹果高性能屏幕共享（虚拟屏限制）：https://support.apple.com/guide/remote-desktop/apdf8e09f5a9/mac
- 苹果公证说明：https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
- App Store 审核指南 2.5.1：https://developer.apple.com/app-store/review/guidelines/
- DriverKit 框架文档：https://developer.apple.com/documentation/driverkit
