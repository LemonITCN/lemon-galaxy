import Foundation
import GalaxyKit

// 真内核冒烟测试（macOS 上用 xcframework 的 macOS 切片跑，验证 GalaxyKit 的适配层与 iOS 端用的同一套模型）：
//   swift run galaxy-core-smoke <服务器地址> <用户名> <密码> [--no-ai] [--keep-device]
// 内存 SecureStore、临时数据目录，按 iOS 主控端的配置（platform=ios, device_kind=mobile）登录，依次验证：
// 登录 → 上线 → 设备列表 → 加密库读写 → 连接 worker → 命令执行 → 终端收发 → 文件读写 → 上传下载 → AI 对话 → SSH 直连 → RDP 转发 → 隐私屏 → 远端声音 → 清理本机设备 → 登出。
// 每步打印结果与耗时；全部通过退出码 0，否则 1。

let started = Date()
func log(_ s: String) {
    let t = String(format: "%7.3f", Date().timeIntervalSince(started))
    print("[\(t)s] \(s)")
    fflush(stdout)
}

/// 去掉终端输出里的 ANSI 控制序列，便于打印
func plain(_ s: String) -> String {
    s.replacingOccurrences(of: "\u{1b}\\[[0-9;?]*[ -/]*[@-~]", with: "", options: .regularExpression)
        .replacingOccurrences(of: "\u{1b}\\][^\u{07}]*\u{07}", with: "", options: .regularExpression)
        .replacingOccurrences(of: "\r", with: "")
}

final class Buffer: @unchecked Sendable {
    private let lock = NSLock()
    private var data = Data()
    var exited: Int32??
    func append(_ d: Data) { lock.locked { data.append(d) } }
    func exit(_ c: Int32?) { lock.locked { exited = .some(c) } }
    var text: String { lock.locked { String(decoding: data, as: UTF8.self) } }
}

@MainActor
final class Smoke {
    let server: String
    let user: String
    let pass: String
    let withAI: Bool
    let keepDevice: Bool
    var failures = 0

    init(server: String, user: String, pass: String, withAI: Bool, keepDevice: Bool) {
        self.keepDevice = keepDevice
        self.server = server
        self.user = user
        self.pass = pass
        self.withAI = withAI
    }

    func step<T>(_ name: String, critical: Bool = false, _ body: () async throws -> T) async -> T? {
        let t0 = Date()
        log("开始 \(name)")
        do {
            let v = try await body()
            log(String(format: "通过 %@（%.0f ms）", name, Date().timeIntervalSince(t0) * 1000))
            return v
        } catch {
            failures += 1
            log(String(format: "失败 %@（%.0f ms）：%@", name, Date().timeIntervalSince(t0) * 1000, describeError(error)))
            return nil
        }
    }

    func waitUntil(_ seconds: Double, _ what: String, _ cond: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if cond() { return }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        throw GalaxyError.Other(detail: "\(Int(seconds)) 秒内没有\(what)")
    }

    func run() async -> Int32 {
        guard CoreProvider.realCoreAvailable else {
            print("这个构建没有接入 Rust 内核（缺 Vendor/LemonGalaxyCore.xcframework 或 Sources/Generated/galaxy_ffi.swift），无法做冒烟测试")
            return 2
        }
        let base = FileManager.default.temporaryDirectory.appendingPathComponent("lemongalaxy-ios-smoke-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: base) }
        let config = CoreEnvironment.config(dataDir: base.appendingPathComponent("data"), cacheDir: base.appendingPathComponent("cache"),
                                            appVersion: "0.1.0", osVersion: "iOS (smoke on macOS)", hostname: "LemonGalaxy-iOS-smoke", locale: "zh-CN")
        let store = MemorySecureStore()
        let account = GalaxyAccount(config: config, store: store)
        log("内核版本 \(account.core.coreVersion)（模拟内核：\(account.isMock)）；平台 \(config.platform) 设备类型 \(config.deviceKind)")
        log("device_id \(account.client.deviceId())，状态 \(account.client.state())")
        var revoked = false
        account.onSessionRevoked = { revoked = true }

        guard let result = await step("login", { try await account.login(server: server, username: user, password: pass, totp: nil) }) else { return 1 }
        log("  登录结果 \(result)；界面阶段 \(account.phase)")
        guard result == .success else { log("  需要两步验证或改密码，冒烟测试不处理"); return 1 }
        if let u = account.user { log("  用户 \(u.username)（\(u.displayName)）AI \(u.aiEnabled) TOTP \(u.totpEnabled)") }
        if let s = account.server { log("  服务器 \(s.name) \(s.version) \(s.address)") }
        log("  SecureStore 里的键：\(store.keys.sorted().joined(separator: ", "))")

        _ = await step("等待上线（ClientListener.on_state 经适配层切到主线程）") {
            try await waitUntil(20, "上线") { account.clientState == .online }
        }

        _ = await step("refresh_devices") { try await account.refreshDevices() }
        log("  设备 \(account.devices.count) 台：")
        for d in account.devices {
            log("    \(d.isSelf ? "*" : " ") \(d.hostname) \(d.kind) \(d.platform) \(d.osVersion) online=\(d.online) lan=\(d.lanSeen) wake=\(account.canWake(d.id))")
        }
        if let me = account.devices.first(where: \.isSelf) {
            log("  本机登记为 \(me.kind) / \(me.platform)")
            if me.kind != .mobile { log("失败 本机设备类型应为 mobile"); failures += 1 }
        }
        log("  本机被控服务：\(account.client.host() == nil ? "无（mobile 不能被控，符合预期）" : "有")")
        account.setLanDiscovery(true)

        let rows = HostTree.rows(account.hosts)
        log("  设备树 \(rows.count) 行（文件夹 \(rows.filter { $0.folder != nil }.count)，条目 \(rows.filter { $0.item != nil }.count)）")

        // 加密库：新建一个临时文件夹，确认 on_vault_changed 回调后能读到，再删掉
        let folderName = "ios-smoke-\(ProcessInfo.processInfo.processIdentifier)"
        if let saved = await step("vault_save（临时文件夹）", {
            try await account.save(VaultEntry(id: "", folderId: nil, sort: 9999, data: .folder(name: folderName, color: nil, expanded: true), updatedAt: 0))
        }) {
            log("  新条目 id \(saved.id)")
            _ = await step("vault_delete") {
                try await account.delete(saved.id)
                try await waitUntil(5, "从加密库消失") { !account.vault.contains { $0.id == saved.id } }
            }
        }

        guard let target = account.devices.first(where: { $0.kind == .worker && $0.online && !$0.isSelf }) ?? account.devices.first(where: { $0.kind != .mobile && $0.online && !$0.isSelf }) else {
            log("失败 没有在线的被控设备可以连接")
            await account.logout()
            return 1
        }
        log("  目标 \(target.hostname)（\(target.id)）")

        guard let peer = await step("connect（连接池）", { try await account.peer(target.id) }) else {
            await account.logout()
            return 1
        }
        let info = peer.info
        log("  路径 \(peer.path.kind) rtt=\(peer.path.rttMs)ms \(peer.path.remoteAddr)")
        log("  对端 \(info.hostname) \(info.platform) \(info.osVersion) \(info.arch) 用户 \(info.systemUser) shell \(info.shell)")
        log("  权限 view=\(info.permissions.view) control=\(info.permissions.control) files=\(info.permissions.files) terminal=\(info.permissions.terminal) ai=\(info.permissions.aiCap)")
        log("  能力 \(info.capabilities.joined(separator: ","))")
        let again = try? await account.peer(target.id)
        log("  连接复用：\(again === peer ? "同一条会话" : "新建了会话")")

        if let r = await step("exec", {
            try await peer.session.exec(request: ExecRequest(command: "echo hello && uname -a", shell: nil, cwd: nil, timeoutMs: 15_000, maxOutput: 65_536))
        }) {
            log("  退出码 \(r.code.map(String.init) ?? "无") 耗时 \(r.durationMs)ms")
            for l in r.stdout.split(separator: "\n") { log("  stdout| \(l)") }
            if !r.stdout.contains("hello") { log("失败 exec 输出里没有 hello"); failures += 1 }
        }

        let buf = Buffer()
        let sink = TerminalSinkAdapter(onData: { buf.append($0) }, onExit: { buf.exit($0) })
        if let term = await step("open_terminal（TerminalSinkAdapter）", {
            try await peer.session.openTerminal(options: TerminalOptions(attachSession: nil, shell: nil, cols: 80, rows: 24,
                                                                          env: [EnvVar(name: "TERM", value: "xterm-256color")]), sink: sink)
        }) {
            log("  会话 \(term.sessionId()) 标题 \(term.title())")
            _ = await step("终端收发（含特殊键编码）") {
                try await Task.sleep(nanoseconds: 300_000_000)
                term.write(data: Data("echo smoke-$((40+2))".utf8))
                term.write(data: Data(TerminalKeys.bytes(.enter)))
                try await waitUntil(10, "收到命令输出") { buf.text.contains("smoke-42") }
                term.resize(cols: 100, rows: 30)
                term.write(data: Data("stty size".utf8) + Data(TerminalKeys.bytes(.enter)))
                try await waitUntil(5, "看到新尺寸 30 100") { plain(buf.text).contains("30 100") }
            }
            for l in plain(buf.text).split(separator: "\n").suffix(4) { log("  term| \(l)") }
            term.detach()
            if let list = await step("list_terminals（分离后保留）", { try await peer.session.listTerminals() }) {
                log("  被控端终端会话 \(list.count) 个：" + list.map { "\($0.id)(\($0.attached ? "已连" : "保留"))" }.joined(separator: " "))
                if let kept = list.first(where: { $0.id == term.sessionId() }) {
                    _ = await step("kill_terminal") { try await peer.session.killTerminal(sessionId: kept.id) }
                }
            }
            sink.detach()
        }

        let files = peer.session.files()
        if let home = await step("files.home", { try await files.home() }) {
            log("  主目录 \(home)")
            if let entries = await step("files.list", { try await files.list(path: home) }) {
                let sorted = FileSort.sorted(entries, by: .name, descending: false, showHidden: false)
                log("  共 \(entries.count) 项（隐藏文件之外 \(sorted.count) 项），前 8 项：")
                for e in sorted.prefix(8) { log("    \(Fmt.mode(e.mode, kind: e.kind)) \(Fmt.bytes(e.size)) \(e.name)") }
            }
            let dir = RemotePath.join(home, ".lemongalaxy-ios-smoke-\(ProcessInfo.processInfo.processIdentifier)")
            let file = RemotePath.join(dir, "hello.txt")
            _ = await step("mkdir / write_text / read_text / rename / chmod") {
                try await files.mkdir(path: dir)
                try await files.writeText(path: file, text: "你好 LemonGalaxy\n")
                let back = try await files.readText(path: file, maxBytes: 4096)
                guard back == "你好 LemonGalaxy\n" else { throw GalaxyError.Other(detail: "读回的内容不一致：\(back)") }
                let renamed = RemotePath.join(dir, "hello2.txt")
                try await files.rename(from: file, to: renamed)
                try await files.chmod(path: renamed, mode: 0o600)
                let st = try await files.stat(path: renamed)
                log("  \(st.name) \(Fmt.mode(st.mode, kind: st.kind)) \(st.size) 字节")
            }
            // 上传、下载（TransferCenter + TransferListenerAdapter）
            let localDir = base.appendingPathComponent("transfer", isDirectory: true)
            try? FileManager.default.createDirectory(at: localDir, withIntermediateDirectories: true)
            let up = localDir.appendingPathComponent("up.bin")
            var payload = Data(count: 3 * 1024 * 1024 + 17)
            payload.withUnsafeMutableBytes { p in for i in 0..<p.count { p[i] = UInt8(truncatingIfNeeded: i &* 31 &+ 7) } }
            try? payload.write(to: up)
            let center = TransferCenter()
            let remoteBin = RemotePath.join(dir, "up.bin")
            _ = await step("upload 3 MB（TransferCenter）") {
                let item = center.upload(files, deviceId: target.id, deviceName: target.hostname, local: up, remote: remoteBin)
                try await waitUntil(60, "上传完成") { !item.isActive }
                guard item.status == .done else { throw GalaxyError.Other(detail: "上传状态 \(item.status)") }
                log("  进度 \(item.done)/\(item.total)")
            }
            let down = localDir.appendingPathComponent("down.bin")
            _ = await step("download 3 MB 并比对") {
                let item = center.download(files, deviceId: target.id, deviceName: target.hostname, remote: remoteBin, to: down)
                try await waitUntil(60, "下载完成") { !item.isActive }
                guard item.status == .done else { throw GalaxyError.Other(detail: "下载状态 \(item.status)") }
                let got = try Data(contentsOf: down)
                guard got == payload else { throw GalaxyError.Other(detail: "下载内容与上传的不一致（\(got.count) 字节）") }
            }
            _ = await step("remove（递归删除临时目录）") { try await files.remove(path: dir, recursive: true) }
        }

        if let displays = try? await peer.session.listDisplays() {
            log("  屏幕 \(displays.count) 块：" + displays.map { "\($0.id) \($0.name) \($0.width)x\($0.height)" }.joined(separator: "; "))
        } else {
            log("  这台设备没有可用的屏幕（Linux worker 正常）")
        }

        if withAI {
            let hub = AgentHub(client: { account.client }, hosts: { account.hosts })
            await hub.loadModels()
            if let e = hub.modelsError {
                log("失败 ai_models：\(e)")
                failures += 1
            } else if let models = hub.models {
                log("  AI 开通 \(models.enabled) 上限 \(models.maxMode) 模型 \(models.items.count) 个：" + models.items.prefix(6).map(\.name).joined(separator: ", "))
                if models.enabled, let m = models.items.first(where: \.tools) ?? models.items.first {
                    let cfg = ConversationConfig(providerId: m.providerId, modelId: m.id, targets: [AgentTarget(kind: .device, id: target.id)],
                                                 mode: .readOnly, planFirst: false, trustMinutes: nil, title: "iOS smoke")
                    if let conv = await step("new_conversation（\(m.name)，只读）", { try await hub.create(cfg) }) {
                        _ = await step("AI 对话（AgentListenerAdapter + ChatReducer）") {
                            conv.send("这是连通性测试。不要调用任何工具，只回复两个字：收到")
                            try await waitUntil(90, "得到回复") {
                                !conv.state.running && conv.state.items.contains { if case .assistant(_, let t, _, false) = $0 { return !t.isEmpty }; if case .error = $0 { return true }; return false }
                            }
                            for item in conv.state.items {
                                switch item {
                                case .assistant(_, let t, _, _): log("  助手| \(t.prefix(120))")
                                case .error(_, let m): throw GalaxyError.Other(detail: "智能体报错：\(m)")
                                case .tool(let t): log("  工具| \(t.call.tool) \(t.call.summary) \(t.phase)")
                                default: break
                                }
                            }
                            log("  步数 \(conv.state.step) token \(conv.state.promptTokens)/\(conv.state.completionTokens)")
                        }
                        conv.setForeground(false)
                        hub.refreshList()
                        log("  本机保存的对话 \(hub.items.count) 个")
                        hub.delete(conv.id)
                    }
                } else {
                    log("  这个账号没有开通 AI 或没有模型，跳过对话")
                }
            }
        }

        // SSH 直连：加一条指向本机关闭端口的临时主机，确认 connect_ssh 走通并把连接错误正确映射（不弹任何认证框）
        if let tmp = await step("vault_save（临时 SSH 主机）", {
            try await account.save(VaultEntry(id: "", folderId: nil, sort: 9999, data: .sshHost(
                label: "ios-smoke-ssh", host: "127.0.0.1", port: 1, username: "nobody", auth: .password(password: "x"),
                jumpDeviceId: nil, jumpHostId: nil, tags: [], note: "", favorite: false, terminalTheme: nil), updatedAt: 0))
        }) {
            var prompted = false
            account.secretPresenter = { req in prompted = true; req.cancel() }
            let t0 = Date()
            do {
                _ = try await account.ssh(tmp.id)
                log("失败 连到关闭的端口居然成功了")
                failures += 1
            } catch {
                log(String(format: "通过 connect_ssh 报错（%.0f ms，%@）：%@", Date().timeIntervalSince(t0) * 1000, String(describing: type(of: error)), describeError(error)))
                if prompted { log("  注意：连接失败前弹了认证框") }
            }
            _ = await step("vault_delete（临时 SSH 主机）") { try await account.delete(tmp.id) }
        }

        // RDP：经设备转发时本机开临时端口，停止后端口关闭；直连时原样给地址；链接不含密码
        if let rdp = await step("vault_save（临时 RDP 主机，经由 \(target.hostname)）", {
            try await account.save(VaultEntry(id: "", folderId: nil, sort: 9999, data: .rdpHost(
                label: "ios-smoke-rdp", host: "127.0.0.1", port: 3389, username: "smoke", password: "secret", domain: "LAB",
                jumpDeviceId: target.id, tags: [], note: "", favorite: false), updatedAt: 0))
        }) {
            if let t = await step("open_rdp（经设备转发）", { try await account.openRdp(rdp.id) }) {
                let link = RdpLink.string(for: t)
                log("  转发 \(t.forwarded) 本机 \(t.host):\(t.port) 链接 \(link)")
                if !t.forwarded || t.port == 0 || link.contains("secret") || RdpLink.url(for: t) == nil { log("失败 open_rdp 结果不对"); failures += 1 }
                let listening = tcpConnectable(t.port)
                account.closeRdp(t.forwardId)
                try? await Task.sleep(nanoseconds: 300_000_000)
                let afterClose = tcpConnectable(t.port)
                log("  本机端口：转发中可连 \(listening)，停止后可连 \(afterClose)；记录的转发 \(account.rdpForwards.count) 条")
                if !listening || afterClose || !account.rdpForwards.isEmpty { log("失败 close_rdp 没有关掉转发"); failures += 1 }
            }
            _ = await step("vault_delete（临时 RDP 主机）") { try await account.delete(rdp.id) }
        }

        if info.capabilities.contains("privacy") {
            _ = await step("set_privacy(on/off)") {
                try await peer.session.setPrivacy(on: true)
                try await peer.session.setPrivacy(on: false)
            }
        } else {
            log("  对端能力里没有 privacy：界面不显示隐私屏开关（这台被控端：\(info.capabilities.joined(separator: ",")))")
        }

        // 远端声音：内核用本机默认输出播放（冒烟测试音量 0，不出声），停止后不再活动
        if info.capabilities.contains("audio") {
            _ = await step("start_audio / stop") {
                let p = try await peer.session.startAudio(volume: 0)
                try await Task.sleep(nanoseconds: 1_500_000_000)
                guard p.isActive() else { throw GalaxyError.Other(detail: "开始播放后 is_active 为假") }
                p.setMuted(muted: true)
                p.stop()
                guard !p.isActive() else { throw GalaxyError.Other(detail: "停止后 is_active 仍为真") }
            }
        } else {
            log("  对端能力里没有 audio：界面不显示声音开关（这台被控端：\(info.capabilities.joined(separator: ",")))")
        }

        account.setLanDiscovery(false)
        account.release(target.id)

        // 清理冒烟测试自己登记的设备（每次运行都会登记一台新的 mobile 设备）：先删以前残留的离线同名设备，再删本机
        if !keepDevice {
            for d in account.devices where d.hostname == "LemonGalaxy-iOS-smoke" && d.kind == .mobile && d.platform == .ios && !d.isSelf && !d.online {
                _ = await step("remove_device（以前残留的 \(d.id.prefix(8))）") { try await account.removeDevice(d.id) }
            }
            if let me = account.devices.first(where: \.isSelf) {
                // 移除本机后服务器随即注销本机会话：删除请求本身成功，随后刷新设备列表会报「未登录」，这是预期结果
                log("开始 remove_device（本机）")
                do {
                    try await account.removeDevice(me.id)
                    log("通过 remove_device（本机）")
                } catch {
                    if case GalaxyError.Auth = error {
                        log("通过 remove_device（本机）：本机会话随之注销（\(describeError(error))）")
                    } else {
                        failures += 1
                        log("失败 remove_device（本机）：\(describeError(error))")
                    }
                }
                try? await waitUntil(5, "收到注销回调") { revoked }
            }
        }
        _ = await step("logout") { await account.logout() }
        log("  最终状态 \(account.client.state())；界面阶段 \(account.phase)；被吊销 \(revoked)")
        log(failures == 0 ? "全部通过" : "有 \(failures) 步失败")
        return failures == 0 ? 0 : 1
    }
}

/// 本机 127.0.0.1 的端口能不能连上（检查 RDP 转发的监听）
func tcpConnectable(_ port: UInt16) -> Bool {
    let fd = socket(AF_INET, SOCK_STREAM, 0)
    guard fd >= 0 else { return false }
    defer { close(fd) }
    var addr = sockaddr_in()
    addr.sin_family = sa_family_t(AF_INET)
    addr.sin_port = port.bigEndian
    addr.sin_addr.s_addr = inet_addr("127.0.0.1")
    let r = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
    }
    return r == 0
}

let args = CommandLine.arguments
guard args.count >= 4 else {
    print("用法：galaxy-core-smoke <服务器地址> <用户名> <密码> [--no-ai] [--keep-device]")
    exit(2)
}
Task { @MainActor in
    let smoke = Smoke(server: args[1], user: args[2], pass: args[3], withAI: !args.contains("--no-ai"), keepDevice: args.contains("--keep-device"))
    let code = await smoke.run()
    exit(code)
}
dispatchMain()
