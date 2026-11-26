# LemonGalaxy 虚拟屏驱动（Windows，IddCx）

XP / Server 2003 用的另一份驱动在 `xp/`（XPDM，32 位），见文末「XP 虚拟屏驱动」。

驱动与独立测试工具；第二阶段已接进 worker（协议见 `docs/protocol.md` 8.6，实施记录见 `docs/research/virtual-display.md` 5.9）。

worker 自己安装 / 卸载驱动（管理员）：

```bat
rem 测试签名包只在开发机上装（--dev 或 LEMON_GALAXY_DEV_DRIVER=1，会信任测试证书）
lemon-galaxy-worker install-display-driver --dev D:\dev\lemon-galaxy-driver\build\x64\Release\LemonGalaxyDisplay
lemon-galaxy-worker uninstall-display-driver
```

不写目录时用程序旁边的 `driver\`；`install-service` 时程序旁边有 `driver\` 包也会顺带装。

## 内容

| 路径 | 说明 |
|---|---|
| `LemonGalaxyDisplay/` | IddCx 用户态驱动（UMDF 2.27，按 IddCx 1.4 头文件编译，最低 IddCx 1.3 / Windows 10 1809）。一个根枚举适配器 `Root\LemonGalaxyDisplay`，启动时没有屏，最多 4 块（3 块给主控 + 1 块无头兜底） |
| `LemonGalaxyDisplay/LemonGalaxyDisplayControl.h` | 控制接口：设备接口 GUID `{BE3C34E4-6555-40C7-916D-A2DB09DAB193}`，`IOCTL_LG_DISPLAY_GET_INFO / ADD / REMOVE` |
| `LemonGalaxyDisplayTest/` | 独立测试工具（C++，Windows 子系统，不弹窗口，结果写文件） |
| `tools/` | `ewdk.cmd`、`build.cmd`（EWDK 编译 + 签名）、`sign.ps1`、`install.ps1`（安装 / 卸载）、`run-test.ps1`（计划任务在控制台会话跑测试）、`sync.sh`（从 Mac 同步到 Windows 开发机） |

驱动移植自 LemonLink 的 `LemonLinkDisplay`，改了名字、接口 GUID、硬件 ID、UMDF 服务名、EDID 厂商码（`LGX`）、日志路径和屏数上限，能和已装的 LemonLink Display、GameViewer 等虚拟屏驱动共存。

要点：
- 一块屏属于加它的句柄，句柄关闭（进程退出或崩溃）时驱动自动删屏。
- 加屏后 Windows 不会自动扩展桌面，调用方要 `SetDisplayConfig` 只加这一条路径（见测试工具 `Activate`）。
- 模式由调用方在 ADD 里给（最多 64 个），驱动原样上报；测试工具给的是常见 16:9/16:10 共 12 种分辨率 x 30/60/120/144 Hz。
- 设备只对 SYSTEM 和管理员开放（INF 里的 SDDL），所以测试工具要以管理员运行。
- 改模式后第一个 swap chain 常常 `IddCxSwapChainSetDevice` 返回 ACCESS_LOST（0x887A0026），第二个正常，不影响使用。
- 驱动日志：`C:\ProgramData\LemonGalaxy\Logs\LemonGalaxyDisplay.log`（`install.ps1` 建目录并给 LOCAL SERVICE 写权限）。

## 编译（Windows 开发机，EWDK）

```sh
# 在 Mac 上：同步到 D:/dev/lemon-galaxy-driver 并编译
LG_WIN_HOST=user@host LG_WIN_KEY=~/.ssh/key lemon-galaxy-worker-windows/driver/tools/sync.sh build Release x64
```

`build.cmd` 挂载 `D:\dev\EWDK_28000.iso`（可用 `LG_EWDK_ISO` 改），msbuild 编出 `build\x64\Release\LemonGalaxyDisplay\`（inf/dll/cat）和 `build\x64\Release\LemonGalaxyDisplayTest.exe`，再由 `sign.ps1` 用测试证书 `CN=LemonGalaxy Test Driver Signing`（放在本机 `LocalMachine\My`，没有就生成，5 年有效）签 `.cat`，并导出公钥 `LemonGalaxyTest.cer`。工程里 `SignMode` 默认关掉，不用 WDK 自动生成的 WDKTestCert。

## 安装与测试（管理员）

```bat
powershell -ExecutionPolicy Bypass -File D:\dev\lemon-galaxy-driver\tools\install.ps1
powershell -ExecutionPolicy Bypass -File D:\dev\lemon-galaxy-driver\tools\run-test.ps1
```

`install.ps1` 把测试证书放进本机“受信任的根证书颁发机构”和“受信任的发布者”（UMDF 驱动这样就能加载，不用开测试模式），用 EWDK 的 devcon 建根设备并装驱动，删掉驱动库里旧版本的包。

`run-test.ps1` 建一次性计划任务（当前控制台用户、`/it /rl highest`），在控制台会话里跑测试工具，结束后删任务并打印结果。测试会扩展桌面几秒，最后删屏并核对显示配置恢复原样。只看驱动状态可以直接在 SSH 里跑 `LemonGalaxyDisplayTest.exe --info --out info.txt`。

## 卸载

一条命令（不需要 EWDK）：

```bat
powershell -ExecutionPolicy Bypass -File D:\dev\lemon-galaxy-driver\tools\install.ps1 -Remove -RemoveSigningCert
```

它依次做下面几步，也可以手动执行：

```bat
rem 1. 删设备（实例 ID 用 pnputil /enum-devices /class Display 查“LemonGalaxy Display”那一项，开发机上是 ROOT\DISPLAY\0002）
pnputil /remove-device ROOT\DISPLAY\0002
rem 2. 删驱动包（oemNN.inf 用 pnputil /enum-drivers 查原始名 lemongalaxydisplay.inf 的那一项）
pnputil /delete-driver oemNN.inf /uninstall
rem 3. 删测试证书（受信任的根、受信任的发布者；-RemoveSigningCert 还删 My 里的签名私钥）
certutil -delstore Root "LemonGalaxy Test Driver Signing"
certutil -delstore TrustedPublisher "LemonGalaxy Test Driver Signing"
certutil -delstore My "LemonGalaxy Test Driver Signing"
rem 4. 可选：删日志目录
rmdir /s /q C:\ProgramData\LemonGalaxy\Logs
```

只按自己的硬件 ID、INF 名和证书主题操作，不会动 LemonLink Display 或 GameViewer。

## 许可

与仓库相同，MIT。驱动的结构参考了微软 Windows-driver-samples 的 IndirectDisplay 示例（MS-PL），代码由 LemonLink 作者重写后移植过来。

## XP 虚拟屏驱动（`xp/`）

XP 没有 IddCx，用 XP 显示驱动模型（XPDM）自己写的最简帧缓冲驱动，32 位：

| 文件 | 说明 |
|---|---|
| `lgxpmp.c` → `lgxpmp.sys` | 小端口，只导入 videoprt.sys。没有硬件：报模式表，按显示驱动的要求从系统分页池分配 / 释放帧缓冲；主视图之外用 DualView 再建 2 个副视图 |
| `lgxpdd.c` → `lgxpdd.dll` | 显示驱动，只导入 win32k.sys。表面是 GDI 管理的 32 位位图，不挂任何绘图函数；光标当硬件光标接下来但不画（远端由被控端叠加） |
| `lgxp.h` | 两边共用的自定义控制码、注册表值名、上限 |
| `LemonGalaxyDisplayXP.inf` | 根枚举设备 `Root\LemonGalaxyDisplayXP`，类 Display（只能是 ASCII：XP 按 ANSI 代码页读 INF） |
| `build.sh` | 用 llvm-mingw 编（Mac / Linux / Windows 都行），`pe-checksum.py` 写 PE 校验和并核对子系统版本 5.01 |
| `compat/` | mingw-w64 的 winddi.h 要的两个 DirectDraw 头文件的占位 |

```sh
lemon-galaxy-worker-windows/driver/xp/build.sh            # 输出到 driver/build/xp
```

- XP 兼容版编译时设 `LEMON_GALAXY_XP_DRIVER_DIR` 指到输出目录，三个文件就嵌进 exe（`lemon-galaxy-worker-windows/build.rs`，CI 的「XP 虚拟屏驱动」任务先编好再交给 XP 版）。
- 安装：XP 上 `install-service` 时顺带装，或单独 `lemon-galaxy-worker install-display-driver`（管理员）。文件放到 `%ALLUSERSPROFILE%\LemonGalaxy\driver-xp\`，建根设备后 `UpdateDriverForPlugAndPlayDevices`；驱动没有微软签名，XP 会提示「没有通过 Windows 徽标测试」，要点「仍然继续」。不用重启。卸载：`uninstall-display-driver`。
- 装好后系统里多出几块不接入桌面的显示设备「LemonGalaxy Virtual Display」。加屏：被控端把要的分辨率写进它的显示设备键（`LemonGalaxy.Width` / `LemonGalaxy.Height`，小端口每次报模式表时读，排在第一个），再 `ChangeDisplaySettingsEx` 接入、放在现有桌面右边；删屏：断开。接入状态写在注册表里，代理启动时先断开上次留下的。
- 2026-11-26 在 Ubuntu 上 VMware 的 XP SP3 虚拟机里实测：装驱动约 1 秒、不用重启；接入 1470x956（非标准分辨率）约 0.16 秒，挪过去的窗口、桌面背景正常画出，GDI 截屏正常；断开正常。DualView 的主视图（`\Device\Video5`）在注册表里有，但系统没把它列成显示设备，所以目前是 2 块。
