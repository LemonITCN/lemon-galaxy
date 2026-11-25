import AppKit

// 入口：--selftest <输出目录> [--scale 2] 用模拟数据离屏渲染全部主要界面为 PNG 后退出（不弹窗、不申请权限、不连网络）；
// 否则作为普通应用运行（主窗口 + 菜单栏图标）。
let args = CommandLine.arguments
// 外部智能体（Claude Desktop 等只支持 stdio 的 MCP 客户端）以 --mcp-stdio 启动：不建界面，
// 读标准输入、写标准输出，转给正在运行的 LemonGalaxy 的本机 MCP 服务（docs/mcp.md）。标准输出只能有协议内容
if args.contains("--mcp-stdio") {
    exit(mcpStdioBridge(dataDir: AppModel.dataDir.path))
}
if args.contains("--cu-selftest") {
    exit(ComputerUseSelfTest.run(args))
}
if args.contains("--cu-exec") {
    exit(MainActor.assumeIsolated { ComputerUseExec.run(args) })
}
// 粘贴时才传（标记文件 + FSEvents + 落盘）的自检：私有剪贴板、临时目录，不开访达
if args.contains("--paste-selftest") {
    setvbuf(stdout, nil, _IOLBF, 0)
    let problems = LazyPasteSelfTest.run()
    print(problems.isEmpty ? "paste-selftest：全部通过" : "paste-selftest：\(problems.count) 项失败")
    exit(problems.isEmpty ? 0 : 1)
}
if args.contains("--core-smoke") {
    exit(CoreSmoke.run(args))
}
if args.contains("--vd-host") {
    exit(VirtualDisplaySmoke.host(args))
}
if args.contains("--vd-smoke") {
    exit(VirtualDisplaySmoke.controller(args))
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
