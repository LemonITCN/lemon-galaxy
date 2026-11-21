import Foundation

/// 界面拿内核实现的唯一入口。界面只认 GalaxyClient 等内核类型（真绑定或 BridgeMirror），不直接碰 MockCore 内部。
///
/// 选择规则：
///   - 启动参数 `--mock` 或环境变量 `LEMONGALAXY_MOCK=1`：模拟内核（selftest 也固定用模拟内核）；
///   - 其余情况：编译时接入了 Rust 内核（LEMONGALAXY_REAL_CORE）就用真内核，否则只能用模拟内核。
enum CoreProvider {
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
        if !realCoreAvailable { return true }
        if arguments.contains("--mock") { return true }
        if let env = environment["LEMONGALAXY_MOCK"], env == "1" || env.lowercased() == "true" { return true }
        return false
    }

    private static var initialized = false
    private static let initLock = NSLock()

    /// core_init 每个进程只能调一次
    static func initCoreOnce(_ config: CoreConfig) {
        initLock.lock(); defer { initLock.unlock() }
        guard !initialized else { return }
        initialized = true
        coreInit(config: config)
    }

    static func make(config: CoreConfig, store: SecureStore, listener: ClientListener, forceMock: Bool = false,
                     mockOptions: MockOptions = MockOptions()) -> Context {
        if forceMock || wantsMock() {
            let client = MockGalaxyClient(store: store, listener: listener, locale: config.locale, options: mockOptions)
            return Context(client: client, isMock: true, coreVersion: MockGalaxyClient.version)
        }
        #if LEMONGALAXY_REAL_CORE
        initCoreOnce(config)
        return Context(client: GalaxyClient(store: store, listener: listener), isMock: false, coreVersion: coreVersion())
        #else
        let client = MockGalaxyClient(store: store, listener: listener, locale: config.locale, options: mockOptions)
        return Context(client: client, isMock: true, coreVersion: MockGalaxyClient.version)
        #endif
    }

    /// 切换界面语言时同步给内核（内核返回的错误文字跟着变）
    static func setLocale(_ locale: String, isMock: Bool) {
        guard !isMock, initialized else { return }
        coreSetLocale(locale: locale)
    }
}

/// 模拟内核的可调项（selftest 用：去掉延时、不起后台定时器）。
struct MockOptions {
    /// 模拟网络延时倍率，0 表示立即返回
    var latencyScale: Double = 1
    /// 是否在后台模拟设备上下线、路径切换等变化
    var liveSimulation: Bool = true
}
