import Foundation

/// 模拟内核的可调项（测试用：去掉延时、不起后台定时器）。
public struct MockOptions {
    /// 模拟网络延时倍率，0 表示立即返回
    public var latencyScale: Double
    /// 是否在后台模拟设备上下线、路径切换等变化
    public var liveSimulation: Bool
    public init(latencyScale: Double = 1, liveSimulation: Bool = true) {
        self.latencyScale = latencyScale
        self.liveSimulation = liveSimulation
    }
}

/// 界面拿内核实现的唯一入口。界面只认 GalaxyClient 等内核类型（真绑定或镜像），不直接碰模拟内核内部。
///
/// 选择规则：
///   - 启动参数 `--mock` 或环境变量 `LEMONGALAXY_MOCK=1`：模拟内核；
///   - 其余情况：编译时接入了 Rust 内核（LEMONGALAXY_REAL_CORE）就用真内核，否则只能用模拟内核。
public enum CoreProvider {
    public struct Context {
        public let client: GalaxyClient
        public let isMock: Bool
        public let coreVersion: String
    }

    public static var realCoreAvailable: Bool {
        #if LEMONGALAXY_REAL_CORE
        return true
        #else
        return false
        #endif
    }

    public static func wantsMock(arguments: [String] = CommandLine.arguments,
                                 environment: [String: String] = ProcessInfo.processInfo.environment) -> Bool {
        if !realCoreAvailable { return true }
        if arguments.contains("--mock") { return true }
        if let env = environment["LEMONGALAXY_MOCK"], env == "1" || env.lowercased() == "true" { return true }
        return false
    }

    private static var initialized = false
    private static let initLock = NSLock()

    /// core_init 每个进程只能调一次
    public static func initCoreOnce(_ config: CoreConfig) {
        initLock.lock(); defer { initLock.unlock() }
        guard !initialized else { return }
        initialized = true
        #if LEMONGALAXY_REAL_CORE
        coreInit(config: config)
        #endif
    }

    public static func make(config: CoreConfig, store: SecureStore, listener: ClientListener, forceMock: Bool = false,
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
    public static func setLocale(_ locale: String, isMock: Bool) {
        #if LEMONGALAXY_REAL_CORE
        guard !isMock, initialized else { return }
        coreSetLocale(locale: locale)
        #endif
    }

    /// 内核日志文件（设置页「导出日志」用）；模拟内核没有
    public static func logPath(isMock: Bool) -> String? {
        #if LEMONGALAXY_REAL_CORE
        guard !isMock, initialized else { return nil }
        let p = coreLogPath()
        return p.isEmpty ? nil : p
        #else
        return nil
        #endif
    }
}

/// iOS 主控端的内核配置：数据放 Application Support，缓存放 Caches；平台 iOS，设备类型 mobile（只能主控）。
public enum CoreEnvironment {
    public static var dataDir: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent("LemonGalaxy", isDirectory: true)
    }

    public static var cacheDir: URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("cn.lemonit.galaxy", isDirectory: true)
    }

    /// 下载的文件放在「文稿/Downloads」，在「文件」App 的「我的 iPhone › LemonGalaxy」里能看到
    public static var downloadsDir: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Downloads", isDirectory: true)
    }

    public static func config(dataDir: URL = CoreEnvironment.dataDir, cacheDir: URL = CoreEnvironment.cacheDir,
                              appVersion: String, osVersion: String, hostname: String, locale: String,
                              logLevel: String = "info") -> CoreConfig {
        for d in [dataDir, cacheDir] { try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true) }
        return CoreConfig(dataDir: dataDir.path, cacheDir: cacheDir.path, logLevel: logLevel, platform: .ios,
                          deviceKind: .mobile, appVersion: appVersion, osVersion: osVersion, hostname: hostname, locale: locale)
    }
}
