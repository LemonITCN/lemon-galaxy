# LemonGalaxy 虚拟屏驱动（Windows，IddCx）

第一阶段：只有驱动和独立测试工具，还没接进 worker。调研与方案见 `docs/research/virtual-display.md`。

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
