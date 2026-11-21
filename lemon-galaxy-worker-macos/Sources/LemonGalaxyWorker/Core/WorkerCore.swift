import Foundation

/// 界面拿内核实现的唯一入口：返回 WorkerHost（真内核或模拟内核），界面只认内核类型。
///
/// 选择规则：
///   - 启动参数 `--mock` 或环境变量 `LEMONGALAXY_MOCK=1`：模拟内核（selftest 也固定用模拟内核）；
///   - 其余情况：编译时接入了 Rust 内核（LEMONGALAXY_REAL_CORE）就用真内核，否则只能用模拟内核。
enum WorkerCore {
    struct Context {
        let worker: WorkerHost
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

    static func config(locale: String, dataDir: URL = AppInfo.dataDir, cacheDir: URL = AppInfo.cacheDir, hostname: String = AppInfo.computerName) -> CoreConfig {
        CoreConfig(dataDir: dataDir.path, cacheDir: cacheDir.path, logLevel: "info", platform: .macos, deviceKind: .worker,
                   appVersion: AppInfo.version, osVersion: AppInfo.osVersion, hostname: hostname, locale: locale)
    }

    static func make(locale: String, forceMock: Bool = false, mockOptions: MockWorkerOptions = MockWorkerOptions()) -> Context {
        if forceMock || wantsMock() {
            return Context(worker: MockWorkerHost(options: mockOptions), isMock: true, coreVersion: MockWorkerHost.version)
        }
        #if LEMONGALAXY_REAL_CORE
        for d in [AppInfo.dataDir, AppInfo.cacheDir] { try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true) }
        initCoreOnce(config(locale: locale))
        let store = PrefixedSecureStore(base: KeychainStore(), prefix: "worker.")
        return Context(worker: WorkerHost(store: store), isMock: false, coreVersion: coreVersion())
        #else
        return Context(worker: MockWorkerHost(options: mockOptions), isMock: true, coreVersion: MockWorkerHost.version)
        #endif
    }

    /// 切换界面语言时同步给内核（内核返回的错误文字跟着变）
    static func setLocale(_ locale: String, isMock: Bool) {
        guard !isMock, initialized else { return }
        coreSetLocale(locale: locale)
    }
}
