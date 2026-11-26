# Linux 图形被控（契约）

2026-11-24 拍板。推翻立项时「Linux 只做终端 + 文件」：Linux 被控端也能远程桌面，并有图形界面。
与 [spec.md](spec.md) 冲突时以本文件为准（spec.md 里 Linux 相关的行已改）。

## 1. 组成

同一个工程 `lemon-galaxy-worker-linux`，出两个程序：

| 程序 | 作用 | 依赖 |
|---|---|---|
| `lemon-galaxy-worker` | 内核：命令行、系统服务、桌面会话里的画面代理、虚拟桌面、显卡编码 | 只链接系统 C 库（glibc 2.17 起，CentOS 7 起都能跑）；X11、D-Bus 是纯 Rust 实现；NVENC、VA-API、PipeWire、PulseAudio 运行时按需加载，没有就跳过 |
| `lemon-galaxy-worker-gui` | 窗口 + 托盘：内核服务的「遥控器」，经本机控制接口（第 7 节）通信 | GTK 3（Ubuntu 20.04、Debian 10、CentOS/RHEL 8 起自带） |

发布（叫法与各端一致：带窗口与托盘的叫桌面端，命令行单文件叫 CLI）：

- 桌面端 `.deb` / `.rpm`：两个程序 + systemd 系统服务（装好即启用）+ 菜单图标 + 登录后自动显示托盘 + polkit 授权规则。依赖由包管理器装。
- CLI `lemon-galaxy-worker-<版本>-linux-<架构>`：单文件，只有内核，给服务器用（没有桌面也能跑终端、文件；装了桌面环境的服务器能开虚拟桌面）。
- CLI 精简版 `…-linux-musl-<架构>`：静态单文件（Alpine 等 musl 系统），只有终端与文件，不含画面（编译时关掉 `desktop` 特性）。

## 2. 进程结构

```
systemd: lemon-galaxy-worker run（root）
  ├─ 联网、账号、终端、文件、命令执行（与以前相同）
  ├─ 本机控制接口 /run/lemon-galaxy-worker/control.sock（第 7 节）
  ├─ 画面代理 lemon-galaxy-worker __lg-session-agent（每次只有一个）
  │    以图形会话的用户身份运行（登录界面时以 root 运行），带上该会话的 DISPLAY / XAUTHORITY /
  │    WAYLAND_DISPLAY / XDG_RUNTIME_DIR / DBUS_SESSION_BUS_ADDRESS；
  │    负责截屏编码、键鼠、剪贴板、声音；经标准输入输出与服务通信（长度前缀 + postcard）
  └─ 虚拟桌面（没有可看的图形会话时按需启动，第 5 节）
```

普通用户运行（用户级服务或前台 `run`）时结构相同，只是画面代理就是本用户、只看本用户的会话。

## 3. 找图形会话

每 2 秒看一次，选「现在屏幕上是谁」：

1. logind：读 `/run/systemd/sessions/*`（键值文件：`ACTIVE`、`SEAT`、`TYPE`=x11/wayland/tty、`CLASS`=user/greeter、`UID`、`USER`、`DISPLAY`、`LEADER`）。取 `seat0` 上 `ACTIVE=1` 且 `TYPE` 是 x11 / wayland 的会话。
2. 会话的环境变量从会话进程里取（`/proc/<pid>/environ`，优先 `gnome-shell`、`kwin_*`、`Xorg`/`Xwayland` 的客户端、`LEADER` 的子进程）。
3. X11 登录界面（GDM、LightDM、SDDM）：从 Xorg 进程的命令行取 `-auth <文件>` 与显示号，代理以 root 运行。
4. 没有 logind（容器、非 systemd 发行版）：扫 `/proc` 找 Xorg / Xvfb / Xwayland 进程和它们的 `-auth`。
5. 都没有：没有可看的会话，按第 5 节处理。

会话变了（换人登录、注销、从登录界面进桌面）就重启画面代理，正在进行的画面自动续上（与 Windows 被控端相同）。

## 4. 画面后端

画面代理启动时按会话类型选：

| 会话 | 截屏 | 键鼠 | 剪贴板 | 光标 |
|---|---|---|---|---|
| X11（含登录界面、虚拟桌面） | MIT-SHM 截根窗口（每块屏按 RandR 显示器切），Damage 扩展判断有没有变化，没有变化不出帧 | XTEST | X11 选择区（CLIPBOARD：文本、PNG 图片、文件列表 text/uri-list） | XFixes：取光标图像自己画进画面，或只报位置 |
| GNOME Wayland | Mutter 的 `org.gnome.Mutter.ScreenCast`（每块屏 `RecordMonitor`）+ PipeWire 收帧（只要共享内存缓冲，不要 dmabuf） | `org.gnome.Mutter.RemoteDesktop` | RemoteDesktop 的 SelectionRead / SelectionWrite（文本） | 光标模式「嵌入画面」或「元数据」 |
| 其他 Wayland（KDE 等） | xdg-desktop-portal `ScreenCast` + `RemoteDesktop`，`persist_mode=2` 记住授权；第一次必须有人在被控机上点「允许」，恢复令牌存在该用户的 `~/.config/lemon-galaxy/portal-restore-token` | 门户 `NotifyPointer*` / `NotifyKeyboardKeycode` | 暂不支持 | 嵌在画面里 |

Mutter 的这两个接口在用户自己的会话总线上，不弹授权，无人值守可用（gnome-remote-desktop 也用它们）。

**锁屏**：GNOME 等 Wayland 桌面锁屏时，系统禁止一切远程桌面与录屏（Mutter 拒绝建会话，已有的会话被关），远程看不到锁屏界面、也没法输密码。处理：

- 远程看画面期间，画面代理向会话申请「禁止闲置」（GNOME 用 `org.gnome.SessionManager.Inhibit`，其他桌面用 `org.freedesktop.ScreenSaver.Inhibit`），不会因为闲置自动锁屏；本机手动锁屏照样生效。
- 已经锁屏时，主控打开画面会收到明确原因「被控机屏幕已锁定……」。
- 被控端设置「远程连入时自动解锁」（`unlock-on-connect on|off`，界面「设置 → 安全」，默认关）：打开后，有控制权限的会话打开屏幕列表时被控端先 `loginctl unlock-session`，所有画面都停了 10 秒后 `loginctl lock-session` 重新锁上（只锁自己解开的）。只建议在无人值守、放在安全地方的电脑上打开。
- X11 不受影响：锁屏界面本身就能看到和操作。

Wayland 的登录界面看不到：GDM 默认用 Wayland。命令 `lemon-galaxy-worker login-screen x11 on`（界面里也有开关）改 GDM 配置 `WaylandEnable=false`，让登录界面（以及之后登录的桌面）用 X11。不改的话，登录界面时远程看到的是虚拟桌面（第 5 节）。

键盘：协议里是 USB HID 用法码，统一先换成 Linux evdev 键码（`hidmap.rs`）：Mutter 与门户直接用 evdev 键码；X11 用「evdev 键码 + 8」，键盘布局里没有的字符（输入文字）临时改一个空闲键码的映射再按。

指针：Wayland（Mutter、门户）每次移动先发相对移动（与上次位置的差）再发绝对位置定准。只发绝对坐标时，程序锁定或限定指针
（VMware 点进虚拟机后抓住鼠标、浏览器游戏的指针锁定）收不到任何移动：Mutter 把虚拟设备的绝对坐标夹在窗口里且不产生相对移动事件，
VMware 里的指针不动、走不到边缘就放不了手，只能按 Ctrl+Alt。平时相对移动已经到位，绝对位置不再挪；换屏后第一次不发相对移动。

输入法提交的文字（`InputEvent::Text`，主控端用本机中文、日文等输入法打出的字）：
- X11：同上，布局里没有的字符临时映射空闲键码。
- Wayland（Mutter、门户）：Mutter 的 `NotifyKeyboardKeysym` 只在当前布局组里找键码，找不到就静默丢弃（日志 `No keycode found for keyval …`，调用照样返回成功），所以中日韩文字、é 等打不出来；GNOME 自己的远程桌面也有同样限制。
  做法（GNOME 论坛上做 Chrome 远程桌面 Linux 主机的开发者给的方向，2026-11-25 在 Ubuntu 26.04 / GNOME 50.1 / IBus 1.5.34 上实测输入框与 Ptyxis 终端都通过）：
  会话代理运行时加载 libibus，把自己注册成 IBus 引擎 `lemongalaxy-remote`，提交文字时临时切成全局引擎、`commit_text`、空闲 300 毫秒后切回原引擎（切回后核对，不对就重试；原引擎不可用时依次用 GNOME 输入源设置里当前的、`xkb:us::eng`）。不碰剪贴板，终端也能用。
  IBus 守护进程在注册过组件的进程退出时会清空全局引擎，所以另有看门进程（`__lg-ibus-guard`）在代理退出后盯 3 秒，空了就设回原引擎。
  拿不到 IBus（没有库或守护进程，比如 KDE 用 fcitx5）时退回 keysym，并记日志。
  只有键盘布局里没有的字符才走 IBus：一段文字里的 ASCII 可见字符、空格、换行、Tab 照样按按键发（`ibus::text_runs`），和真键盘一样。VMware / VirtualBox 等虚拟机窗口把按键原样转给客户机、不认 IBus 提交的文字，全走 IBus 时虚拟机里打不出字（0.1.0.61 曾这样）。

## 5. 虚拟桌面

主控打开屏幕列表时（内核在阻塞线程里调 `HostPlatform::prepare_desktop(系统用户)`），如果没有可看的图形会话（没接显示器、没人登录且登录界面是 Wayland、远程登录的服务器），就给**这个账号绑定的系统用户**开一个虚拟桌面：

- 分辨率用 `headless-display`（默认 1920x1080，`off` 关闭）。
- 有 `Xvfb` 时：`Xvfb :N`（N 从 90 起找空闲，`-nolisten tcp`，独立 Xauthority），再以该用户身份起桌面会话：从 `/usr/share/xsessions/*.desktop` 里按 Xfce、MATE、Cinnamon、LXQt、LXDE、KDE Plasma（X11）、GNOME（Xorg）的顺序选第一个装了的（`dbus-run-session` 包着）。
- 没有 Xvfb（或没有 X11 桌面会话，比如 GNOME 49 起的 Ubuntu 25.10 / 26.04）但有 GNOME：`dbus-run-session` 里起 PipeWire、WirePlumber，再 `gnome-shell --headless --virtual-monitor 宽x高`（系统自带，不用装任何东西），按 GNOME Wayland 后端接。2026-11-24 在 Ubuntu 26.04（GNOME 50）真机实测：没登录的用户开出虚拟桌面，画面、NVENC 编码、声音都正常。
- 都没有：报错「没有可用的虚拟桌面：请安装 xvfb 和一个桌面环境（如 `sudo apt install xvfb xfce4`）」。
- 断开后保留，下次连接接着用；同一个系统用户只有一个虚拟桌面；该用户在真屏幕上登录后，虚拟桌面关掉（同一用户两个桌面会互相干扰）。
- 屏幕列表里虚拟桌面的 `is_virtual` 为真，名字「虚拟桌面」。

root 账号默认不开虚拟桌面（服务带环境变量 `LEMON_GALAXY_ALLOW_ROOT_DESKTOP=1` 才开），避免远程拿到 root 图形桌面。

命令：`lemon-galaxy-worker desktop` 看屏幕上的会话与虚拟桌面，`desktop start <用户> [宽x高]` 先开好，`desktop stop <用户>` 关掉；`selftest-capture --user <用户>` 在某个用户的虚拟桌面里做画面自检。

以 root 运行、按别的用户身份起画面代理时，程序经 `/proc/self/fd/N`（root 打开的描述符）执行：程序放在那个用户进不去的目录（比如 0750 的家目录）里也能起来，只要求程序文件本身 0755。

## 6. 编码

H.264（Annex-B，关键帧带 SPS/PPS，不带 AUD，与其他被控端相同）。按顺序试，运行时探测，失败就下一个：

1. **NVIDIA NVENC**：加载驱动自带的 `libnvidia-encode.so.1` + `libcuda.so.1`；CUDA 设备内存输入（BGRA 直接给，显卡自己转色彩空间），超低延迟、CBR、无 B 帧、按需 IDR，码率运行时改。结构体与 Windows 被控端同一版 SDK（12.0）。
2. **VA-API**（Intel、AMD，以及 NVIDIA 以外装了 VA 驱动的显卡）：`libva.so.2` + `libva-drm.so.2`，打开 `/dev/dri/renderD*`；CPU 转 NV12 后上传，驱动自己写 SPS/PPS。
3. **openh264** 软件编码兜底。

NVENC、VA-API 按配置帧率分码率，画面帧率低时由内核 `galaxy-host` 的 `pacing.rs` 放大交给编码器的码率、VBV 按时间算；openh264 用时间戳码控；画面停下后补编几次（见 [protocol.md](protocol.md) 8.3）。
建好编码器后先试编一帧，确认真能出带 SPS/PPS 的关键帧才用；硬件编码连续出错 3 次，这路画面改用软件编码。环境变量 `LEMON_GALAXY_VIDEO_ENCODER=auto|nvenc|vaapi|software` 可指定。`lemon-galaxy-worker selftest-capture` 打印找到的会话、屏幕、各编码器能不能用与耗时。

## 7. 本机控制接口（图形界面用）

Windows 被控端用同一套消息，走命名管道，鉴权按管道那头的令牌，普通用户改配置经 UAC 授权，见 [spec.md](spec.md) 3.0.2。

服务监听 Unix 套接字：root 服务 `/run/lemon-galaxy-worker/control.sock`（目录 0755、套接字 0666，靠对端凭据鉴权），用户级服务 `$XDG_RUNTIME_DIR/lemon-galaxy-worker/control.sock`（0600，没有 `XDG_RUNTIME_DIR` 时放数据目录）。环境变量 `LEMON_GALAXY_CONTROL_SOCKET` 可改位置（服务和界面都认）。旧套接字还连得上说明另一个实例在用，新实例不开控制接口。

服务没登录任何账号时照常运行、只开控制接口，界面或命令行登录后再连服务器；账号全部退出后回到这个状态，进程不退。

消息：一行一个 JSON，请求带 `op` 字段，回复与事件带 `type` 字段。类型定义在 `galaxy-worker/src/control.rs`（界面用 `#[path]` 引入同一个文件）。登录是多步的（两步验证码、首次设新密码、更换服务器确认、选系统用户、root 二次确认），服务每一步告诉界面下一步。`status` 里的 `config_rev`（host.json 修改时间）变了，界面就重读账号与设置，命令行改的配置也能同步到界面。

鉴权（按对端 `SO_PEERCRED`）：

| 操作 | 谁能做 |
|---|---|
| 看状态（在线、谁在控制本机） | 任何本机用户 |
| 断开远程、停止 AI、禁止远程、开始 / 结束临时协助 | root；或当前屏幕前的图形会话用户（重新允许远程要管理员授权；协助的设备码与验证码只给开协助的连接和 root） |
| 看账号、设置、操作日志 | root；账号绑定的系统用户；管理员组（sudo / wheel / admin） |
| 改设置、登录 / 退出账号、改权限、开机自启、更新 | root；或通过 polkit 动作 `cn.lemonit.galaxy.worker.manage`（`auth_admin_keep`，输一次管理员密码记住几分钟） |

用户级服务：同一个用户全部允许。

## 8. 安装包与更新

- `.deb` / `.rpm`：`/usr/bin/lemon-galaxy-worker`、`/usr/bin/lemon-galaxy-worker-gui`、`/usr/lib/systemd/system/lemon-galaxy-worker.service`（装好即启用，数据 `/var/lib/lemon-galaxy-worker`）、`/usr/share/applications/cn.lemonit.galaxy.worker.desktop`、`/etc/xdg/autostart/lemon-galaxy-worker-gui.desktop`（`--tray`）、`/usr/share/polkit-1/actions/cn.lemonit.galaxy.worker.policy`、hicolor 图标。依赖 GTK 3 与 librsvg（托盘的 SVG 图标）。包内单元带 `LEMON_GALAXY_SUPERVISED=1`。
- 桌面端（包管理器装的）在命令行运行 `install-service` / `uninstall-service` 只启用 / 停用包自带的服务，不再复制程序。
- 自动更新：被控服务按发布签名下载新的 CLI（内核单文件）替换 `/usr/bin/lemon-galaxy-worker`（界面程序不随之更新，下次装新包时一起换；控制接口是 JSON，新旧版本互相兼容）。musl 精简版的平台名是 `linux-musl`，只会更新到精简版。
- 发布文件：`lemon-galaxy-worker_<版本>_amd64.deb` / `_arm64.deb`、`lemon-galaxy-worker-<版本>.x86_64.rpm` / `.aarch64.rpm`（桌面端）、`lemon-galaxy-worker-<版本>-linux-<架构>`（CLI，glibc 2.17 起）、`lemon-galaxy-worker-<版本>-linux-musl-<架构>`（CLI 精简版）。界面在 Debian 10 容器里编（glibc 2.28），内核用 cargo-zigbuild 按 glibc 2.17 编。

## 9. 图形界面

GTK 3，跟随系统主题与语言（中文 / 英文）。界面里能做命令行能做的全部事情：

- 概览：设备名、设备 ID、服务状态（运行中 / 已连上服务器 / 公网 IP）、版本；一键允许 / 禁止远程；正在控制本机的会话（谁、从哪台设备、直连还是中转、有没有 AI），每个都能断开，一键停止所有 AI。
- 账号：列表（用户名、显示名、状态、系统用户）；登录（服务器地址、用户名、密码、两步验证码、首次登录设新密码、选系统用户、root 二次确认）；退出；权限（系统用户、看画面、控制、文件、终端、剪贴板、声音、被控提示、AI 上限）。
- 临时协助：开始 / 结束，显示设备码与验证码，可设固定验证码；请求来了弹「允许 / 拒绝」。
- 设置：开机自动启动（系统服务）、登录后显示托盘图标、远程结束后自动锁屏、自动更新 + 检查更新 / 立即更新、虚拟桌面（开关与分辨率）、登录界面使用 X11（GDM）。
- 日志：本机远程操作记录。
- 托盘（StatusNotifierItem，KDE、Ubuntu 自带支持；GNOME 原版要装 AppIndicator 扩展，没有托盘时关窗口就最小化）：状态图标（空闲 / 被控中）、打开主界面、允许远程开关、临时协助、断开所有远程、退出托盘。
- 有人连入时右上角通知；被控期间托盘图标变色。
