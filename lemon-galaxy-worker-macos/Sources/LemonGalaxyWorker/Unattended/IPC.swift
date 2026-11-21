import Foundation
import Darwin
import Security

// 无人值守模式下进程之间的本机通信：后台服务（root，LaunchDaemon）、会话代理（每个图形会话一个，LaunchAgent）、
// 菜单栏应用（界面）。走 Unix 域套接字，每条消息：
//   [u32 头长][u32 正文长][u32 附件长]（小端）+ 头（JSON）+ 正文（JSON）+ 附件（原始字节，画面帧、声音样本）
// 头里有消息类型（请求 / 应答 / 通知）、编号、方法名、错误。请求在并发队列上处理，通知按到达顺序串行处理（键鼠不乱序）。
// 对端身份：内核给的 uid / gid / pid / 审计令牌（LOCAL_PEERCRED / LOCAL_PEERPID / LOCAL_PEERTOKEN），
// 再用审计令牌核对对端代码签名与本进程的指定要求一致（三种角色是同一个可执行文件）。

enum IPCError: Error, CustomStringConvertible {
    case closed
    case timeout(String)
    case io(String)
    case denied(String)
    case remote(IPCFault)
    case decode(String)

    var description: String {
        switch self {
        case .closed: return "连接已断开"
        case .timeout(let m): return "等待应答超时：\(m)"
        case .io(let m): return "读写失败：\(m)"
        case .denied(let m): return "拒绝：\(m)"
        case .remote(let f): return f.detail
        case .decode(let m): return "消息格式不对：\(m)"
        }
    }
}

/// 应答里的错误（内核错误原样带过去，界面照常显示）
struct IPCFault: Codable, Equatable, Error {
    var kind: String
    var code: String?
    var detail: String

    static func other(_ detail: String) -> IPCFault { IPCFault(kind: "other", code: nil, detail: detail) }
    static func denied(_ detail: String) -> IPCFault { IPCFault(kind: "permission", code: nil, detail: detail) }

    init(kind: String, code: String?, detail: String) {
        self.kind = kind
        self.code = code
        self.detail = detail
    }

    init(_ error: Error) {
        if let f = error as? IPCFault { self = f; return }
        if let g = error as? GalaxyError {
            switch g {
            case .Network(let d): self.init(kind: "network", code: nil, detail: d)
            case .Auth(let c, let d): self.init(kind: "auth", code: c, detail: d)
            case .Permission(let d): self.init(kind: "permission", code: nil, detail: d)
            case .NotFound(let d): self.init(kind: "notFound", code: nil, detail: d)
            case .Protocol(let d): self.init(kind: "protocol", code: nil, detail: d)
            case .Cancelled: self.init(kind: "cancelled", code: nil, detail: "")
            case .Other(let d): self.init(kind: "other", code: nil, detail: d)
            }
            return
        }
        if let e = error as? IPCError, case .remote(let f) = e { self = f; return }
        self.init(kind: "other", code: nil, detail: String(describing: error))
    }

    var galaxyError: GalaxyError {
        switch kind {
        case "network": return .Network(detail: detail)
        case "auth": return .Auth(code: code ?? "", detail: detail)
        case "permission": return .Permission(detail: detail)
        case "notFound": return .NotFound(detail: detail)
        case "protocol": return .Protocol(detail: detail)
        case "cancelled": return .Cancelled
        default: return .Other(detail: detail)
        }
    }
}

struct IPCEmpty: Codable {}

// MARK: - 对端身份

struct PeerCredentials: Equatable {
    var uid: uid_t
    var gid: gid_t
    var groups: [gid_t]
    var pid: pid_t
    /// 审计令牌（32 字节），用来核对代码签名
    var auditToken: Data?

    var userName: String {
        guard let pw = getpwuid(uid), let n = pw.pointee.pw_name else { return "\(uid)" }
        return String(cString: n)
    }

    /// 管理员：root 或 admin 组（gid 80）成员。套接字凭据只带前 16 个组，不全时再查一次组成员
    var isAdmin: Bool {
        if uid == 0 || groups.contains(80) { return true }
        guard let pw = getpwuid(uid) else { return false }
        var n: Int32 = 64
        var list = [Int32](repeating: 0, count: Int(n))
        let base = Int32(bitPattern: pw.pointee.pw_gid)
        if getgrouplist(pw.pointee.pw_name, base, &list, &n) == -1 {
            list = [Int32](repeating: 0, count: Int(n) + 64)
            n = Int32(list.count)
            _ = getgrouplist(pw.pointee.pw_name, base, &list, &n)
        }
        return list.prefix(Int(max(0, n))).contains(80)
    }

    static func of(fd: Int32) -> PeerCredentials {
        var cred = xucred()
        var len = socklen_t(MemoryLayout<xucred>.size)
        var uid: uid_t = .max, gid: gid_t = .max
        var groups: [gid_t] = []
        if getsockopt(fd, SOL_LOCAL, LOCAL_PEERCRED, &cred, &len) == 0, cred.cr_version == XUCRED_VERSION {
            uid = cred.cr_uid
            let n = Int(max(0, min(Int(cred.cr_ngroups), Int(NGROUPS))))
            groups = withUnsafeBytes(of: cred.cr_groups) { Array($0.bindMemory(to: gid_t.self).prefix(n)) }
            gid = groups.first ?? .max
        }
        var pid: pid_t = 0
        len = socklen_t(MemoryLayout<pid_t>.size)
        _ = getsockopt(fd, SOL_LOCAL, LOCAL_PEERPID, &pid, &len)
        var token = audit_token_t()
        len = socklen_t(MemoryLayout<audit_token_t>.size)
        let tokenData: Data? = getsockopt(fd, SOL_LOCAL, LOCAL_PEERTOKEN, &token, &len) == 0
            ? withUnsafeBytes(of: &token) { Data($0) } : nil
        return PeerCredentials(uid: uid, gid: gid, groups: groups, pid: pid, auditToken: tokenData)
    }
}

/// 代码签名核对：对端必须满足本进程的指定要求（Developer ID 下是「同一 Bundle ID + 同一团队」，临时签名下是同一份可执行文件）。
enum CodeIdentity {
    /// 本进程的指定要求；没有签名（极少见的开发构建）时为空，此时不核对签名、只按 uid 授权
    static let ownRequirement: SecRequirement? = {
        var code: SecCode?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code else { return nil }
        var st: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &st) == errSecSuccess, let st else { return nil }
        var req: SecRequirement?
        guard SecCodeCopyDesignatedRequirement(st, [], &req) == errSecSuccess else { return nil }
        return req
    }()

    static var ownRequirementText: String {
        guard let r = ownRequirement else { return "" }
        var s: CFString?
        return SecRequirementCopyString(r, [], &s) == errSecSuccess ? (s as String? ?? "") : ""
    }

    static func peerMatches(auditToken: Data?) -> Bool {
        guard let req = ownRequirement else { return true }
        guard let token = auditToken else { return false }
        var code: SecCode?
        let attrs = [kSecGuestAttributeAudit: token] as CFDictionary
        guard SecCodeCopyGuestWithAttributes(nil, attrs, [], &code) == errSecSuccess, let code else { return false }
        return SecCodeCheckValidity(code, [], req) == errSecSuccess
    }
}

// MARK: - 套接字

enum IPCSocket {
    /// 套接字路径最长 103 字节
    static func address(_ path: String) -> sockaddr_un? {
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        let cap = MemoryLayout.size(ofValue: addr.sun_path)
        guard bytes.count < cap else { return nil }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        addr.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        return addr
    }

    static func configure(_ fd: Int32) {
        var one: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
        // 画面帧走这条连接：缓冲开大一些，减少系统调用与写阻塞
        var buf: Int32 = 2 * 1024 * 1024
        setsockopt(fd, SOL_SOCKET, SO_SNDBUF, &buf, socklen_t(MemoryLayout<Int32>.size))
        setsockopt(fd, SOL_SOCKET, SO_RCVBUF, &buf, socklen_t(MemoryLayout<Int32>.size))
        _ = fcntl(fd, F_SETFD, FD_CLOEXEC)
    }

    /// 连接到服务端；失败抛错
    static func connect(path: String) throws -> Int32 {
        guard var addr = address(path) else { throw IPCError.io("套接字路径太长：\(path)") }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw IPCError.io(String(cString: strerror(errno))) }
        configure(fd)
        let r = withUnsafePointer(to: &addr) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        if r != 0 {
            let e = String(cString: strerror(errno))
            Darwin.close(fd)
            throw IPCError.io(e)
        }
        return fd
    }
}

/// 服务端：监听、接受连接（每个连接交给回调）。
final class IPCServer: @unchecked Sendable {
    let path: String
    private let fd: Int32
    private let lock = NSLock()
    private var stopped = false

    /// mode：套接字文件权限。后台服务用 0666（谁都能连，连上后按对端身份授权）
    init(path: String, mode: mode_t = 0o666) throws {
        self.path = path
        guard var addr = IPCSocket.address(path) else { throw IPCError.io("套接字路径太长：\(path)") }
        // 上次异常退出留下的套接字文件
        unlink(path)
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw IPCError.io(String(cString: strerror(errno))) }
        _ = fcntl(fd, F_SETFD, FD_CLOEXEC)
        let r = withUnsafePointer(to: &addr) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard r == 0 else {
            let e = String(cString: strerror(errno))
            Darwin.close(fd)
            throw IPCError.io("绑定 \(path) 失败：\(e)")
        }
        chmod(path, mode)
        guard listen(fd, 16) == 0 else {
            let e = String(cString: strerror(errno))
            Darwin.close(fd)
            throw IPCError.io(e)
        }
        self.fd = fd
    }

    func start(accept handler: @escaping (IPCConnection) -> Void) {
        let t = Thread { [self] in
            while !lock.locked({ stopped }) {
                let c = Darwin.accept(fd, nil, nil)
                if c < 0 {
                    if errno == EINTR || errno == ECONNABORTED { continue }
                    if lock.locked({ stopped }) { break }
                    Thread.sleep(forTimeInterval: 0.2)
                    continue
                }
                IPCSocket.configure(c)
                handler(IPCConnection(fd: c))
            }
        }
        t.name = "galaxy.ipc.accept"
        t.start()
    }

    func stop() {
        let first = lock.locked { () -> Bool in
            defer { stopped = true }
            return !stopped
        }
        guard first else { return }
        shutdown(fd, SHUT_RDWR)
        Darwin.close(fd)
        unlink(path)
    }
}

// MARK: - 连接

final class IPCConnection: @unchecked Sendable {
    enum Kind: Int, Codable { case request = 0, response = 1, notify = 2 }

    struct Header: Codable {
        var k: Kind
        var i: UInt64
        var m: String
        var err: IPCFault?
    }

    struct Message {
        var kind: Kind
        var id: UInt64
        var method: String
        var fault: IPCFault?
        var body: Data
        var blob: Data

        func decode<T: Decodable>(_ type: T.Type = T.self) throws -> T {
            do {
                return try IPCJSON.decoder.decode(T.self, from: body.isEmpty ? Data("{}".utf8) : body)
            } catch {
                throw IPCError.decode("\(method)：\(error)")
            }
        }
    }

    /// 处理请求时回应答用（只能调一次）
    final class Reply: @unchecked Sendable {
        private weak var conn: IPCConnection?
        private let id: UInt64
        private let method: String
        private let lock = NSLock()
        private var done = false
        init(conn: IPCConnection, id: UInt64, method: String) {
            self.conn = conn
            self.id = id
            self.method = method
        }
        private func once() -> Bool { lock.locked { defer { done = true }; return !done } }
        func ok<T: Encodable>(_ body: T, blob: Data = Data()) {
            guard once() else { return }
            conn?.send(kind: .response, id: id, method: method, fault: nil, body: IPCJSON.encode(body), blob: blob)
        }
        func ok() { ok(IPCEmpty()) }
        func fail(_ fault: IPCFault) {
            guard once() else { return }
            conn?.send(kind: .response, id: id, method: method, fault: fault, body: Data(), blob: Data())
        }
        func fail(_ error: Error) { fail(IPCFault(error)) }
    }

    static let maxHeader = 1 << 20
    static let maxPart = 128 << 20

    let fd: Int32
    let peer: PeerCredentials
    /// 由使用方挂的标签（日志用）
    var label = "ipc"

    private let lock = NSLock()
    private var closed = false
    private var started = false
    private var nextId: UInt64 = 1
    private var pending: [UInt64: (Message?) -> Void] = [:]
    private var queuedBytes = 0
    private let writeQueue = DispatchQueue(label: "galaxy.ipc.write")
    private let notifyQueue = DispatchQueue(label: "galaxy.ipc.notify")
    private let requestQueue = DispatchQueue(label: "galaxy.ipc.request", attributes: .concurrent)

    private var requestHandler: ((Message, Reply) -> Void)?
    private var notifyHandler: ((Message) -> Void)?
    private var closeHandler: (() -> Void)?

    init(fd: Int32) {
        self.fd = fd
        peer = PeerCredentials.of(fd: fd)
    }

    deinit { Darwin.close(fd) }

    /// 接上处理函数并开始读；处理函数之后还可以换（第一条消息决定角色时）
    func start(onRequest: ((Message, Reply) -> Void)? = nil, onNotify: ((Message) -> Void)? = nil, onClose: (() -> Void)? = nil) {
        lock.locked {
            requestHandler = onRequest
            notifyHandler = onNotify
            closeHandler = onClose
        }
        let go = lock.locked { () -> Bool in defer { started = true }; return !started }
        guard go else { return }
        let t = Thread { [self] in readLoop() }
        t.name = "galaxy.ipc.read"
        t.qualityOfService = .userInteractive
        t.start()
    }

    func setHandlers(onRequest: ((Message, Reply) -> Void)?, onNotify: ((Message) -> Void)?, onClose: (() -> Void)?) {
        lock.locked {
            requestHandler = onRequest
            notifyHandler = onNotify
            closeHandler = onClose
        }
    }

    var isClosed: Bool { lock.locked { closed } }

    /// 还没写出去的字节数（发画面帧前看一眼，积压时丢帧）
    var backlog: Int { lock.locked { queuedBytes } }

    func close() {
        let r = lock.locked { () -> ([(Message?) -> Void], (() -> Void)?)? in
            guard !closed else { return nil }
            closed = true
            let h = Array(pending.values)
            pending.removeAll()
            let c = closeHandler
            closeHandler = nil
            requestHandler = nil
            notifyHandler = nil
            return (h, c)
        }
        guard let (handlers, onClose) = r else { return }
        // 只关读写，文件描述符在对象释放时关（写队列里可能还有任务在用）
        shutdown(fd, SHUT_RDWR)
        for h in handlers { h(nil) }
        onClose?()
    }

    // MARK: 发送

    @discardableResult
    fileprivate func send(kind: Kind, id: UInt64, method: String, fault: IPCFault?, body: Data, blob: Data, droppable: Bool = false, dropAbove: Int = 0) -> Bool {
        let header = IPCJSON.encode(Header(k: kind, i: id, m: method, err: fault))
        let size = 12 + header.count + body.count + blob.count
        let ok = lock.locked { () -> Bool in
            if closed { return false }
            if droppable && queuedBytes > dropAbove { return false }
            queuedBytes += size
            return true
        }
        guard ok else { return false }
        writeQueue.async { [self] in
            var prefix = Data(count: 12)
            prefix.withUnsafeMutableBytes { p in
                p.storeBytes(of: UInt32(header.count).littleEndian, toByteOffset: 0, as: UInt32.self)
                p.storeBytes(of: UInt32(body.count).littleEndian, toByteOffset: 4, as: UInt32.self)
                p.storeBytes(of: UInt32(blob.count).littleEndian, toByteOffset: 8, as: UInt32.self)
            }
            let ok = writeAll(prefix) && writeAll(header) && writeAll(body) && writeAll(blob)
            lock.locked { queuedBytes -= size }
            if !ok { close() }
        }
        return true
    }

    private func writeAll(_ d: Data) -> Bool {
        if d.isEmpty { return true }
        return d.withUnsafeBytes { raw -> Bool in
            guard var p = raw.baseAddress else { return true }
            var left = raw.count
            while left > 0 {
                let n = Darwin.write(fd, p, left)
                if n < 0 {
                    if errno == EINTR { continue }
                    return false
                }
                left -= n
                p += n
            }
            return true
        }
    }

    /// 单向通知。droppable 为真且积压超过 dropAbove 字节时不发（画面帧），返回是否发出
    @discardableResult
    func notify<T: Encodable>(_ method: String, _ body: T, blob: Data = Data(), droppable: Bool = false, dropAbove: Int = 8 << 20) -> Bool {
        send(kind: .notify, id: 0, method: method, fault: nil, body: IPCJSON.encode(body), blob: blob, droppable: droppable, dropAbove: dropAbove)
    }

    @discardableResult
    func notify(_ method: String) -> Bool { notify(method, IPCEmpty()) }

    /// 阻塞调用（内核线程、后台线程用；主线程上只用于很快的本机查询）
    func call<T: Encodable>(_ method: String, _ body: T, blob: Data = Data(), timeout: Double) throws -> Message {
        let sem = DispatchSemaphore(value: 0)
        nonisolated(unsafe) var result: Message?
        let id = lock.locked { () -> UInt64? in
            guard !closed else { return nil }
            let id = nextId
            nextId += 1
            pending[id] = { m in result = m; sem.signal() }
            return id
        }
        guard let id else { throw IPCError.closed }
        guard send(kind: .request, id: id, method: method, fault: nil, body: IPCJSON.encode(body), blob: blob) else {
            lock.locked { _ = pending.removeValue(forKey: id) }
            throw IPCError.closed
        }
        if sem.wait(timeout: .now() + timeout) == .timedOut {
            lock.locked { _ = pending.removeValue(forKey: id) }
            throw IPCError.timeout(method)
        }
        guard let m = result else { throw IPCError.closed }
        if let f = m.fault { throw IPCError.remote(f) }
        return m
    }

    func call(_ method: String, timeout: Double) throws -> Message { try call(method, IPCEmpty(), timeout: timeout) }

    /// 异步调用（界面用）。超时 timeout 秒；登录这类要等网络的给长一点
    func callAsync<T: Encodable>(_ method: String, _ body: T, blob: Data = Data(), timeout: Double = 120) async throws -> Message {
        let id = lock.locked { () -> UInt64? in
            guard !closed else { return nil }
            let id = nextId
            nextId += 1
            return id
        }
        guard let id else { throw IPCError.closed }
        let m: Message? = await withCheckedContinuation { (cont: CheckedContinuation<Message?, Never>) in
            let once = OnceFlag()
            let registered = lock.locked { () -> Bool in
                guard !closed else { return false }
                pending[id] = { m in if once.fire() { cont.resume(returning: m) } }
                return true
            }
            guard registered else { cont.resume(returning: nil); return }
            DispatchQueue.global().asyncAfter(deadline: .now() + timeout) { [weak self] in
                guard once.fire() else { return }
                self?.lock.locked { _ = self?.pending.removeValue(forKey: id) }
                cont.resume(returning: Message(kind: .response, id: id, method: method, fault: IPCFault(kind: "network", code: "timeout", detail: IPCError.timeout(method).description), body: Data(), blob: Data()))
            }
            if !send(kind: .request, id: id, method: method, fault: nil, body: IPCJSON.encode(body), blob: blob) {
                lock.locked { _ = pending.removeValue(forKey: id) }
                if once.fire() { cont.resume(returning: nil) }
            }
        }
        guard let m else { throw IPCError.closed }
        if let f = m.fault { throw IPCError.remote(f) }
        return m
    }

    // MARK: 接收

    private func readExact(_ n: Int) -> Data? {
        if n == 0 { return Data() }
        var d = Data(count: n)
        let ok = d.withUnsafeMutableBytes { raw -> Bool in
            guard var p = raw.baseAddress else { return false }
            var left = n
            while left > 0 {
                let r = Darwin.read(fd, p, left)
                if r < 0 {
                    if errno == EINTR { continue }
                    return false
                }
                if r == 0 { return false }
                left -= r
                p += r
            }
            return true
        }
        return ok ? d : nil
    }

    private func readLoop() {
        while true {
            guard let prefix = readExact(12) else { break }
            let (hl, bl, xl) = prefix.withUnsafeBytes { p in
                (Int(UInt32(littleEndian: p.loadUnaligned(fromByteOffset: 0, as: UInt32.self))),
                 Int(UInt32(littleEndian: p.loadUnaligned(fromByteOffset: 4, as: UInt32.self))),
                 Int(UInt32(littleEndian: p.loadUnaligned(fromByteOffset: 8, as: UInt32.self))))
            }
            guard hl > 0, hl <= Self.maxHeader, bl <= Self.maxPart, xl <= Self.maxPart else {
                NSLog("LemonGalaxy IPC(%@): 消息长度不合法，断开", label)
                break
            }
            guard let h = readExact(hl), let body = readExact(bl), let blob = readExact(xl) else { break }
            guard let header = try? IPCJSON.decoder.decode(Header.self, from: h) else {
                NSLog("LemonGalaxy IPC(%@): 消息头无法解析，断开", label)
                break
            }
            let msg = Message(kind: header.k, id: header.i, method: header.m, fault: header.err, body: body, blob: blob)
            dispatch(msg)
        }
        close()
    }

    private func dispatch(_ msg: Message) {
        switch msg.kind {
        case .response:
            let h = lock.locked { pending.removeValue(forKey: msg.id) }
            h?(msg)
        case .request:
            let reply = Reply(conn: self, id: msg.id, method: msg.method)
            requestQueue.async { [weak self] in
                guard let self else { return }
                if let h = self.lock.locked({ self.requestHandler }) {
                    h(msg, reply)
                } else {
                    reply.fail(.other("没有处理 \(msg.method) 的对象"))
                }
            }
        case .notify:
            notifyQueue.async { [weak self] in
                guard let self, let h = self.lock.locked({ self.notifyHandler }) else { return }
                h(msg)
            }
        }
    }
}

/// 只触发一次的标记（超时与应答二选一）
final class OnceFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var fired = false
    func fire() -> Bool { lock.locked { defer { fired = true }; return !fired } }
}

enum IPCJSON {
    static let decoder = JSONDecoder()
    static let encoderInstance: JSONEncoder = {
        let e = JSONEncoder()
        e.outputFormatting = []
        return e
    }()
    private static let lock = NSLock()
    static func encode<T: Encodable>(_ v: T) -> Data {
        if v is IPCEmpty { return Data() }
        // JSONEncoder 不保证并发安全，串行编码（消息都很小，画面帧走附件不经过这里）
        return lock.locked { (try? encoderInstance.encode(v)) ?? Data() }
    }
}
