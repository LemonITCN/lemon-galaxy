import Foundation
import Security

/// docs/ffi.md 的 SecureStore：Apple 钥匙串。
/// 服务名 cn.lemonit.galaxy，账户名为内核给的 key；只在本机、首次解锁后可读（不随备份迁移到别的设备）。
public final class KeychainStore: SecureStore, @unchecked Sendable {
    public static let service = "cn.lemonit.galaxy"
    private let lock = NSLock()
    private let service: String

    public init(service: String = KeychainStore.service) { self.service = service }

    private func baseQuery(_ key: String) -> [CFString: Any] {
        var q: [CFString: Any] = [kSecClass: kSecClassGenericPassword,
                                  kSecAttrService: service,
                                  kSecAttrAccount: key]
        #if os(macOS)
        q[kSecUseDataProtectionKeychain] = true
        #endif
        return q
    }

    public func get(key: String) -> Data? {
        lock.lock(); defer { lock.unlock() }
        var q = baseQuery(key)
        q[kSecReturnData] = true
        q[kSecMatchLimit] = kSecMatchLimitOne
        var out: CFTypeRef?
        var status = SecItemCopyMatching(q as CFDictionary, &out)
        #if os(macOS)
        if status == errSecMissingEntitlement || status == errSecNotAvailable {
            q[kSecUseDataProtectionKeychain] = nil
            status = SecItemCopyMatching(q as CFDictionary, &out)
        }
        #endif
        return status == errSecSuccess ? out as? Data : nil
    }

    public func set(key: String, value: Data) {
        lock.lock(); defer { lock.unlock() }
        let q = baseQuery(key)
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
        if status != errSecSuccess {
            NSLog("LemonGalaxy: 钥匙串写入失败 %d", status)
        }
    }

    public func remove(key: String) {
        lock.lock(); defer { lock.unlock() }
        SecItemDelete(baseQuery(key) as CFDictionary)
    }
}

/// 只在内存里的安全存储：模拟内核、测试与冒烟测试用，不碰钥匙串。
public final class MemorySecureStore: SecureStore, @unchecked Sendable {
    private var items: [String: Data] = [:]
    private let lock = NSLock()
    public init() {}
    public func get(key: String) -> Data? { lock.lock(); defer { lock.unlock() }; return items[key] }
    public func set(key: String, value: Data) { lock.lock(); items[key] = value; lock.unlock() }
    public func remove(key: String) { lock.lock(); items[key] = nil; lock.unlock() }
    public var keys: [String] { lock.lock(); defer { lock.unlock() }; return Array(items.keys) }
}
