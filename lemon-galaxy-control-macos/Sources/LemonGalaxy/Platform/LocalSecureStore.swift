import Foundation
import Security

/// docs/ffi.md 1.1 的 SecureStore 在 macOS 上的实现（主控端与被控端共用这一份）。
///
/// - 临时签名（ad-hoc）或没签名的包：存本机文件 `<数据目录>/secrets/<key>`，目录 0700、文件 0600，不碰钥匙串。
///   临时签名的指定要求是程序的哈希，每个新版本在 macOS 看来都是另一个程序：传统钥匙串里的条目每次更新后
///   读写都要弹「想要使用钥匙串中的机密信息」，一项弹一次；数据保护钥匙串又要开发者团队的权利，临时签名拿不到。
/// - 带开发者团队签名的包（Developer ID、Apple Development）：用钥匙串（服务名 cn.lemonit.galaxy，账户名为 key），
///   这种签名的指定要求跨版本不变，更新后不弹。
///
/// 换存法时搬一次旧数据：文件模式下某项文件里没有、又没搬过，就从钥匙串读一次（旧条目由上一个版本创建，
/// 这一次仍会弹框，之后不再碰钥匙串）；缓存类条目不搬（登录后从服务器重新拉），免得多弹。
/// 钥匙串模式下钥匙串里没有、文件里有，就搬进钥匙串并删掉文件。
final class LocalSecureStore: SecureStore, @unchecked Sendable {
    static let defaultService = "cn.lemonit.galaxy"

    /// 本进程的签名带开发者团队（指定要求跨版本稳定），这时才用钥匙串
    static let hasTeamSignature: Bool = {
        var code: SecCode?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code else { return false }
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else { return false }
        var info: CFDictionary?
        guard SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &info) == errSecSuccess,
              let dict = info as? [String: Any] else { return false }
        return (dict[kSecCodeInfoTeamIdentifier as String] as? String).map { !$0.isEmpty } ?? false
    }()

    private let dir: URL
    private let service: String
    private let useKeychain: Bool
    private let lock = NSLock()
    /// 已经从钥匙串搬过（或确认钥匙串里没有）的 key，记在 secrets/.from-keychain，一行一个
    private var migrated: Set<String> = []

    /// - Parameters:
    ///   - dir: 文件模式的存放目录（应用数据目录下的 secrets）
    ///   - service: 钥匙串服务名（自检用别的名字，免得碰到真数据）
    ///   - useKeychain: 不传就按签名决定
    init(dir: URL, service: String = LocalSecureStore.defaultService, useKeychain: Bool? = nil) {
        self.dir = dir
        self.service = service
        self.useKeychain = useKeychain ?? Self.hasTeamSignature
        if !self.useKeychain {
            if let s = try? String(contentsOf: markerURL, encoding: .utf8) {
                migrated = Set(s.split(separator: "\n").map(String.init))
            }
        }
    }

    func get(key: String) -> Data? {
        lock.lock(); defer { lock.unlock() }
        if useKeychain {
            if let d = keychainGet(key) { return d }
            // 以前是文件模式（临时签名的包）存的：搬进钥匙串
            guard let d = try? Data(contentsOf: fileURL(key)) else { return nil }
            if keychainSet(key, d) { try? FileManager.default.removeItem(at: fileURL(key)) }
            return d
        }
        if let d = try? Data(contentsOf: fileURL(key)) { return d }
        guard !migrated.contains(key) else { return nil }
        // 第一次用到这一项：旧版本可能存在钥匙串里
        let old = Self.isCache(key) ? nil : keychainGet(key)
        if let old { fileWrite(key, old) }
        markMigrated(key)
        return old
    }

    func set(key: String, value: Data) {
        lock.lock(); defer { lock.unlock() }
        if useKeychain {
            _ = keychainSet(key, value)
            return
        }
        fileWrite(key, value)
        // 文件里有了新值，以后不再去钥匙串找旧的
        if !migrated.contains(key) { markMigrated(key) }
    }

    func remove(key: String) {
        lock.lock(); defer { lock.unlock() }
        if useKeychain {
            var q = keychainQuery(key)
            SecItemDelete(q as CFDictionary)
            q[kSecUseDataProtectionKeychain] = nil
            SecItemDelete(q as CFDictionary)
        } else {
            // 只删文件：钥匙串里的旧条目不再读（删它本身也可能弹框），也就不会在退出登录后被捞回来
            if !migrated.contains(key) { markMigrated(key) }
        }
        try? FileManager.default.removeItem(at: fileURL(key))
    }

    // MARK: 文件

    /// 缓存类条目（设备列表、密码库缓存）：丢了从服务器重新拉，不值得为它弹框
    private static func isCache(_ key: String) -> Bool { key.contains("cache") }

    private var markerURL: URL { dir.appendingPathComponent(".from-keychain") }

    private func fileURL(_ key: String) -> URL {
        // 与 Rust FileStore 同样的文件名规则
        let safe = String(key.map { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" || $0 == ".") ? $0 : "_" })
        return dir.appendingPathComponent(safe)
    }

    private func ensureDir() {
        let fm = FileManager.default
        if !fm.fileExists(atPath: dir.path) {
            try? fm.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        }
        try? fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: dir.path)
    }

    /// 先写同目录的临时文件（创建时就是 0600）再改名，写到一半断电也不会留下半个文件
    private func fileWrite(_ key: String, _ value: Data) {
        ensureDir()
        let dest = fileURL(key)
        let tmp = dir.appendingPathComponent(".\(dest.lastPathComponent).\(getpid()).tmp")
        let fd = open(tmp.path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0o600)
        guard fd >= 0 else {
            NSLog("LemonGalaxy: 写入本机安全存储失败（%d）", errno)
            return
        }
        let ok = value.withUnsafeBytes { buf -> Bool in
            var off = 0
            while off < buf.count {
                let n = write(fd, buf.baseAddress! + off, buf.count - off)
                if n <= 0 { return false }
                off += n
            }
            return true
        }
        let synced = fsync(fd) == 0
        close(fd)
        if ok && synced && rename(tmp.path, dest.path) == 0 { return }
        NSLog("LemonGalaxy: 写入本机安全存储失败（%d）", errno)
        unlink(tmp.path)
    }

    private func markMigrated(_ key: String) {
        migrated.insert(key)
        ensureDir()
        let text = migrated.sorted().joined(separator: "\n")
        let fd = open(markerURL.path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0o600)
        guard fd >= 0 else { return }
        _ = text.withCString { write(fd, $0, strlen($0)) }
        close(fd)
    }

    // MARK: 钥匙串

    private func keychainQuery(_ key: String) -> [CFString: Any] {
        [kSecClass: kSecClassGenericPassword,
         kSecAttrService: service,
         kSecAttrAccount: key,
         kSecUseDataProtectionKeychain: true]
    }

    private func keychainGet(_ key: String) -> Data? {
        var q = keychainQuery(key)
        q[kSecReturnData] = true
        q[kSecMatchLimit] = kSecMatchLimitOne
        var out: CFTypeRef?
        var status = SecItemCopyMatching(q as CFDictionary, &out)
        if status != errSecSuccess {
            // 没有钥匙串访问组的包写入时会退回传统（文件型）钥匙串，
            // 而数据保护钥匙串在这种包里查不到时报的是「找不到」而不是「缺权限」，所以只要没查到就再查传统钥匙串
            q[kSecUseDataProtectionKeychain] = nil
            out = nil
            status = SecItemCopyMatching(q as CFDictionary, &out)
        }
        return status == errSecSuccess ? out as? Data : nil
    }

    @discardableResult
    private func keychainSet(_ key: String, _ value: Data) -> Bool {
        for useDP in [true, false] {
            var q = keychainQuery(key)
            if !useDP { q[kSecUseDataProtectionKeychain] = nil }
            let attrs: [CFString: Any] = [kSecValueData: value,
                                          kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
            var status = SecItemUpdate(q as CFDictionary, attrs as CFDictionary)
            if status == errSecItemNotFound {
                var add = q
                add[kSecValueData] = value
                add[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
                add[kSecAttrLabel] = "LemonGalaxy"
                status = SecItemAdd(add as CFDictionary, nil)
            }
            if status == errSecSuccess { return true }
            if status != errSecMissingEntitlement && status != errSecNotAvailable {
                NSLog("LemonGalaxy: 钥匙串写入失败 %d", status)
                return false
            }
        }
        return false
    }
}

/// 只在内存里的安全存储：selftest 与模拟内核的演示模式用，不碰钥匙串。
final class MemorySecureStore: SecureStore, @unchecked Sendable {
    private var items: [String: Data] = [:]
    private let lock = NSLock()
    func get(key: String) -> Data? { lock.lock(); defer { lock.unlock() }; return items[key] }
    func set(key: String, value: Data) { lock.lock(); items[key] = value; lock.unlock() }
    func remove(key: String) { lock.lock(); items[key] = nil; lock.unlock() }
}
