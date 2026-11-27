import AppKit

// 入口：
//   --core-smoke <服务器地址> <用户名> <密码> [--hold 秒]   不开窗口，用真内核验证登录、上线、权限、协助、退出
//   --selftest <输出目录> [--scale 2] [--lang zh-Hans|en] [--theme light|dark] [--only 名称片段]
//                                                          用模拟内核把主要界面离屏渲染成 PNG（不弹窗、不申请权限、不连网络）
//   --make-icon <目录>                                     把应用标志渲染成 AppIcon.iconset（再用 iconutil 生成 .icns）
//   --mock                                                 用模拟内核运行（演示界面，不连服务器）
//   --daemon [--socket 路径] [--data-dir 目录] [--mock]    无人值守的后台服务（launchd 以 root 启动）
//   --session-agent [--socket 路径]                        无人值守的会话代理（launchd 在登录界面与每个用户会话里启动）
//   --unattended-status [--socket 路径]                    不开窗口，打印后台服务与会话代理的状态（排查用）
//   --unattended-update [--socket 路径]                    不开窗口，让后台服务检查并立即更新（零交互，等它以新版本重新上线）
//   --ipc-selftest [--bundle 应用路径]                     不用 root 检查后台服务、会话代理、界面之间的通信与包内 launchd 配置
//   --paste-selftest                                       粘贴时才传（标记文件、FSEvents、落盘）的自检
//   --update-selftest [--dmg 发布的 DMG --current 旧 .app]  自动安装更新的自检（临时目录里的副本，不碰「应用程序」）
//   --update-helper <状态目录> / --update-probe <状态目录>   自动更新的助手 / 自检用的「新版本」（内部使用）
// 否则作为菜单栏应用运行。
let args = CommandLine.arguments
// 自动更新的助手（旧版本换下后由它等旧进程退出、打开新版本或等 launchd 拉起新后台服务、不成功就换回，
// 见 Shared/AppUpdateInstaller.swift）。放在最前面：它从隐藏位置上的旧包运行，不能碰界面、内核与套接字
if args.contains("--update-helper") {
    exit(AppUpdateHelper.run(args))
}
if args.contains("--update-probe") {
    exit(AppUpdateProbe.run(args))
}
if args.contains("--update-selftest") {
    exit(AppUpdateSelfTest.main(args))
}
// 无人值守（docs/spec.md 3.0.1）：后台服务与会话代理都是本程序换个参数运行，同一签名，系统授权对它们同样生效
if args.contains("--daemon") {
    exit(DaemonMain.run(args))
}
if args.contains("--session-agent") {
    exit(SessionAgentMain.run(args))
}
if args.contains("--unattended-status") {
    exit(UnattendedStatusCLI.run(args))
}
if args.contains("--unattended-update") {
    exit(UnattendedUpdateCLI.run(args))
}
if args.contains("--ipc-selftest") {
    exit(IPCSelfTest.run(args))
}
// 粘贴时才传（标记文件 + FSEvents + 落盘）的自检：私有剪贴板、临时目录，不开访达、不按键
if args.contains("--paste-selftest") {
    setvbuf(stdout, nil, _IOLBF, 0)
    let problems = LazyPasteSelfTest.run()
    print(problems.isEmpty ? "paste-selftest：全部通过" : "paste-selftest：\(problems.count) 项失败")
    exit(problems.isEmpty ? 0 : 1)
}
if args.contains("--core-smoke") {
    exit(CoreSmoke.run(args))
}
if let i = args.firstIndex(of: "--make-icon"), i + 1 < args.count {
    exit(MainActor.assumeIsolated { SelfTest.makeIcon(args[i + 1]) })
}
if let i = args.firstIndex(of: "--selftest") {
    let dir = i + 1 < args.count && !args[i + 1].hasPrefix("--") ? args[i + 1] : "selftest"
    let code = MainActor.assumeIsolated { SelfTest.run(outputDir: dir, args: args) }
    exit(code)
}

let app = NSApplication.shared
let delegate = MainActor.assumeIsolated { AppDelegate() }
app.delegate = delegate
app.run()
