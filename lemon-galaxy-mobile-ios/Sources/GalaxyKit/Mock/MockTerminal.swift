import Foundation

/// 被控端保留的终端会话（模拟）：断线后可接续。每台设备预置两条「已断开但仍在运行」的会话。
final class MockTerminalRegistry {
    static let shared = MockTerminalRegistry()
    private let lock = NSLock()
    private var items: [String: (deviceId: String, info: TerminalSessionInfo, screen: String)] = [:]
    private var seeded: Set<String> = []

    func sessions(for deviceId: String) -> [TerminalSessionInfo] {
        lock.lock(); defer { lock.unlock() }
        if !seeded.contains(deviceId) {
            seeded.insert(deviceId)
            let t = MockData.fixedNow
            let a = TerminalSessionInfo(id: "t-\(deviceId)-1", title: "htop", createdAt: t - 3_600_000 * 5, attached: false)
            let b = TerminalSessionInfo(id: "t-\(deviceId)-2", title: "tail -f /var/log/syslog", createdAt: t - 60_000 * 42, attached: false)
            items[a.id] = (deviceId, a, "htop")
            items[b.id] = (deviceId, b, "tail")
        }
        return items.values.filter { $0.deviceId == deviceId }.map(\.info).sorted { $0.createdAt < $1.createdAt }
    }

    func register(_ info: TerminalSessionInfo, deviceId: String) {
        lock.locked { items[info.id] = (deviceId, info, "") }
    }

    func setAttached(_ id: String, _ attached: Bool) {
        lock.locked { items[id]?.info.attached = attached }
    }

    func screen(_ id: String) -> String? { lock.lock(); defer { lock.unlock() }; return items[id]?.screen }

    func remove(_ id: String) { lock.locked { items[id] = nil } }
}

/// 模拟远端 shell：本地回显、行编辑、几条常用命令的假输出。全部走 TerminalSink 的原始字节，和真内核一样。
final class MockTerminalChannel: TerminalChannel, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("模拟对象不能由内核指针构造") }
    private let client: MockGalaxyClient
    private let device: DeviceInfo
    private weak var sink: TerminalSink?
    private let id: String
    private let user: String
    private var cwd = "~"
    private var line = ""
    private var history: [String] = []
    private var historyIndex = 0
    private var cols: UInt16
    private var rows: UInt16
    private var passwordMode = false
    private var escBuffer: [UInt8] = []
    private var closed = false
    private let queue = DispatchQueue(label: "mock.terminal")
    private let registered: Bool

    init(client: MockGalaxyClient, device: DeviceInfo, options: TerminalOptions, sink: TerminalSink,
         usernameOverride: String? = nil, register: Bool = true) {
        self.client = client
        self.device = device
        self.sink = sink
        self.cols = options.cols
        self.rows = options.rows
        self.user = usernameOverride ?? MockData.username(for: device)
        self.registered = register
        self.id = options.attachSession ?? "t-\(device.id)-\(UUID().uuidString.prefix(6).lowercased())"
        super.init(noHandle: NoHandle())
        if register {
            if options.attachSession == nil {
                MockTerminalRegistry.shared.register(TerminalSessionInfo(id: id, title: device.platform == .windows ? "powershell" : "bash",
                                                                         createdAt: MockData.now, attached: true), deviceId: device.id)
            } else {
                MockTerminalRegistry.shared.setAttached(id, true)
            }
        }
        let attachScreen = options.attachSession.flatMap { MockTerminalRegistry.shared.screen($0) }
        queue.async { [weak self] in
            guard let self else { return }
            if let attachScreen { self.emitAttached(attachScreen) } else { self.emitBanner() }
        }
    }

    private var isWindows: Bool { device.platform == .windows }
    private var zh: Bool { client.zh }

    private func out(_ s: String) {
        let data = Data(s.replacingOccurrences(of: "\n", with: "\r\n").utf8)
        sink?.onOutput(data: data)
    }

    private var prompt: String {
        if isWindows { return "PS C:\\Users\\\(user)\(cwd == "~" ? "" : "\\" + cwd.replacingOccurrences(of: "~/", with: ""))> " }
        let host = device.hostname.split(separator: ".").first.map(String.init) ?? device.hostname
        return "\u{1b}[1;32m\(user)@\(host)\u{1b}[0m:\u{1b}[1;34m\(cwd)\u{1b}[0m$ "
    }

    private func emitBanner() {
        if isWindows {
            out("Windows PowerShell\nCopyright (C) Microsoft Corporation. All rights reserved.\n\n")
        } else if device.platform == .macos {
            out("Last login: Fri Nov 21 09:12:44 on ttys002\n")
        } else {
            out("Welcome to \(device.osVersion) (GNU/Linux 6.8.0-49-generic \(device.arch))\n\n")
            out(" * Documentation:  https://help.ubuntu.com\n")
            out("\n  System load:  0.42               Processes:             231\n")
            out("  Usage of /:   37.8% of 196.70GB   Users logged in:       1\n")
            out("  Memory usage: 41%                 IPv4 address for eth0: 10.0.1.5\n\n")
            out("Last login: Fri Nov 21 09:12:44 2026 from 10.0.0.12\n")
        }
        out(prompt)
    }

    private func emitAttached(_ kind: String) {
        out("\u{1b}[2J\u{1b}[H")
        out(zh ? "\u{1b}[2m[已接续被控端保留的会话]\u{1b}[0m\n" : "\u{1b}[2m[Re-attached to the session kept on the remote device]\u{1b}[0m\n")
        if kind == "tail" {
            for l in MockShell.syslogLines.suffix(8) { out(l + "\n") }
        } else {
            out(MockShell.topOutput(zh: zh))
            out("\n")
            out(prompt)
        }
    }

    override func sessionId() -> String { id }
    override func title() -> String { isWindows ? "powershell" : "bash" }

    override func write(data: Data) {
        queue.async { [weak self] in self?.handle(Array(data)) }
    }

    private func handle(_ bytes: [UInt8]) {
        guard !closed else { return }
        var i = 0
        while i < bytes.count {
            let b = bytes[i]
            if !escBuffer.isEmpty || b == 0x1b {
                escBuffer.append(b)
                // ESC [ A / B / C / D：方向键
                if escBuffer.count >= 3, escBuffer[1] == 0x5b {
                    let final = escBuffer.last!
                    if (0x40...0x7e).contains(final) {
                        if final == 0x41 { historyUp() } else if final == 0x42 { historyDown() }
                        escBuffer.removeAll()
                    }
                } else if escBuffer.count >= 2, escBuffer[1] != 0x5b, escBuffer[1] != 0x4f {
                    escBuffer.removeAll()
                } else if escBuffer.count >= 3, escBuffer[1] == 0x4f {
                    escBuffer.removeAll()
                }
                i += 1
                continue
            }
            switch b {
            case 0x0d, 0x0a:
                out("\n")
                let cmd = line
                line = ""
                if passwordMode {
                    passwordMode = false
                    out(zh ? "sudo：已模拟（不会真的执行）\n" : "sudo: simulated (nothing was run)\n")
                    out(prompt)
                } else {
                    run(cmd)
                }
            case 0x7f, 0x08:
                if !line.isEmpty {
                    line.removeLast()
                    if !passwordMode { out("\u{8} \u{8}") }
                }
            case 0x03:
                out("^C\n")
                line = ""
                passwordMode = false
                out(prompt)
            case 0x04:
                if line.isEmpty { finish(0) }
            case 0x0c:
                out("\u{1b}[2J\u{1b}[H" + prompt + line)
            case 0x09:
                break
            default:
                // 收集 UTF-8 字节（中文输入法提交的文字也走这里）
                var end = i + 1
                while end < bytes.count, bytes[end] >= 0x80 || (bytes[end] >= 0x20 && bytes[end] != 0x7f) { end += 1 }
                if b >= 0x20 {
                    let chunk = String(decoding: bytes[i..<end], as: UTF8.self)
                    line += chunk
                    if !passwordMode { out(chunk) }
                    i = end
                    continue
                }
            }
            i += 1
        }
    }

    private func historyUp() {
        guard !history.isEmpty, historyIndex > 0 else { return }
        historyIndex -= 1
        replaceLine(history[historyIndex])
    }

    private func historyDown() {
        guard historyIndex < history.count else { return }
        historyIndex += 1
        replaceLine(historyIndex < history.count ? history[historyIndex] : "")
    }

    private func replaceLine(_ s: String) {
        out(String(repeating: "\u{8} \u{8}", count: line.count))
        line = s
        out(s)
    }

    private func run(_ raw: String) {
        let cmd = raw.trimmingCharacters(in: .whitespaces)
        if !cmd.isEmpty { history.append(cmd) }
        historyIndex = history.count
        if cmd.isEmpty { out(prompt); return }
        let parts = cmd.split(separator: " ").map(String.init)
        switch parts[0] {
        case "exit", "logout":
            finish(0)
            return
        case "clear", "cls":
            out("\u{1b}[2J\u{1b}[H")
        case "cd":
            let target = parts.count > 1 ? parts[1] : "~"
            if target == "~" || target == "/" { cwd = target }
            else if target == ".." { cwd = cwd.contains("/") ? String(cwd[..<cwd.lastIndex(of: "/")!]) : "~" }
            else if ["projects", "logs", "Downloads", "Documents", "app"].contains(target) { cwd = (cwd == "~" ? "~/" : cwd + "/") + target }
            else { out("bash: cd: \(target): No such file or directory\n") }
        case "sudo":
            out("[sudo] password for \(user): ")
            passwordMode = true
            return
        default:
            let r = MockShell.run(command: cmd, device: device, cwd: cwd, zh: zh)
            out(r.text)
        }
        out(prompt)
    }

    private func finish(_ code: Int32) {
        guard !closed else { return }
        closed = true
        out("logout\n")
        if registered { MockTerminalRegistry.shared.remove(id) }
        sink?.onExit(code: code)
    }

    override func resize(cols: UInt16, rows: UInt16) {
        queue.async { [weak self] in self?.cols = cols; self?.rows = rows }
    }

    override func detach() {
        closed = true
        if registered { MockTerminalRegistry.shared.setAttached(id, false) }
    }

    override func kill() {
        closed = true
        if registered { MockTerminalRegistry.shared.remove(id) }
    }
}

/// 假命令输出，终端和 AI 的 run_command 共用。
enum MockShell {
    static let syslogLines = [
        "Nov 21 09:40:01 build-server-01 CRON[48211]: (root) CMD (command -v debian-sa1 > /dev/null && debian-sa1 1 1)",
        "Nov 21 09:41:12 build-server-01 systemd[1]: Started docker-compose@ci.service - CI stack.",
        "Nov 21 09:41:13 build-server-01 dockerd[912]: time=\"2026-11-21T09:41:13\" level=info msg=\"Container started\"",
        "Nov 21 09:42:55 build-server-01 kernel: [812345.1234] EXT4-fs (sda1): re-mounted. Quota mode: none.",
        "Nov 21 09:44:02 build-server-01 sshd[49870]: Accepted publickey for deploy from 10.0.0.12 port 53122",
        "Nov 21 09:45:30 build-server-01 systemd[1]: Starting apt-daily.service - Daily apt download activities...",
        "Nov 21 09:45:41 build-server-01 systemd[1]: apt-daily.service: Deactivated successfully.",
        "Nov 21 09:47:18 build-server-01 runner[2231]: Job ci-1842 finished in 3m12s (success)",
        "Nov 21 09:48:02 build-server-01 runner[2231]: Job ci-1843 started: build linux-x86_64",
    ]

    static func topOutput(zh: Bool) -> String {
        """
        top - 09:48:12 up 41 days,  3:02,  1 user,  load average: 0.42, 0.51, 0.47
        Tasks: 231 total,   1 running, 230 sleeping,   0 stopped,   0 zombie
        %Cpu(s):  4.1 us,  1.2 sy,  0.0 ni, 94.3 id,  0.3 wa,  0.0 hi,  0.1 si
        MiB Mem :  31842.6 total,   9120.4 free,  13044.8 used,   9677.4 buff/cache
        \u{1b}[7m    PID USER      PR  NI    VIRT    RES  %CPU  %MEM     TIME+ COMMAND        \u{1b}[0m
           2231 runner    20   0 4123456 812340  12.3   2.5 812:41.22 runner
            912 root      20   0 2985612 142380   3.1   0.4 201:13.05 dockerd
          41022 deploy    20   0 1203344 401220   2.7   1.2  12:03.88 node
           1189 postgres  20   0  412332  98120   1.0   0.3  88:41.10 postgres
        """
    }

    static func run(command: String, device: DeviceInfo, cwd: String, zh: Bool) -> (text: String, code: Int32) {
        let parts = command.split(separator: " ").map(String.init)
        guard let head = parts.first else { return ("", 0) }
        let windows = device.platform == .windows
        switch head {
        case "ls", "dir", "Get-ChildItem":
            if windows {
                return ("""

                    Directory: C:\\Users\\alice

                Mode                 LastWriteTime         Length Name
                ----                 -------------         ------ ----
                d-r---        2026/11/18     10:21                Desktop
                d-r---        2026/11/20     17:02                Documents
                d-r---        2026/11/21      9:15                Downloads
                -a----        2026/11/19     14:40          18244 report.xlsx

                """, 0)
            }
            if parts.contains("-la") || parts.contains("-l") || parts.contains("-al") {
                return ("""
                total 48
                drwxr-x--- 7 deploy deploy 4096 Nov 21 09:12 \u{1b}[1;34m.\u{1b}[0m
                drwxr-xr-x 4 root   root   4096 Oct  2 11:40 \u{1b}[1;34m..\u{1b}[0m
                -rw------- 1 deploy deploy 8214 Nov 21 09:12 .bash_history
                drwx------ 2 deploy deploy 4096 Oct  2 11:41 \u{1b}[1;34m.ssh\u{1b}[0m
                drwxr-xr-x 5 deploy deploy 4096 Nov 20 18:22 \u{1b}[1;34mapp\u{1b}[0m
                -rw-r--r-- 1 deploy deploy 1532 Nov 19 10:04 docker-compose.yml
                drwxr-xr-x 2 deploy deploy 4096 Nov 21 09:40 \u{1b}[1;34mlogs\u{1b}[0m
                -rwxr-xr-x 1 deploy deploy  642 Nov 12 16:30 \u{1b}[1;32mdeploy.sh\u{1b}[0m
                drwxr-xr-x 6 deploy deploy 4096 Nov 18 21:03 \u{1b}[1;34mprojects\u{1b}[0m
                -rw-r--r-- 1 deploy deploy  220 Oct  2 11:40 README.md

                """, 0)
            }
            return ("\u{1b}[1;34mapp\u{1b}[0m  \u{1b}[1;32mdeploy.sh\u{1b}[0m  docker-compose.yml  \u{1b}[1;34mlogs\u{1b}[0m  \u{1b}[1;34mprojects\u{1b}[0m  README.md\n", 0)
        case "pwd":
            return ((cwd == "~" ? "/home/\(MockData.username(for: device))" : cwd.replacingOccurrences(of: "~", with: "/home/\(MockData.username(for: device))")) + "\n", 0)
        case "whoami":
            return (MockData.username(for: device) + "\n", 0)
        case "hostname":
            return (device.hostname + "\n", 0)
        case "uname":
            return ("Linux \(device.hostname) 6.8.0-49-generic #49-Ubuntu SMP PREEMPT_DYNAMIC x86_64 GNU/Linux\n", 0)
        case "date":
            return ("Fri Nov 21 09:48:12 CST 2026\n", 0)
        case "uptime":
            return (" 09:48:12 up 41 days,  3:02,  1 user,  load average: 0.42, 0.51, 0.47\n", 0)
        case "echo":
            return (parts.dropFirst().joined(separator: " ") + "\n", 0)
        case "df":
            return ("""
            Filesystem      Size  Used Avail Use% Mounted on
            /dev/sda1       197G   75G  113G  40% /
            tmpfs            16G     0   16G   0% /dev/shm
            /dev/sdb1       1.8T  1.6T  142G  92% /data
            /dev/sda15      105M  6.1M   99M   6% /boot/efi

            """, 0)
        case "free":
            return ("""
                           total        used        free      shared  buff/cache   available
            Mem:            31Gi        12Gi       8.9Gi       412Mi       9.4Gi        18Gi
            Swap:          4.0Gi       128Mi       3.9Gi

            """, 0)
        case "du":
            return ("""
            1.1T\t/data/backups
            312G\t/data/docker
            98G\t/data/datasets
            41G\t/data/logs

            """, 0)
        case "top", "htop":
            return (topOutput(zh: zh) + "\n", 0)
        case "journalctl", "tail":
            return (syslogLines.joined(separator: "\n") + "\n", 0)
        case "docker":
            return ("""
            NAMES          STATUS                   PORTS
            ci-runner      Up 6 days
            postgres       Up 6 days (healthy)      5432/tcp
            nginx          Up 2 hours               0.0.0.0:80->80/tcp, 0.0.0.0:443->443/tcp
            redis          Exited (137) 3 hours ago

            """, 0)
        case "cat":
            if parts.count > 1, parts[1].hasSuffix("README.md") {
                return ("# build-server-01\n\nCI runner and artifact cache.\n", 0)
            }
            if parts.count > 1, parts[1].hasSuffix("docker-compose.yml") {
                return ("services:\n  nginx:\n    image: nginx:1.27\n    ports: [\"80:80\", \"443:443\"]\n  redis:\n    image: redis:7\n", 0)
            }
            return ("cat: \(parts.count > 1 ? parts[1] : ""): No such file or directory\n", 1)
        case "colors":
            var s = ""
            for c in 0..<8 { s += "\u{1b}[\(30 + c)m\u{2588}\u{2588}\u{2588} \u{1b}[0m" }
            s += "\n"
            for c in 0..<8 { s += "\u{1b}[\(90 + c)m\u{2588}\u{2588}\u{2588} \u{1b}[0m" }
            s += "\n"
            return (s, 0)
        case "help":
            return ((zh ? "模拟终端可用命令：" : "Commands available in the mock terminal: ") +
                    "ls, ls -la, cd, pwd, whoami, hostname, uname, date, uptime, df, free, du, top, journalctl, docker, cat, echo, colors, clear, sudo, exit\n", 0)
        case "systemctl":
            return ("● nginx.service - A high performance web server\n     Loaded: loaded (/lib/systemd/system/nginx.service; enabled)\n     Active: \u{1b}[1;32mactive (running)\u{1b}[0m since Fri 2026-11-21 07:41:02 CST; 2h 7min ago\n", 0)
        case "rm", "find":
            return ("", 0)
        default:
            if windows { return ("\(head) : The term '\(head)' is not recognized as the name of a cmdlet.\n", 1) }
            return ("bash: \(head): command not found\n", 127)
        }
    }
}
