import Foundation
import Security

/// docs/ffi.md 1.1 的 SecureStore：macOS 钥匙串。
/// 服务名 cn.lemonit.galaxy，账户名为内核给的 key；只在本机、首次解锁后可读（不随备份迁移到别的设备）。
final class KeychainStore: SecureStore {
    static let service = "cn.lemonit.galaxy"
    private let lock = NSLock()

    private func baseQuery(_ key: String) -> [CFString: Any] {
        [kSecClass: kSecClassGenericPassword,
         kSecAttrService: Self.service,
         kSecAttrAccount: key,
         kSecUseDataProtectionKeychain: true]
    }

    func get(key: String) -> Data? {
        lock.lock(); defer { lock.unlock() }
        var q = baseQuery(key)
        q[kSecReturnData] = true
        q[kSecMatchLimit] = kSecMatchLimitOne
        var out: CFTypeRef?
        var status = SecItemCopyMatching(q as CFDictionary, &out)
        if status == errSecMissingEntitlement || status == errSecNotAvailable {
            // 临时签名的开发包没有钥匙串访问组，退回传统（文件型）钥匙串
            q[kSecUseDataProtectionKeychain] = nil
            status = SecItemCopyMatching(q as CFDictionary, &out)
        }
        return status == errSecSuccess ? out as? Data : nil
    }

    func set(key: String, value: Data) {
        lock.lock(); defer { lock.unlock() }
        for useDP in [true, false] {
            var q = baseQuery(key)
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
            if status == errSecSuccess { return }
            if status != errSecMissingEntitlement && status != errSecNotAvailable {
                NSLog("LemonGalaxy: 钥匙串写入失败 %d", status)
                return
            }
        }
    }

    func remove(key: String) {
        lock.lock(); defer { lock.unlock() }
        var q = baseQuery(key)
        SecItemDelete(q as CFDictionary)
        q[kSecUseDataProtectionKeychain] = nil
        SecItemDelete(q as CFDictionary)
    }
}

/// 只在内存里的安全存储：selftest 与模拟内核的演示模式用，不碰钥匙串。
final class MemorySecureStore: SecureStore {
    private var items: [String: Data] = [:]
    private let lock = NSLock()
    func get(key: String) -> Data? { lock.lock(); defer { lock.unlock() }; return items[key] }
    func set(key: String, value: Data) { lock.lock(); items[key] = value; lock.unlock() }
    func remove(key: String) { lock.lock(); items[key] = nil; lock.unlock() }
}
