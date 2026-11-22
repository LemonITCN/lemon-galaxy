import AppKit

// 入口：--selftest <输出目录> [--scale 2] 用模拟数据离屏渲染全部主要界面为 PNG 后退出（不弹窗、不申请权限、不连网络）；
// 否则作为普通应用运行（主窗口 + 菜单栏图标）。
let args = CommandLine.arguments
if args.contains("--cu-selftest") {
    exit(ComputerUseSelfTest.run(args))
}
if args.contains("--cu-exec") {
    exit(MainActor.assumeIsolated { ComputerUseExec.run(args) })
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
