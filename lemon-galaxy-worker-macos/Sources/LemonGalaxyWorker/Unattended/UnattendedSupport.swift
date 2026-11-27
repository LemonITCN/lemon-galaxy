import Foundation
import SystemConfiguration
import Security

/// 无人值守的固定名字与位置（docs/spec.md 3.0.1）。
enum Unattended {
    static let daemonLabel = "cn.lemonit.galaxy.worker.daemon"
    static let agentLabel = "cn.lemonit.galaxy.worker.agent"
    /// SMAppService.daemon 用的文件名（包内 Contents/Library/LaunchDaemons/）
    static let daemonPlistName = daemonLabel + ".plist"
    /// 会话代理的模板（包内 Contents/Library/LaunchAgents/），后台服务把它换成绝对路径后写到 /Library/LaunchAgents/
    static let agentPlistName = agentLabel + ".plist"
    static let globalAgentPlist = "/Library/LaunchAgents/" + agentPlistName
    /// 后台服务的套接字。/var/run 只有 root 能写，路径本身可信；重启后自动清空
    static let defaultSocket = "/var/run/cn.lemonit.galaxy.worker.sock"
    /// 后台服务的数据根目录：内核在其下建 worker/（账号、事件）与 logs/
    static let daemonDataRoot = URL(fileURLWithPath: "/Library/Application Support/LemonGalaxy", isDirectory: true)
    static let daemonCacheDir = URL(fileURLWithPath: "/Library/Caches/cn.lemonit.galaxy.worker", isDirectory: true)
    static let launchctl = "/bin/launchctl"

    static var socketPath: String {
        if let s = ProcessInfo.processInfo.environment["LEMONGALAXY_SOCKET"], !s.isEmpty { return s }
        return defaultSocket
    }

    static func arg(_ args: [String], _ name: String) -> String? {
        guard let i = args.firstIndex(of: name), i + 1 < args.count else { return nil }
        return args[i + 1]
    }

    /// 当前控制台用户（登录界面或没人时为空）
    static func consoleUser() -> (name: String, uid: uid_t)? {
        var uid: uid_t = 0
        guard let name = SCDynamicStoreCopyConsoleUser(nil, &uid, nil) as String?, !name.isEmpty, name != "loginwindow", uid != 0 else { return nil }
        return (name, uid)
    }

    /// 所有已登录图形会话的用户（含快速切换后留在后台的会话）：utmpx 里终端为 console 的记录
    static func guiUsers() -> [uid_t] {
        var out = Set<uid_t>()
        setutxent()
        while let e = getutxent() {
            guard e.pointee.ut_type == Int16(USER_PROCESS) else { continue }
            let line = withUnsafeBytes(of: e.pointee.ut_line) { String(decoding: $0.prefix(while: { $0 != 0 }), as: UTF8.self) }
            let user = withUnsafeBytes(of: e.pointee.ut_user) { String(decoding: $0.prefix(while: { $0 != 0 }), as: UTF8.self) }
            guard line == "console", let pw = getpwnam(user) else { continue }
            out.insert(pw.pointee.pw_uid)
        }
        endutxent()
        if let c = consoleUser() { out.insert(c.uid) }
        return out.sorted()
    }

    /// 另一个进程的启动参数（sysctl KERN_PROCARGS2；只能读同一用户的进程）
    static func arguments(of pid: pid_t) -> [String]? { AppUpdateProcess.arguments(of: pid) }

    /// 是不是本应用的会话代理 / 后台服务 / 更新助手进程（界面的「只跑一份」检查要跳过它们）
    static func isHelperProcess(_ pid: pid_t) -> Bool {
        guard let a = arguments(of: pid) else { return false }
        return a.contains("--session-agent") || a.contains("--daemon") || a.contains("--update-helper") || a.contains("--update-probe")
    }
}

// MARK: - 文件型安全存储

/// 后台服务的设备身份存文件：目录 0700、文件 0600，只有 root 能读（与 Linux 被控端的 FileStore 相同）。
/// 系统钥匙串对 root 也可用，但会话代理与界面不需要读设备密钥，文件更简单、也便于整体移交。
final class FileSecureStore: SecureStore, @unchecked Sendable {
    let dir: URL
    private let lock = NSLock()

    init(dir: URL) {
        self.dir = dir
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        chmod(dir.path, 0o700)
    }

    private func path(_ key: String) -> URL {
        let safe = String(key.map { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" || $0 == ".") ? $0 : "_" })
        return dir.appendingPathComponent(safe)
    }

    func get(key: String) -> Data? { lock.locked { try? Data(contentsOf: path(key)) } }

    func set(key: String, value: Data) {
        lock.locked {
            let p = path(key)
            let tmp = p.appendingPathExtension("tmp")
            FileManager.default.createFile(atPath: tmp.path, contents: value, attributes: [.posixPermissions: 0o600])
            _ = rename(tmp.path, p.path)
        }
    }

    func remove(key: String) { lock.locked { _ = unlink(path(key).path) } }
}

// MARK: - 移交（菜单栏应用的数据 ↔ 后台服务的数据）

enum Migration {
    static let maxBytes = 64 << 20

    /// 数据目录里要搬的文件（跳过 keys/ 与临时文件），相对路径
    static func collect(dir: URL) -> [MigrationBundle.File] {
        let fm = FileManager.default
        // 按路径枚举直接得到相对路径（/var 与 /private/var 这类符号链接不影响）
        guard let e = fm.enumerator(atPath: dir.path) else { return [] }
        var out: [MigrationBundle.File] = []
        var total = 0
        while let rel = e.nextObject() as? String {
            if rel == "keys" || rel.hasPrefix("keys/") { e.skipDescendants(); continue }
            guard e.fileAttributes?[.type] as? FileAttributeType == .typeRegular, !rel.hasSuffix(".tmp"),
                  let data = fm.contents(atPath: dir.appendingPathComponent(rel).path) else { continue }
            total += data.count
            if total > maxBytes { break }
            out.append(.init(path: rel, data: data))
        }
        return out.sorted { $0.path < $1.path }
    }

    /// 相对路径必须留在目录里（不许 ..、绝对路径）
    static func safeRelative(_ p: String) -> Bool {
        guard !p.isEmpty, !p.hasPrefix("/"), p.utf8.count < 512 else { return false }
        return !p.split(separator: "/").contains { $0 == ".." || $0 == "." || $0.isEmpty }
    }

    /// 写到目录（目录事先应为空或不存在）；文件 0600
    static func write(_ files: [MigrationBundle.File], to dir: URL) throws {
        let fm = FileManager.default
        try fm.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        for f in files {
            guard safeRelative(f.path) else { throw IPCFault.other("不合法的路径：\(f.path)") }
            let url = dir.appendingPathComponent(f.path)
            try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            guard fm.createFile(atPath: url.path, contents: f.data, attributes: [.posixPermissions: 0o600]) else {
                throw IPCFault.other("无法写入 \(url.path)")
            }
        }
    }

    /// 菜单栏应用（当前用户）的数据：~/Library/Application Support/LemonGalaxyWorker/worker + 本机安全存储里的设备身份（LocalSecureStore）
    static var localWorkerDir: URL { AppInfo.dataDir.appendingPathComponent("worker", isDirectory: true) }

    static func localBundle() -> MigrationBundle? {
        let dir = localWorkerDir
        guard FileManager.default.fileExists(atPath: dir.appendingPathComponent("host.json").path) else { return nil }
        let store = PrefixedSecureStore(base: LocalSecureStore(dir: AppInfo.secretsDir), prefix: "worker.")
        guard let id = store.get(key: "device_id"), let key = store.get(key: "device_key") else { return nil }
        return MigrationBundle(deviceId: id, deviceKey: key, files: collect(dir: dir), sourceUser: NSUserName())
    }

    static func deviceIdHex(_ d: Data) -> String { d.map { String(format: "%02x", $0) }.joined() }

    /// 移交成功后把本地数据改名留作备份（不删，关闭无人值守且不是管理员时还能拿回来）
    static func markLocalMigrated() {
        let dir = localWorkerDir
        guard FileManager.default.fileExists(atPath: dir.path) else { return }
        let backup = AppInfo.dataDir.appendingPathComponent("worker.migrated", isDirectory: true)
        try? FileManager.default.removeItem(at: backup)
        try? FileManager.default.moveItem(at: dir, to: backup)
    }

    /// 关闭无人值守：把后台服务导出的数据写回当前用户
    static func restoreLocal(_ b: MigrationBundle) throws {
        let fm = FileManager.default
        let dir = localWorkerDir
        if fm.fileExists(atPath: dir.path) {
            let stamp = Int(Date().timeIntervalSince1970)
            try fm.moveItem(at: dir, to: AppInfo.dataDir.appendingPathComponent("worker.backup-\(stamp)", isDirectory: true))
        }
        try write(b.files, to: dir)
        let store = PrefixedSecureStore(base: LocalSecureStore(dir: AppInfo.secretsDir), prefix: "worker.")
        store.set(key: "device_id", value: b.deviceId)
        store.set(key: "device_key", value: b.deviceKey)
        try? fm.removeItem(at: AppInfo.dataDir.appendingPathComponent("worker.migrated", isDirectory: true))
    }

    /// 不是管理员、拿不回后台服务的数据时：用移交前留下的备份
    static func restoreMigratedBackup() -> Bool {
        let fm = FileManager.default
        let backup = AppInfo.dataDir.appendingPathComponent("worker.migrated", isDirectory: true)
        guard fm.fileExists(atPath: backup.path), !fm.fileExists(atPath: localWorkerDir.path) else { return false }
        return (try? fm.moveItem(at: backup, to: localWorkerDir)) != nil
    }
}

// MARK: - 会话代理的 launchd 配置

enum AgentPlist {
    /// 全局会话代理：登录界面（root）与每个已登录用户的图形会话各跑一份。
    /// SMAppService.agent 只能登记到当前用户、也不支持 LoginWindow（Apple 开发者论坛 thread 726864、737038），
    /// 所以由后台服务（root）写到 /Library/LaunchAgents，程序路径指向包内主程序（同一签名，系统授权对它生效）。
    static func make(executable: String, bundleId: String) -> [String: Any] {
        [
            "Label": Unattended.agentLabel,
            "ProgramArguments": [executable, "--session-agent"],
            "LimitLoadToSessionType": ["LoginWindow", "Aqua"],
            "RunAtLoad": true,
            // 正常退出（关闭了无人值守、同一会话已有代理）不再拉起；出错退出隔几秒重启
            "KeepAlive": ["SuccessfulExit": false],
            "ThrottleInterval": 5,
            "ProcessType": "Interactive",
            "AssociatedBundleIdentifiers": [bundleId],
        ]
    }

    static func data(_ dict: [String: Any]) -> Data? {
        try? PropertyListSerialization.data(fromPropertyList: dict, format: .xml, options: 0)
    }

    /// 与现有文件相同就不动；返回是否改写了
    @discardableResult
    static func install(executable: String, bundleId: String, path: String = Unattended.globalAgentPlist) -> Bool {
        guard let d = data(make(executable: executable, bundleId: bundleId)) else { return false }
        if let cur = FileManager.default.contents(atPath: path), cur == d { return false }
        let tmp = path + ".tmp"
        guard FileManager.default.createFile(atPath: tmp, contents: d, attributes: [.posixPermissions: 0o644]) else {
            NSLog("LemonGalaxy: 无法写入 %@", tmp)
            return false
        }
        chown(tmp, 0, 0)
        guard rename(tmp, path) == 0 else { unlink(tmp); return false }
        return true
    }

    /// 让已登录用户的会话立刻用上（新开的登录界面与之后登录的用户由 launchd 自动加载）
    static func reloadInGuiSessions(path: String = Unattended.globalAgentPlist) {
        for uid in Unattended.guiUsers() {
            _ = run([Unattended.launchctl, "bootout", "gui/\(uid)/\(Unattended.agentLabel)"])
            let r = run([Unattended.launchctl, "bootstrap", "gui/\(uid)", path])
            NSLog("LemonGalaxy: 在用户 %d 的会话里加载会话代理：%@", uid, r == 0 ? "成功" : "退出码 \(r)")
        }
    }

    static func uninstall(path: String = Unattended.globalAgentPlist) {
        for uid in Unattended.guiUsers() {
            _ = run([Unattended.launchctl, "bootout", "gui/\(uid)/\(Unattended.agentLabel)"])
        }
        unlink(path)
    }

    @discardableResult
    static func run(_ argv: [String]) -> Int32 {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: argv[0])
        p.arguments = Array(argv.dropFirst())
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        do { try p.run() } catch { return -1 }
        p.waitUntilExit()
        return p.terminationStatus
    }
}
