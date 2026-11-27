import Foundation
import Security
import Darwin

// 自动安装更新（Mac 主控端与 Mac 被控端共用：被控端经 Sources/Shared/ 的符号链接引用同一份，docs/spec.md「Mac 自动更新」）。
//
// 用户点「立即更新」后一路自动做完，不弹任何要点的东西（没有密码框、确认框、访达窗口）：
//   1. 内核下载好的 DMG（已验发布签名与 SHA-256）用 hdiutil 只读挂载到临时目录（-nobrowse -noautoopen，不出现在访达里）；
//   2. 找到包里的 .app，用本进程自己的指定要求（designated requirement，跨版本不变：标识 + 证书）严格核对它的签名
//      （所有架构、嵌套代码、严格模式），核对包标识、主程序、版本号必须比当前新；
//   3. ditto 到目标旁边（同一个卷、同一个目录）的隐藏位置，去掉隔离标记，再核对一次，卸载 DMG；
//   4. renamex_np(RENAME_SWAP) 原子地对换新旧两个包：目标路径上是新版本，隐藏位置上是旧版本（留作回滚），
//      全程目标路径上都有一个完整可用的应用；新包是新文件（新 inode），不会撞上「原地改写已签名代码」的问题；
//   5. 用旧版本的程序起一个脱离本进程的助手（--update-helper，新会话、新进程组，本进程或 launchd 任务退出时不受牵连），
//      本进程退出；
//   6. 助手等旧进程退出，打开新版本（后台服务模式由 launchd 重新拉起），等新版本报告「启动成功」：
//      报告了就删掉旧包；超时就结束新版本、把旧包换回来、重新打开旧版本，记下这个版本（以后不再自动下载）。
// 任何一步失败都退回原状：没换之前失败什么都不动；换了之后起不来由助手换回。

// MARK: - 版本号

/// 点分数字版本号（0.1.0.76）。各段按数字比较，缺的段按 0（0.1.0 == 0.1.0.0）
struct AppVersion: Comparable, CustomStringConvertible {
    let parts: [Int]
    let description: String

    init?(_ s: String) {
        let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return nil }
        var p: [Int] = []
        for c in t.split(separator: ".", omittingEmptySubsequences: false) {
            guard !c.isEmpty, c.allSatisfy(\.isASCII), let n = Int(c), n >= 0 else { return nil }
            p.append(n)
        }
        parts = p
        description = t
    }

    private static func padded(_ a: AppVersion, _ b: AppVersion) -> ([Int], [Int]) {
        let n = max(a.parts.count, b.parts.count)
        return (a.parts + Array(repeating: 0, count: n - a.parts.count), b.parts + Array(repeating: 0, count: n - b.parts.count))
    }

    static func < (a: AppVersion, b: AppVersion) -> Bool {
        let (x, y) = padded(a, b)
        return x.lexicographicallyPrecedes(y)
    }

    static func == (a: AppVersion, b: AppVersion) -> Bool {
        let (x, y) = padded(a, b)
        return x == y
    }

    /// a 比 b 新（任一方不是合法版本号时为假）
    static func newer(_ a: String, than b: String) -> Bool {
        guard let x = AppVersion(a), let y = AppVersion(b) else { return false }
        return x > y
    }
}

// MARK: - 错误与不能自动安装的原因

struct AppUpdateError: Error, CustomStringConvertible, Equatable {
    enum Kind: String {
        case noFile, mount, noApp, badBundle, notNewer, signature, copy, swap, spawn, busy, blocked
    }
    let kind: Kind
    let detail: String
    var description: String { detail }

    static func make(_ kind: Kind, _ arg: String = "", _ arg2: String = "") -> AppUpdateError {
        AppUpdateError(kind: kind, detail: L("install.err." + kind.rawValue, arg, arg2))
    }
}

/// 应用所在位置不能自动替换的原因（界面上说明，不弹窗）
enum AppUpdateBlocker: Equatable {
    case notBundled
    case devBuild
    case translocated
    case readOnly
    case notWritable(String)

    var message: String {
        switch self {
        case .notBundled: return L("install.blocked.notBundled")
        case .devBuild: return L("install.blocked.devBuild")
        case .translocated: return L("install.blocked.translocated")
        case .readOnly: return L("install.blocked.readOnly")
        case .notWritable(let p): return L("install.blocked.notWritable", p)
        }
    }
}

// MARK: - 包信息

struct AppBundleInfo: Equatable {
    let identifier: String
    let version: String
    let build: String
    let executable: String

    /// 直接读磁盘上的 Info.plist（不经 Bundle 的缓存：包可能刚被换掉）
    static func read(_ app: URL) -> AppBundleInfo? {
        let plist = app.appendingPathComponent("Contents/Info.plist")
        guard let d = NSDictionary(contentsOf: plist) as? [String: Any],
              let id = d["CFBundleIdentifier"] as? String,
              let exe = d["CFBundleExecutable"] as? String, !exe.isEmpty, !exe.contains("/") else { return nil }
        return AppBundleInfo(identifier: id, version: d["CFBundleShortVersionString"] as? String ?? "",
                             build: d["CFBundleVersion"] as? String ?? "", executable: exe)
    }

    func executableURL(in app: URL) -> URL { app.appendingPathComponent("Contents/MacOS").appendingPathComponent(executable) }
}

// MARK: - 代码签名

enum AppUpdateCodeSign {
    /// 本进程的静态代码与指定要求。启动时就取（包被换掉之后再取，指向的就不是正在运行的这份了）
    static let own: (code: SecStaticCode?, requirement: SecRequirement?) = {
        var code: SecCode?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code else { return (nil, nil) }
        var st: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &st) == errSecSuccess, let st else { return (nil, nil) }
        var req: SecRequirement?
        _ = SecCodeCopyDesignatedRequirement(st, [], &req)
        return (st, req)
    }()

    static var ownRequirement: SecRequirement? { own.requirement }

    /// 本进程是用证书签的（不是临时签名、也不是没签名的开发构建）
    static var ownIsCertSigned: Bool {
        guard let c = own.code else { return false }
        return isCertSigned(c)
    }

    static func isCertSigned(_ code: SecStaticCode) -> Bool {
        var info: CFDictionary?
        guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue: UInt32(kSecCSSigningInformation)), &info) == errSecSuccess,
              let d = info as? [String: Any] else { return false }
        let flags = (d[kSecCodeInfoFlags as String] as? NSNumber)?.uint32Value ?? 0
        let certs = d[kSecCodeInfoCertificates as String] as? [Any] ?? []
        return flags & 0x2 == 0 && !certs.isEmpty   // 0x2 = kSecCodeSignatureAdhoc
    }

    static func staticCode(_ url: URL) -> SecStaticCode? {
        var sc: SecStaticCode?
        guard SecStaticCodeCreateWithPath(url as CFURL, [], &sc) == errSecSuccess else { return nil }
        return sc
    }

    /// 磁盘上某个包的指定要求（自检用：拿发布包当「当前应用」）
    static func designatedRequirement(of url: URL) -> SecRequirement? {
        guard let sc = staticCode(url) else { return nil }
        var req: SecRequirement?
        guard SecCodeCopyDesignatedRequirement(sc, [], &req) == errSecSuccess else { return nil }
        return req
    }

    static func text(_ r: SecRequirement?) -> String {
        guard let r else { return "" }
        var s: CFString?
        return SecRequirementCopyString(r, [], &s) == errSecSuccess ? (s as String? ?? "") : ""
    }

    /// 严格核对：所有架构、嵌套代码、严格模式，并且满足给定的要求（当前应用的指定要求）
    static func verify(_ url: URL, requirement: SecRequirement) throws {
        guard let sc = staticCode(url) else { throw AppUpdateError.make(.signature, L("install.err.noSignature")) }
        let flags = SecCSFlags(rawValue: UInt32(kSecCSCheckAllArchitectures) | UInt32(kSecCSStrictValidate) | UInt32(kSecCSCheckNestedCode))
        var err: Unmanaged<CFError>?
        let st = SecStaticCodeCheckValidityWithErrors(sc, flags, requirement, &err)
        guard st == errSecSuccess else {
            let msg = err?.takeRetainedValue().localizedDescription ?? (SecCopyErrorMessageString(st, nil) as String? ?? "OSStatus \(st)")
            throw AppUpdateError.make(.signature, msg)
        }
    }
}

// MARK: - 子进程

enum AppUpdateProcess {
    final class Output: @unchecked Sendable { var data = Data() }

    /// 运行命令并收集输出；超时结束它
    @discardableResult
    static func run(_ exe: String, _ args: [String], timeout: Double = 120) -> (status: Int32, output: String) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: exe)
        p.arguments = args
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = pipe
        p.standardInput = FileHandle.nullDevice
        do { try p.run() } catch { return (-1, error.localizedDescription) }
        let out = Output()
        let done = DispatchSemaphore(value: 0)
        DispatchQueue.global().async {
            out.data = pipe.fileHandleForReading.readDataToEndOfFile()
            done.signal()
        }
        if done.wait(timeout: .now() + timeout) == .timedOut {
            p.terminate()
            _ = done.wait(timeout: .now() + 5)
            return (-2, "timeout after \(Int(timeout))s")
        }
        p.waitUntilExit()
        return (p.terminationStatus, String(decoding: out.data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines))
    }

    /// 起一个脱离本进程的子进程：新会话、新进程组（本进程、本进程所在的 launchd 任务结束时不会连带结束它），
    /// 信号处理恢复默认（后台服务忽略了 SIGTERM / SIGPIPE，不能遗传给它），标准输入接 /dev/null，输出追加到日志文件
    static func spawnDetached(_ exe: String, _ args: [String], log: String?) throws -> pid_t {
        var attr: posix_spawnattr_t?
        posix_spawnattr_init(&attr)
        defer { posix_spawnattr_destroy(&attr) }
        posix_spawnattr_setflags(&attr, Int16(POSIX_SPAWN_SETSID | POSIX_SPAWN_CLOEXEC_DEFAULT | POSIX_SPAWN_SETSIGDEF | POSIX_SPAWN_SETSIGMASK))
        var all = sigset_t()
        sigfillset(&all)
        posix_spawnattr_setsigdefault(&attr, &all)
        var none = sigset_t()
        sigemptyset(&none)
        posix_spawnattr_setsigmask(&attr, &none)

        var fa: posix_spawn_file_actions_t?
        posix_spawn_file_actions_init(&fa)
        defer { posix_spawn_file_actions_destroy(&fa) }
        posix_spawn_file_actions_addopen(&fa, 0, "/dev/null", O_RDONLY, 0)
        posix_spawn_file_actions_addopen(&fa, 1, log ?? "/dev/null", O_WRONLY | O_CREAT | O_APPEND, 0o600)
        posix_spawn_file_actions_adddup2(&fa, 1, 2)

        let argv: [UnsafeMutablePointer<CChar>?] = ([exe] + args).map { strdup($0) } + [nil]
        defer { for a in argv { free(a) } }
        var pid: pid_t = 0
        let r = posix_spawn(&pid, exe, &fa, &attr, argv, environ)
        guard r == 0 else { throw AppUpdateError.make(.spawn, String(cString: strerror(r))) }
        return pid
    }

    static func alive(_ pid: pid_t) -> Bool {
        guard pid > 0 else { return false }
        return kill(pid, 0) == 0 || errno == EPERM
    }

    /// 等进程退出；超时先发 SIGTERM，再等一会儿还在就 SIGKILL。返回是否已退出
    @discardableResult
    static func waitExit(_ pid: pid_t, timeout: Double, kill force: Bool = true) -> Bool {
        func wait(_ t: Double) -> Bool {
            let end = Date().addingTimeInterval(t)
            while Date() < end {
                if !alive(pid) { return true }
                // 是自己的子进程时顺便回收，免得变成僵尸一直「活着」
                var st: Int32 = 0
                _ = waitpid(pid, &st, WNOHANG)
                usleep(100_000)
            }
            return !alive(pid)
        }
        if wait(timeout) { return true }
        guard force else { return false }
        Darwin.kill(pid, SIGTERM)
        if wait(10) { return true }
        Darwin.kill(pid, SIGKILL)
        return wait(5)
    }

    /// 解开符号链接后的真实路径（/var → /private/var，比较进程路径时要用）
    static func realPath(_ path: String) -> String {
        guard let r = realpath(path, nil) else { return (path as NSString).standardizingPath }
        defer { free(r) }
        return String(cString: r)
    }

    /// 可执行文件在某个包里的所有进程
    static func pids(runningFrom bundle: String) -> [pid_t] {
        let prefix = realPath(bundle) + "/"
        let n = proc_listallpids(nil, 0)
        guard n > 0 else { return [] }
        var buf = [pid_t](repeating: 0, count: Int(n) + 64)
        let got = buf.withUnsafeMutableBytes { proc_listallpids($0.baseAddress, Int32($0.count)) }
        var out: [pid_t] = []
        var path = [CChar](repeating: 0, count: 4 * Int(MAXPATHLEN))
        for pid in buf.prefix(Int(max(0, got))) where pid > 0 && pid != getpid() {
            guard proc_pidpath(pid, &path, UInt32(path.count)) > 0 else { continue }
            if String(cString: path).hasPrefix(prefix) { out.append(pid) }
        }
        return out
    }

    /// 另一个进程的启动参数（sysctl KERN_PROCARGS2；普通用户只能读自己的进程）
    static func arguments(of pid: pid_t) -> [String]? {
        var mib: [Int32] = [CTL_KERN, KERN_PROCARGS2, pid]
        var size = 0
        guard sysctl(&mib, 3, nil, &size, nil, 0) == 0, size > 4 else { return nil }
        var buf = [UInt8](repeating: 0, count: size)
        guard sysctl(&mib, 3, &buf, &size, nil, 0) == 0, size > 4 else { return nil }
        let argc = Int(buf.withUnsafeBytes { $0.loadUnaligned(as: Int32.self) })
        var i = 4
        // 跳过可执行文件路径与后面的填充 0
        while i < size && buf[i] != 0 { i += 1 }
        while i < size && buf[i] == 0 { i += 1 }
        var args: [String] = []
        while i < size && args.count < argc {
            let start = i
            while i < size && buf[i] != 0 { i += 1 }
            args.append(String(decoding: buf[start..<i], as: UTF8.self))
            i += 1
        }
        return args
    }
}

// MARK: - 状态目录（计划、启动成功标记、结果、不再自动下载的版本）

struct AppUpdatePlan: Codable, Equatable {
    var id: String
    /// app：普通应用，助手打开新版本；daemon：无人值守后台服务，launchd 重新拉起
    var mode: String
    /// open：/usr/bin/open -n 打开；none：不打开（launchd 负责）；probe / probe-fail：自检用，直接运行 --update-probe
    var launch: String
    var target: String
    var backup: String
    var oldPid: Int32
    var fromVersion: String
    var fromBuild: String
    var toVersion: String
    var toBuild: String
    /// 等「启动成功」的秒数
    var timeout: Double
    /// 后台服务模式：launchd 的标签（新版本迟迟不起来、换回旧版本后叫 launchd 立刻启动它）
    var launchdLabel: String?
    var createdAt: Double
}

struct AppUpdateMarker: Codable, Equatable {
    var plan: String
    var version: String
    var build: String
    var pid: Int32
    var path: String
    var at: Double
}

struct AppUpdateResult: Codable, Equatable {
    /// ok：新版本已正常启动；rolledBack：新版本没起来，已换回旧版本；rollbackFailed：换回也失败（日志里有原因）
    var outcome: String
    var plan: String
    var fromVersion: String
    var toVersion: String
    var reason: String
    var at: Double
    var seen: Bool
}

struct AppUpdateState {
    let dir: URL

    init(dir: URL) {
        self.dir = dir
    }

    var pendingURL: URL { dir.appendingPathComponent("pending.json") }
    var markerURL: URL { dir.appendingPathComponent("launched.json") }
    var resultURL: URL { dir.appendingPathComponent("result.json") }
    var failedURL: URL { dir.appendingPathComponent("failed-versions.json") }
    var helperPidURL: URL { dir.appendingPathComponent("helper.pid") }
    var logURL: URL { dir.appendingPathComponent("update.log") }
    var probeLogURL: URL { dir.appendingPathComponent("probe.log") }

    /// 只有本用户（后台服务是 root）能读写
    func ensure() {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        chmod(dir.path, 0o700)
    }

    func read<T: Decodable>(_ type: T.Type, _ url: URL) -> T? {
        guard let d = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(T.self, from: d)
    }

    /// 先写临时文件再改名（读的一方不会读到半个文件）
    func write<T: Encodable>(_ value: T, _ url: URL) throws {
        ensure()
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data = try enc.encode(value)
        let tmp = url.appendingPathExtension("tmp-\(getpid())")
        guard FileManager.default.createFile(atPath: tmp.path, contents: data, attributes: [.posixPermissions: 0o600]) else {
            throw AppUpdateError.make(.copy, tmp.path)
        }
        guard rename(tmp.path, url.path) == 0 else {
            let e = String(cString: strerror(errno))
            unlink(tmp.path)
            throw AppUpdateError.make(.copy, e)
        }
    }

    func remove(_ url: URL) { unlink(url.path) }

    var pending: AppUpdatePlan? { read(AppUpdatePlan.self, pendingURL) }
    var marker: AppUpdateMarker? { read(AppUpdateMarker.self, markerURL) }
    var result: AppUpdateResult? { read(AppUpdateResult.self, resultURL) }
    var failedVersions: [String] { read([String].self, failedURL) ?? [] }

    func addFailed(_ v: String) {
        var list = failedVersions.filter { $0 != v }
        list.append(v)
        try? write(Array(list.suffix(20)), failedURL)
    }

    /// 助手还在跑（同一时间只能有一次更新）
    var helperAlive: Bool {
        guard let s = try? String(contentsOf: helperPidURL, encoding: .utf8), let pid = Int32(s.trimmingCharacters(in: .whitespacesAndNewlines)) else { return false }
        return AppUpdateProcess.alive(pid)
    }

    /// 没看过的结果（看过后标记，下次启动不再提示）
    func takeResult() -> AppUpdateResult? {
        guard var r = result, !r.seen else { return nil }
        r.seen = true
        try? write(r, resultURL)
        return r
    }

    func log(_ text: String) {
        ensure()
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm:ss"
        let line = "\(f.string(from: Date())) [\(getpid())] \(text)\n"
        if let h = FileHandle(forWritingAtPath: logURL.path) {
            h.seekToEndOfFile()
            h.write(Data(line.utf8))
            try? h.close()
        } else {
            FileManager.default.createFile(atPath: logURL.path, contents: Data(line.utf8), attributes: [.posixPermissions: 0o600])
        }
        // 日志过大时截掉前一半
        if let size = (try? FileManager.default.attributesOfItem(atPath: logURL.path))?[.size] as? Int, size > 512 * 1024,
           let d = try? Data(contentsOf: logURL) {
            try? d.suffix(256 * 1024).write(to: logURL)
        }
    }
}

// MARK: - 安装

enum AppUpdate {
    enum Step: Equatable { case verifying, replacing, restarting }

    struct Request {
        var dmg: URL
        var target: URL
        var bundleId: String
        var currentVersion: String
        var currentBuild: String
        /// 新应用必须满足的要求（当前应用的指定要求）；只有自检里合成的假包才传空
        var requirement: SecRequirement?
        var state: AppUpdateState
        var mode = "app"
        var launch = "open"
        var timeout: Double = 90
        var oldPid: pid_t = getpid()
        var launchdLabel: String?
        /// 挂载点放在哪里（后台服务放在只有 root 能写的目录）
        var workDir: URL = FileManager.default.temporaryDirectory
    }

    static let hiddenUpdatePrefix = "-update-"

    /// 隐藏的临时位置：/Applications/.LemonGalaxy-update-1a2b3c4d.app
    static func stagingURL(for target: URL) -> URL {
        let name = target.deletingPathExtension().lastPathComponent
        let id = UUID().uuidString.prefix(8).lowercased()
        return target.deletingLastPathComponent().appendingPathComponent(".\(name)\(hiddenUpdatePrefix)\(id).app", isDirectory: true)
    }

    // MARK: 能不能自动替换

    /// 当前应用所在位置能否由本进程替换（不可以时界面说明原因，「立即更新」不可用）
    static func blocker(for bundle: URL = Bundle.main.bundleURL, requireCertSigned: Bool = true) -> AppUpdateBlocker? {
        guard bundle.pathExtension == "app", AppBundleInfo.read(bundle) != nil else { return .notBundled }
        if requireCertSigned && !AppUpdateCodeSign.ownIsCertSigned { return .devBuild }
        if bundle.path.contains("/AppTranslocation/") { return .translocated }
        if (try? bundle.resourceValues(forKeys: [.volumeIsReadOnlyKey]))?.volumeIsReadOnly == true { return .readOnly }
        let parent = bundle.deletingLastPathComponent().path
        // 换包要改父目录；删旧包要能改包里的目录
        if access(parent, W_OK) != 0 || access(bundle.appendingPathComponent("Contents").path, W_OK) != 0 {
            return .notWritable(bundle.path)
        }
        return nil
    }

    // MARK: DMG

    static func attach(_ dmg: URL, at mount: URL) throws {
        guard FileManager.default.fileExists(atPath: dmg.path) else { throw AppUpdateError.make(.noFile) }
        try? FileManager.default.createDirectory(at: mount, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        // -nobrowse：不出现在访达与桌面；-noautoopen：不自动打开窗口；-readonly：只读
        let args = ["attach", dmg.path, "-nobrowse", "-readonly", "-noautoopen", "-mountpoint", mount.path]
        var r = AppUpdateProcess.run("/usr/bin/hdiutil", args, timeout: 300)
        if r.status != 0 {
            // 偶尔报「资源忙」：等一下再试一次
            sleep(2)
            r = AppUpdateProcess.run("/usr/bin/hdiutil", args, timeout: 300)
        }
        guard r.status == 0 else {
            rmdir(mount.path)
            throw AppUpdateError.make(.mount, r.output.isEmpty ? "hdiutil \(r.status)" : r.output)
        }
    }

    static func detach(_ mount: URL) {
        if AppUpdateProcess.run("/usr/bin/hdiutil", ["detach", mount.path, "-quiet"], timeout: 60).status != 0 {
            AppUpdateProcess.run("/usr/bin/hdiutil", ["detach", mount.path, "-force", "-quiet"], timeout: 60)
        }
        // 只删空目录（卸载失败时挂载点里是只读卷的内容，不能递归删）
        rmdir(mount.path)
    }

    static func findApp(in mount: URL, bundleId: String) -> URL? {
        let items = (try? FileManager.default.contentsOfDirectory(at: mount, includingPropertiesForKeys: [.isSymbolicLinkKey, .isDirectoryKey])) ?? []
        for u in items.sorted(by: { $0.lastPathComponent < $1.lastPathComponent }) where u.pathExtension == "app" {
            let v = try? u.resourceValues(forKeys: [.isSymbolicLinkKey, .isDirectoryKey])
            guard v?.isSymbolicLink != true, v?.isDirectory == true else { continue }
            if AppBundleInfo.read(u)?.identifier == bundleId { return u }
        }
        return nil
    }

    // MARK: 核对

    /// 包完整（Info.plist、主程序）、标识一致、版本比 newerThan 新、签名满足 requirement
    @discardableResult
    static func checkBundle(_ app: URL, bundleId: String, newerThan current: String?, requirement: SecRequirement?) throws -> AppBundleInfo {
        guard let info = AppBundleInfo.read(app) else { throw AppUpdateError.make(.badBundle, "Info.plist") }
        guard info.identifier == bundleId else { throw AppUpdateError.make(.badBundle, info.identifier) }
        let exe = info.executableURL(in: app)
        var st = stat()
        guard lstat(exe.path, &st) == 0, (st.st_mode & S_IFMT) == S_IFREG, access(exe.path, X_OK) == 0 else {
            throw AppUpdateError.make(.badBundle, "Contents/MacOS/" + info.executable)
        }
        if let current {
            guard AppVersion.newer(info.version, than: current) else { throw AppUpdateError.make(.notNewer, info.version, current) }
        }
        if let requirement { try AppUpdateCodeSign.verify(app, requirement: requirement) }
        return info
    }

    /// 挂载、核对、拷到目标旁边、再核对、卸载。返回拷好的新包（隐藏位置）
    static func stage(_ r: Request) throws -> (url: URL, info: AppBundleInfo) {
        let mount = r.workDir.appendingPathComponent("lg-update-mnt-\(UUID().uuidString.prefix(8))", isDirectory: true)
        try attach(r.dmg, at: mount)
        defer { detach(mount) }
        guard let app = findApp(in: mount, bundleId: r.bundleId) else { throw AppUpdateError.make(.noApp, r.target.lastPathComponent) }
        let info = try checkBundle(app, bundleId: r.bundleId, newerThan: r.currentVersion, requirement: r.requirement)
        let staged = stagingURL(for: r.target)
        // --noqtn：不带隔离标记（带了第一次打开会被门禁拦下）；ditto 写的全是新文件（新 inode）
        let c = AppUpdateProcess.run("/usr/bin/ditto", ["--noqtn", app.path, staged.path], timeout: 300)
        guard c.status == 0 else {
            try? FileManager.default.removeItem(at: staged)
            throw AppUpdateError.make(.copy, c.output.isEmpty ? "ditto \(c.status)" : c.output)
        }
        AppUpdateProcess.run("/usr/bin/xattr", ["-d", "-r", "com.apple.quarantine", staged.path], timeout: 60)
        // 后台服务（root）拷出来的属于 root：改成和原来的包一样的属主，关闭无人值守后菜单栏应用照样能自己更新、用户照样能删
        if getuid() == 0, let a = try? FileManager.default.attributesOfItem(atPath: r.target.path),
           let uid = (a[.ownerAccountID] as? NSNumber)?.intValue, let gid = (a[.groupOwnerAccountID] as? NSNumber)?.intValue, uid != 0 {
            AppUpdateProcess.run("/usr/sbin/chown", ["-R", "\(uid):\(gid)", staged.path], timeout: 60)
        }
        do {
            let again = try checkBundle(staged, bundleId: r.bundleId, newerThan: r.currentVersion, requirement: r.requirement)
            guard again == info else { throw AppUpdateError.make(.badBundle, "copy differs") }
        } catch {
            try? FileManager.default.removeItem(at: staged)
            throw error
        }
        return (staged, info)
    }

    // MARK: 对换

    /// 原子地对换两个目录（同一个卷）。卷不支持 RENAME_SWAP 时退回三次改名（中间失败尽量还原）
    static func exchange(_ a: URL, _ b: URL) throws {
        if renamex_np(a.path, b.path, UInt32(RENAME_SWAP)) == 0 { return }
        let e = errno
        guard e == ENOTSUP || e == EINVAL else { throw AppUpdateError.make(.swap, String(cString: strerror(e))) }
        let tmp = b.deletingLastPathComponent().appendingPathComponent(".\(b.lastPathComponent)-swap-\(UUID().uuidString.prefix(8))")
        guard rename(b.path, tmp.path) == 0 else { throw AppUpdateError.make(.swap, String(cString: strerror(errno))) }
        guard rename(a.path, b.path) == 0 else {
            let msg = String(cString: strerror(errno))
            _ = rename(tmp.path, b.path)
            throw AppUpdateError.make(.swap, msg)
        }
        guard rename(tmp.path, a.path) == 0 else {
            let msg = String(cString: strerror(errno))
            _ = rename(b.path, a.path)
            _ = rename(tmp.path, b.path)
            throw AppUpdateError.make(.swap, msg)
        }
    }

    // MARK: 整个流程

    /// 核对、换包、起助手。返回后调用方应尽快退出（应用 NSApp.terminate，后台服务 exit 由 launchd 重启）。
    /// 抛错时目标路径上仍是原来的应用、没有留下临时文件。
    static func install(_ r: Request, step: (Step) -> Void = { _ in }) throws -> AppUpdatePlan {
        let state = r.state
        state.ensure()
        if state.pending != nil && state.helperAlive { throw AppUpdateError.make(.busy) }
        cleanupLeftovers(target: r.target, state: state)
        step(.verifying)
        state.log("开始安装：\(r.dmg.lastPathComponent) → \(r.target.path)（当前 \(r.currentVersion) / \(r.currentBuild)）")
        let staged: (url: URL, info: AppBundleInfo)
        do { staged = try stage(r) } catch {
            state.log("核对失败：\(error)")
            throw error
        }
        step(.replacing)
        let plan = AppUpdatePlan(id: UUID().uuidString, mode: r.mode, launch: r.launch, target: r.target.path, backup: staged.url.path,
                                 oldPid: r.oldPid, fromVersion: r.currentVersion, fromBuild: r.currentBuild,
                                 toVersion: staged.info.version, toBuild: staged.info.build, timeout: r.timeout,
                                 launchdLabel: r.launchdLabel, createdAt: Date().timeIntervalSince1970)
        state.remove(state.markerURL)
        do { try state.write(plan, state.pendingURL) } catch {
            try? FileManager.default.removeItem(at: staged.url)
            throw error
        }
        do { try exchange(staged.url, r.target) } catch {
            state.log("替换失败：\(error)")
            try? FileManager.default.removeItem(at: staged.url)
            state.remove(state.pendingURL)
            throw error
        }
        // 现在目标路径上是新版本，staged 位置上是旧版本（正在运行的这份）
        step(.restarting)
        do {
            try spawnHelper(plan, state: state)
        } catch {
            state.log("起助手失败，换回旧版本：\(error)")
            try? exchange(staged.url, r.target)
            try? FileManager.default.removeItem(at: staged.url)
            state.remove(state.pendingURL)
            throw error
        }
        state.log("已换成 \(plan.toVersion)（\(plan.toBuild)），旧版本暂存在 \(plan.backup)，等助手确认新版本启动")
        return plan
    }

    /// 用旧版本（备份位置上那份，也就是正在运行的这份程序）起助手
    static func spawnHelper(_ plan: AppUpdatePlan, state: AppUpdateState) throws {
        let backup = URL(fileURLWithPath: plan.backup)
        guard let info = AppBundleInfo.read(backup) else { throw AppUpdateError.make(.spawn, plan.backup) }
        let exe = info.executableURL(in: backup).path
        let pid = try AppUpdateProcess.spawnDetached(exe, ["--update-helper", state.dir.path], log: state.logURL.path)
        try? "\(pid)".write(to: state.helperPidURL, atomically: true, encoding: .utf8)
    }

    // MARK: 新版本报告启动成功

    /// 新版本启动并稳定下来后调用（应用：界面起来 5 秒后；后台服务：开始监听 5 秒后）。
    /// 是这次要装的版本就写「启动成功」标记并返回 true（调用方可以提示「已更新到 x」）；助手已经不在（中途关机等）时自己收尾
    @discardableResult
    static func reportLaunched(state: AppUpdateState, bundle: URL = Bundle.main.bundleURL, build: String = AppInfo.build,
                               version: String = AppInfo.version) -> Bool {
        guard let plan = state.pending else { return false }
        let here = AppUpdateProcess.realPath(bundle.path) == AppUpdateProcess.realPath(plan.target)
        let fresh = here && build == plan.toBuild
        if fresh {
            let m = AppUpdateMarker(plan: plan.id, version: version, build: build, pid: getpid(),
                                    path: Bundle.main.executablePath ?? bundle.path, at: Date().timeIntervalSince1970)
            try? state.write(m, state.markerURL)
        }
        // 助手不在了（关机、被结束）：正在目标位置上运行的这份就是结果
        guard here, !state.helperAlive, Date().timeIntervalSince1970 - plan.createdAt > 15 else { return fresh }
        let ok = build == plan.toBuild
        if FileManager.default.fileExists(atPath: plan.backup) && AppUpdateProcess.realPath(plan.backup) != AppUpdateProcess.realPath(bundle.path) {
            try? FileManager.default.removeItem(atPath: plan.backup)
        }
        if !ok { state.addFailed(plan.toVersion) }
        let r = AppUpdateResult(outcome: ok ? "ok" : "rolledBack", plan: plan.id, fromVersion: plan.fromVersion, toVersion: plan.toVersion,
                                reason: ok ? "" : "helper gone", at: Date().timeIntervalSince1970, seen: false)
        try? state.write(r, state.resultURL)
        state.remove(state.pendingURL)
        state.log("助手已不在，由 \(version)（\(build)）收尾：\(r.outcome)")
        return fresh
    }

    /// 清理上次中断留下的隐藏包（有更新正在进行时不动）
    static func cleanupLeftovers(target: URL, state: AppUpdateState) {
        if state.pending != nil && state.helperAlive { return }
        let parent = target.deletingLastPathComponent()
        let prefix = "." + target.deletingPathExtension().lastPathComponent + hiddenUpdatePrefix
        let me = AppUpdateProcess.realPath(Bundle.main.bundlePath)
        for name in (try? FileManager.default.contentsOfDirectory(atPath: parent.path)) ?? [] where name.hasPrefix(prefix) && name.hasSuffix(".app") {
            let p = parent.appendingPathComponent(name)
            // 正在运行的程序所在的包不能删（例如助手本身）
            guard AppUpdateProcess.realPath(p.path) != me, AppUpdateProcess.pids(runningFrom: p.path).isEmpty else { continue }
            try? FileManager.default.removeItem(at: p)
            state.log("清理上次留下的 \(name)")
        }
    }

    /// 缓存目录里不比当前版本新的安装包都删掉
    static func pruneDownloads(_ dir: URL, current: String) {
        let fm = FileManager.default
        for name in (try? fm.contentsOfDirectory(atPath: dir.path)) ?? [] where name.hasSuffix(".dmg") || name.hasSuffix(".part") {
            if let v = version(inFileName: name), AppVersion.newer(v, than: current) { continue }
            try? fm.removeItem(at: dir.appendingPathComponent(name))
        }
    }

    /// LemonGalaxy-0.1.0.76-macOS.dmg → 0.1.0.76
    static func version(inFileName name: String) -> String? {
        guard let r = name.range(of: #"[0-9]+(\.[0-9]+)+"#, options: .regularExpression) else { return nil }
        return String(name[r])
    }

    /// 本应用在磁盘上（可能刚被换掉）的版本号
    static func diskBuild(_ bundle: URL = Bundle.main.bundleURL) -> String? {
        guard bundle.pathExtension == "app", let i = AppBundleInfo.read(bundle), i.identifier == Bundle.main.bundleIdentifier else { return nil }
        return i.build
    }

    /// 本进程退出后重新打开某个包（脱离本进程的 /bin/sh 等本进程退出再 open -n，最多等 30 秒）
    static func relaunchAfterExit(_ bundle: URL = Bundle.main.bundleURL) {
        let script = "i=0; while kill -0 \"$1\" 2>/dev/null && [ $i -lt 150 ]; do sleep 0.2; i=$((i+1)); done; exec /usr/bin/open -n \"$0\""
        _ = try? AppUpdateProcess.spawnDetached("/bin/sh", ["-c", script, bundle.path, "\(getpid())"], log: nil)
    }
}

// MARK: - 助手（--update-helper <状态目录>）

/// 由旧版本程序起的独立进程：等旧进程退出、打开新版本、等它报告启动成功；不成功就换回旧版本。
/// 输出追加到状态目录的 update.log。
enum AppUpdateHelper {
    static func run(_ args: [String]) -> Int32 {
        signal(SIGPIPE, SIG_IGN)
        guard let i = args.firstIndex(of: "--update-helper"), i + 1 < args.count else { return 2 }
        let state = AppUpdateState(dir: URL(fileURLWithPath: args[i + 1], isDirectory: true))
        try? "\(getpid())".write(to: state.helperPidURL, atomically: true, encoding: .utf8)
        guard let plan = state.pending else {
            state.log("助手：没有待完成的更新")
            return 1
        }
        let code = execute(plan, state: state)
        state.remove(state.helperPidURL)
        return code
    }

    static func execute(_ plan: AppUpdatePlan, state: AppUpdateState) -> Int32 {
        state.log("助手：等旧进程 \(plan.oldPid) 退出（\(plan.fromVersion) → \(plan.toVersion)，方式 \(plan.mode)/\(plan.launch)）")
        if !AppUpdateProcess.waitExit(plan.oldPid, timeout: 30) {
            state.log("助手：旧进程 \(plan.oldPid) 结束不了，继续")
        }
        let target = URL(fileURLWithPath: plan.target)
        // 后台服务模式：launchd 会用新程序重启后台服务；这里只在它还没起来时提醒 launchd
        if !launch(plan, state: state, target: target, newVersion: true) {
            return rollback(plan, state: state, reason: "打不开新版本")
        }
        let deadline = Date().addingTimeInterval(plan.timeout)
        var kicked = false
        while Date() < deadline {
            if let m = state.marker, m.plan == plan.id, m.build == plan.toBuild {
                return succeed(plan, state: state, marker: m)
            }
            if plan.mode == "daemon", !kicked, Date() > deadline.addingTimeInterval(-plan.timeout / 2), let label = plan.launchdLabel {
                kicked = true
                let r = AppUpdateProcess.run("/bin/launchctl", ["kickstart", "system/\(label)"], timeout: 20)
                state.log("助手：新后台服务还没报告启动，launchctl kickstart：\(r.status)")
            }
            usleep(250_000)
        }
        return rollback(plan, state: state, reason: "新版本 \(Int(plan.timeout)) 秒内没有报告启动成功")
    }

    /// 打开新版本（或换回后的旧版本）。只有 open 打不开时返回 false
    @discardableResult
    private static func launch(_ plan: AppUpdatePlan, state: AppUpdateState, target: URL, newVersion: Bool) -> Bool {
        switch plan.launch {
        case "open":
            let ls = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
            AppUpdateProcess.run(ls, ["-f", target.path], timeout: 30)
            var r = AppUpdateProcess.run("/usr/bin/open", ["-n", target.path], timeout: 60)
            if r.status != 0 {
                sleep(2)
                r = AppUpdateProcess.run("/usr/bin/open", ["-n", target.path], timeout: 60)
            }
            state.log("助手：打开\(newVersion ? "新" : "旧")版本：\(r.status == 0 ? "成功" : "失败 \(r.status) \(r.output)")")
            return r.status == 0
        case "probe", "probe-fail":
            guard let info = AppBundleInfo.read(target) else { return false }
            var args = ["--update-probe", state.dir.path]
            if plan.launch == "probe-fail" && newVersion { args.append("--fail") }
            if let pid = try? AppUpdateProcess.spawnDetached(info.executableURL(in: target).path, args, log: state.logURL.path) {
                AppUpdateProcess.waitExit(pid, timeout: 20)
            }
            return true
        default:
            return true
        }
    }

    private static func succeed(_ plan: AppUpdatePlan, state: AppUpdateState, marker: AppUpdateMarker) -> Int32 {
        state.log("助手：新版本 \(marker.version)（\(marker.build)，进程 \(marker.pid)）已启动")
        if plan.mode == "daemon" {
            // 会话代理、菜单栏应用还是旧程序时会在重连时被要求重启；旧包多留一会儿，它们的签名核对还能对上
            sleep(15)
        }
        do {
            try FileManager.default.removeItem(atPath: plan.backup)
        } catch {
            state.log("助手：删除旧版本 \(plan.backup) 失败：\(error)（下次更新前会再清理）")
        }
        // 新版本报告启动成功时已经自己提示过「已更新」，结果记成已看过
        let r = AppUpdateResult(outcome: "ok", plan: plan.id, fromVersion: plan.fromVersion, toVersion: plan.toVersion, reason: "",
                                at: Date().timeIntervalSince1970, seen: true)
        try? state.write(r, state.resultURL)
        state.remove(state.pendingURL)
        state.log("助手：更新完成 \(plan.fromVersion) → \(plan.toVersion)")
        return 0
    }

    private static func rollback(_ plan: AppUpdatePlan, state: AppUpdateState, reason: String) -> Int32 {
        state.log("助手：\(reason)，换回 \(plan.fromVersion)")
        NSLog("LemonGalaxy 更新助手：%@，换回 %@", reason, plan.fromVersion)
        var r = AppUpdateResult(outcome: "rolledBack", plan: plan.id, fromVersion: plan.fromVersion, toVersion: plan.toVersion, reason: reason,
                                at: Date().timeIntervalSince1970, seen: false)
        try? state.write(r, state.resultURL)
        state.addFailed(plan.toVersion)
        let target = URL(fileURLWithPath: plan.target), backup = URL(fileURLWithPath: plan.backup)
        // 结束新版本的进程。后台服务模式只结束后台服务与会话代理（launchd 会用换回来的旧程序重启它们）；
        // 菜单栏应用发现磁盘上的版本变了会自己重新打开
        let victims = AppUpdateProcess.pids(runningFrom: target.path).filter { pid in
            guard plan.mode == "daemon" else { return true }
            let a = AppUpdateProcess.arguments(of: pid) ?? []
            return a.contains("--daemon") || a.contains("--session-agent")
        }
        for pid in victims { kill(pid, SIGTERM) }
        for pid in victims where !AppUpdateProcess.waitExit(pid, timeout: 5, kill: false) { kill(pid, SIGKILL) }
        guard FileManager.default.fileExists(atPath: plan.backup), AppBundleInfo.read(backup) != nil else {
            r.outcome = "rollbackFailed"
            r.reason = reason + "；旧版本已不在 \(plan.backup)"
            try? state.write(r, state.resultURL)
            state.remove(state.pendingURL)
            state.log("助手：旧版本不在了，无法换回，保留新版本")
            return 1
        }
        do {
            try AppUpdate.exchange(backup, target)
        } catch {
            r.outcome = "rollbackFailed"
            r.reason = reason + "；换回失败：\(error)"
            try? state.write(r, state.resultURL)
            state.remove(state.pendingURL)
            state.log("助手：换回失败：\(error)")
            return 1
        }
        try? FileManager.default.removeItem(at: backup)
        state.remove(state.pendingURL)
        state.log("助手：已换回 \(plan.fromVersion)，\(plan.toVersion) 以后不再自动下载")
        if plan.mode == "daemon", let label = plan.launchdLabel {
            let k = AppUpdateProcess.run("/bin/launchctl", ["kickstart", "system/\(label)"], timeout: 20)
            state.log("助手：launchctl kickstart 旧后台服务：\(k.status)")
        } else {
            launch(plan, state: state, target: target, newVersion: false)
        }
        return 0
    }
}

/// --update-probe <状态目录> [--fail]：自检用的「新版本」。不加 --fail 时像正常启动的新版本一样报告启动成功
enum AppUpdateProbe {
    static func run(_ args: [String]) -> Int32 {
        guard let i = args.firstIndex(of: "--update-probe"), i + 1 < args.count else { return 2 }
        let state = AppUpdateState(dir: URL(fileURLWithPath: args[i + 1], isDirectory: true))
        let fail = args.contains("--fail")
        let line = "\(AppInfo.build) \(getpid()) \(fail ? "fail" : "ok")\n"
        if let h = FileHandle(forWritingAtPath: state.probeLogURL.path) {
            h.seekToEndOfFile()
            h.write(Data(line.utf8))
            try? h.close()
        } else {
            FileManager.default.createFile(atPath: state.probeLogURL.path, contents: Data(line.utf8))
        }
        if !fail { AppUpdate.reportLaunched(state: state) }
        return 0
    }
}
