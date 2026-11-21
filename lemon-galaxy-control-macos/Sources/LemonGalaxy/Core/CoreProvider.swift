import Foundation

/// 界面拿内核实现的唯一入口。界面只认 GalaxyClient 等 Bridge 类型，不直接碰 MockCore 的内部。
///
/// 选择规则：
///   - 启动参数 `--mock` 或环境变量 `LEMONGALAXY_MOCK=1`：模拟内核；
///   - 启动参数 `--real` 或 `LEMONGALAXY_MOCK=0`：真内核（编译时接入了 LemonGalaxyCore.xcframework 才有）；
///   - 都没指定：`defaultUsesMock`（真内核接入并稳定后改成 false）。
enum CoreProvider {
    static let defaultUsesMock = true

    struct Context {
        let client: GalaxyClient
        let isMock: Bool
        let coreVersion: String
    }

    static var realCoreAvailable: Bool {
        #if LEMONGALAXY_REAL_CORE
        return true
        #else
        return false
        #endif
    }

    static func wantsMock(arguments: [String] = CommandLine.arguments,
                          environment: [String: String] = ProcessInfo.processInfo.environment) -> Bool {
        if arguments.contains("--mock") { return true }
        if arguments.contains("--real") { return !realCoreAvailable }
        if let env = environment["LEMONGALAXY_MOCK"] {
            if env == "1" || env.lowercased() == "true" { return true }
            if env == "0" || env.lowercased() == "false" { return !realCoreAvailable }
        }
        return defaultUsesMock || !realCoreAvailable
    }

    static func make(config: CoreConfig, store: SecureStore, listener: ClientListener, forceMock: Bool = false,
                     mockOptions: MockOptions = MockOptions()) -> Context {
        if forceMock || wantsMock() {
            let client = MockGalaxyClient(store: store, listener: listener, locale: config.locale, options: mockOptions)
            return Context(client: client, isMock: true, coreVersion: MockGalaxyClient.version)
        }
        #if LEMONGALAXY_REAL_CORE
        coreInit(config: config)
        return Context(client: GalaxyClient(store: store, listener: listener), isMock: false, coreVersion: coreVersion())
        #else
        let client = MockGalaxyClient(store: store, listener: listener, locale: config.locale, options: mockOptions)
        return Context(client: client, isMock: true, coreVersion: MockGalaxyClient.version)
        #endif
    }
}

/// 模拟内核的可调项（selftest 用：去掉延时、不起后台定时器）。
struct MockOptions {
    /// 模拟网络延时倍率，0 表示立即返回
    var latencyScale: Double = 1
    /// 是否在后台模拟设备上下线、路径切换等变化
    var liveSimulation: Bool = true
}
