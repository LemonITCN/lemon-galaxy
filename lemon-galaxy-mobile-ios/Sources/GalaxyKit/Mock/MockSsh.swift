import Foundation

// 模拟 SSH 直连：与真内核一样走 SecretPrompt（没存密码时问密码、键盘交互逐个问、首次连接确认主机密钥并存进加密库）。

extension MockGalaxyClient {
    func mockConnectSsh(hostId: String, prompt: SecretPrompt) async throws -> SshSession {
        await delay(500)
        guard let e = vault().first(where: { $0.id == hostId }),
              case let .sshHost(label, host, port, username, auth, jumpDevice, jumpHost, _, _, _, _) = e.data else {
            throw GalaxyError.NotFound(detail: zh ? "主机不存在" : "Host not found")
        }
        // 内核在自己的线程上同步调用 SecretPrompt：模拟时也放到后台线程，界面才能弹窗等回答
        let known = vault().first {
            if case .knownHost(let h, let p, _, _) = $0.data { return h.caseInsensitiveCompare(host) == .orderedSame && p == port }
            return false
        }
        let fingerprint = "SHA256:" + Data((host + ":\(port)").utf8).base64EncodedString().prefix(43)
        if known == nil {
            let ok = await Self.blocking { prompt.confirmHostKey(host: host, port: port, keyType: "ssh-ed25519", fingerprint: String(fingerprint), changed: false) }
            guard ok else { throw GalaxyError.Cancelled }
            _ = try await vaultSave(entry: VaultEntry(id: "", folderId: nil, sort: 0,
                                                      data: .knownHost(host: host, port: port, keyType: "ssh-ed25519", fingerprint: String(fingerprint)), updatedAt: 0))
        }
        switch auth {
        case .password(let pw) where pw == nil || pw!.isEmpty:
            let answer = await Self.blocking { prompt.ask(prompt: self.zh ? "\(username)@\(host) 的密码：" : "Password for \(username)@\(host):", echo: false) }
            guard let answer, !answer.isEmpty else { throw GalaxyError.Cancelled }
            if answer == "wrong" { throw GalaxyError.Auth(code: "ssh_auth", detail: zh ? "SSH 认证失败" : "SSH authentication failed") }
        case .interactive:
            let answer = await Self.blocking { prompt.ask(prompt: self.zh ? "验证码（键盘交互）：" : "Verification code (keyboard-interactive):", echo: true) }
            guard answer != nil else { throw GalaxyError.Cancelled }
        case .agent:
            throw GalaxyError.Other(detail: zh ? "暂不支持 SSH agent 认证" : "SSH agent authentication is not supported yet")
        default:
            break
        }
        let via: String
        if let jumpDevice { via = "device:" + (device(jumpDevice)?.hostname ?? jumpDevice) }
        else if let jumpHost { via = "ssh:" + jumpHost }
        else { via = "direct" }
        return MockSshSession(client: self, info: SshInfo(hostId: hostId, label: label, host: host, port: port, username: username, via: via))
    }

    /// 在后台线程上同步调用（SecretPrompt 会阻塞等用户回答）
    static func blocking<T>(_ body: @escaping () -> T) async -> T {
        await withCheckedContinuation { cont in
            DispatchQueue.global().async { cont.resume(returning: body()) }
        }
    }
}

final class MockSshSession: SshSession, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    let client: MockGalaxyClient
    let sshInfo: SshInfo
    let device: DeviceInfo
    private var closed = false
    lazy var fileClient = MockFileClient(client: client, platform: .linux, username: sshInfo.username)

    init(client: MockGalaxyClient, info: SshInfo) {
        self.client = client
        self.sshInfo = info
        self.device = DeviceInfo(id: "ssh-" + info.hostId, kind: .worker, platform: .linux, hostname: info.label.isEmpty ? info.host : info.label,
                                 osVersion: "Debian GNU/Linux 12", arch: "x86_64", appVersion: "", online: true, lanSeen: false,
                                 lastSeenAt: MockData.now, isSelf: false)
        super.init(noHandle: NoHandle())
    }

    override func info() -> SshInfo { sshInfo }
    override func isClosed() -> Bool { closed }
    override func disconnect() { closed = true }

    override func openTerminal(options: TerminalOptions, sink: TerminalSink) async throws -> TerminalChannel {
        await client.delay(200)
        return MockTerminalChannel(client: client, device: device, options: options, sink: sink, usernameOverride: sshInfo.username, register: false)
    }

    override func files() -> FileClient { fileClient }

    override func exec(request: ExecRequest) async throws -> ExecResult {
        await client.delay(200)
        let out = MockShell.run(command: request.command, device: device, cwd: request.cwd ?? "~", zh: client.zh)
        return ExecResult(code: out.code, stdout: out.text, stderr: "", timedOut: false, truncated: false, durationMs: 30)
    }

    override func forwardPort(localPort: UInt16, remoteHost: String, remotePort: UInt16) async throws -> PortForward {
        MockPortForward(port: localPort == 0 ? 50_000 + remotePort % 1000 : localPort)
    }
}
