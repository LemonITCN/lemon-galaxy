import AppKit

// 入口：
//   --core-smoke <服务器地址> <用户名> <密码> [--hold 秒]   不开窗口，用真内核验证登录、上线、权限、协助、退出
//   --selftest <输出目录> [--scale 2] [--lang zh-Hans|en] [--theme light|dark] [--only 名称片段]
//                                                          用模拟内核把主要界面离屏渲染成 PNG（不弹窗、不申请权限、不连网络）
//   --make-icon <目录>                                     把应用标志渲染成 AppIcon.iconset（再用 iconutil 生成 .icns）
//   --mock                                                 用模拟内核运行（演示界面，不连服务器）
// 否则作为菜单栏应用运行。
let args = CommandLine.arguments
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
