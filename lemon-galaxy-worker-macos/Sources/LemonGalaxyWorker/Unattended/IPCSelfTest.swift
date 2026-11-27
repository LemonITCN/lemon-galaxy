import Foundation
import Security

/// --ipc-selftest [--bundle 应用路径]
/// 不用 root、不登记任何系统服务：在一个进程里起一个后台服务（模拟内核、临时目录里的套接字与数据），
/// 接上两个假会话代理（登录界面 + 用户会话，平台层是假的）与一个界面（RemoteWorkerHost），检查：
///   消息分帧与大附件、平台层转发（画面帧、键鼠顺序、剪贴板、截图、声音、隐私屏）、会话切换后画面续上、
///   代理断开与顶替、版本不一致时让代理重启、界面的账号 / 协助 / 事件 / 更新下载、权限判断、移交导入导出、
///   会话代理的 launchd 配置；给了 --bundle 时再检查包内 launchd 配置与签名。
/// 有失败时退出码为 1。
enum IPCSelfTest {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var passed = 0
    nonisolated(unsafe) private static var failed: [String] = []

    static func check(_ name: String, _ ok: Bool, _ detail: @autoclosure () -> String = "") {
        lock.locked { if ok { passed += 1 } else { failed.append(name) } }
        let d = detail()
        print("\(ok ? "[通过]" : "[失败]") \(name)\(d.isEmpty ? "" : "：" + d)")
    }

    @discardableResult
    static func wait(_ timeout: Double = 5, _ cond: () -> Bool) -> Bool {
        let end = Date().addingTimeInterval(timeout)
        while Date() < end {
            if cond() { return true }
            Thread.sleep(forTimeInterval: 0.02)
        }
        return cond()
    }

    static func run(_ args: [String]) -> Int32 {
        setvbuf(stdout, nil, _IOLBF, 0)
        let sem = DispatchSemaphore(value: 0)
        Task.detached {
            await body(args)
            sem.signal()
        }
        sem.wait()
        print("\nipc-selftest：通过 \(passed) 项，失败 \(failed.count) 项" + (failed.isEmpty ? "" : "：" + failed.joined(separator: "、")))
        return failed.isEmpty ? 0 : 1
    }

    private static func body(_ args: [String]) async {
        let tmp = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("lg-ipc-\(getpid())", isDirectory: true)
        try? FileManager.default.removeItem(at: tmp)
        try? FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmp) }
        let sock = tmp.appendingPathComponent("d.sock").path
        print("临时目录 \(tmp.path)")
        print("本进程签名要求：\(CodeIdentity.ownRequirement == nil ? "（无签名，跳过签名核对）" : CodeIdentity.ownRequirementText)")

        basics(tmp)

        let restarts = Counter()
        var o = DaemonService.Options()
        o.socketPath = sock
        o.dataRoot = tmp.appendingPathComponent("data", isDirectory: true)
        o.cacheDir = tmp.appendingPathComponent("cache", isDirectory: true)
        o.mock = true
        o.installAgents = false
        o.checkAgentUid = false
        o.restart = { restarts.add() }
        // 自动更新的替换目标放在临时目录（不是应用包：「立即更新」应被拒绝，什么都不动）
        o.updateTarget = tmp.appendingPathComponent("Apps/NotAnApp.app", isDirectory: true)
        let svc = DaemonService(o, locale: "zh-CN")
        let console = Box<uid_t?>(nil)
        svc.platform.consoleUid = { console.value }
        do { try svc.start() } catch { check("后台服务启动", false, "\(error)"); return }
        check("后台服务启动并监听套接字", FileManager.default.fileExists(atPath: sock))
        check("套接字权限 0666（连上后按身份授权）", (try? FileManager.default.attributesOfItem(atPath: sock)[.posixPermissions] as? Int) == 0o666)

        malformed(sock, svc)
        foreignBinary(sock, svc, tmp)
        let (fa, ca) = platformForwarding(sock, svc)
        consoleSwitch(sock, svc, fa, ca, console)
        await control(sock, svc, tmp)
        migration(svc, tmp, restarts)
        ca.stop()
        svc.stop()
    }

    // MARK: 基础

    private static func basics(_ tmp: URL) {
        check("套接字路径过长时拒绝", IPCSocket.address(String(repeating: "x", count: 200)) == nil)
        check("相对路径校验", Migration.safeRelative("host.json") && Migration.safeRelative("a/b.log")
              && !Migration.safeRelative("../x") && !Migration.safeRelative("/etc/passwd") && !Migration.safeRelative("a/../b") && !Migration.safeRelative(""))

        // 内核类型编码往返
        let st = HostStatus(online: true, remoteEnabled: false, deviceId: "abc", publicIp: "203.0.113.7",
                            accounts: [HostAccount(userId: 7, username: "alice", displayName: "Alice", status: .needsRelogin,
                                                   policy: HostPolicy(systemUser: "alice", view: true, control: false, files: true, terminal: false,
                                                                      clipboard: true, audio: false, aiCap: .confirmDangerous, notify: true))],
                            sessions: [HostSessionInfo(id: "s1", fromUser: "bob", fromDevice: "PC", mode: .assist, path: .relayTcp,
                                                       startedAt: 123, aiActive: true, accountId: nil)],
                            assist: AssistInfo(deviceCode: "1", code: "2"))
        let back = try? IPCJSON.decoder.decode(HostStatus.self, from: IPCJSON.encode(st))
        check("被控状态编码往返", back == st)
        let evs: [InputEvent] = [.mouseMove(x: 1, y: -2), .mouseButton(button: .back, down: true, x: 3, y: 4), .wheel(dx: -5, dy: 6),
                                 .key(hidUsage: 0x04, down: false), .text(text: "中文 ok"), .touch(id: 9, phase: .cancelled, x: 1, y: 1),
                                 .specialCombo(combo: .ctrlAltDel)]
        check("输入事件编码往返", (try? IPCJSON.decoder.decode([InputEvent].self, from: IPCJSON.encode(evs))) == evs)
        let clips: [ClipboardData] = [.text(text: "a"), .files(paths: ["/tmp/x"]), .image(image: ImageData(mime: "image/png", data: Data([1, 2, 3]), width: 1, height: 1))]
        check("剪贴板编码往返", (try? IPCJSON.decoder.decode([ClipboardData].self, from: IPCJSON.encode(clips))) == clips)
        let offer = ClipOfferInfo(id: UInt64.max - 12_345, entries: [ClipOfferEntry(path: "目录", isDir: true, size: 0, mtime: -1),
                                                                    ClipOfferEntry(path: "目录/a b.txt", isDir: false, size: 1 << 40, mtime: 1_795_000_000)],
                                  totalBytes: 1 << 40)
        check("文件清单编码往返（大 id 不丢精度）", (try? IPCJSON.decoder.decode(ClipOfferInfo.self, from: IPCJSON.encode(offer))) == offer)
        let lr: [WorkerLoginResult] = [.success(userId: 3, username: "x"), .needTotp, .mustChangePassword]
        check("登录结果编码往返", (try? IPCJSON.decoder.decode([WorkerLoginResult].self, from: IPCJSON.encode(lr))) == lr)
        let f = IPCFault(GalaxyError.Auth(code: "bad_credentials", detail: "密码错误"))
        check("内核错误跨进程还原", f.galaxyError == GalaxyError.Auth(code: "bad_credentials", detail: "密码错误"))

        // 权限判断
        let pol = { (u: String) in HostPolicy(systemUser: u, view: true, control: true, files: true, terminal: true, clipboard: true, audio: true, aiCap: .fullTrust, notify: true) }
        let stranger = PeerAuth(uid: 502, userName: "eve", isAdmin: false, isConsole: false, codeOK: true)
        let consoleUser = PeerAuth(uid: 501, userName: "alice", isAdmin: false, isConsole: true, codeOK: true)
        let admin = PeerAuth(uid: 503, userName: "root-ish", isAdmin: true, isConsole: false, codeOK: true)
        let forged = PeerAuth(uid: 503, userName: "root-ish", isAdmin: true, isConsole: true, codeOK: false)
        check("权限：既不是管理员也不在屏幕前的用户被拒绝", !stranger.allowed)
        check("权限：屏幕前的普通用户可用，但只能管对应到自己的账号", consoleUser.allowed && consoleUser.mayManage(pol("alice")) && !consoleUser.mayManage(pol("bob")))
        check("权限：普通用户不能把账号改对应到别的系统用户", !consoleUser.mayChange(pol("alice"), to: pol("root")))
        check("权限：管理员都可以", admin.allowed && admin.mayManage(pol("bob")) && admin.mayChange(pol("alice"), to: pol("bob")))
        check("权限：签名不一致一律不放行", !forged.allowed)
        check("权限：root 后台服务里只有管理员能开启或允许临时协助", !consoleUser.mayOperateAssist(daemonIsRoot: true)
              && admin.mayOperateAssist(daemonIsRoot: true) && consoleUser.mayOperateAssist(daemonIsRoot: false))

        // 会话代理的 launchd 配置
        let exe = "/Applications/LemonGalaxyWorker.app/Contents/MacOS/LemonGalaxyWorker"
        let dict = AgentPlist.make(executable: exe, bundleId: AppInfo.bundleId)
        if let d = AgentPlist.data(dict), let p = try? PropertyListSerialization.propertyList(from: d, format: nil) as? [String: Any] {
            check("会话代理配置：登录界面与用户会话", (p["LimitLoadToSessionType"] as? [String]).map { Set($0) } == ["LoginWindow", "Aqua"])
            check("会话代理配置：程序路径与参数", (p["ProgramArguments"] as? [String]) == [exe, "--session-agent"])
            check("会话代理配置：关联应用", (p["AssociatedBundleIdentifiers"] as? [String]) == [AppInfo.bundleId])
            check("会话代理配置：出错重启、正常退出不重启", ((p["KeepAlive"] as? [String: Any])?["SuccessfulExit"] as? Bool) == false)
        } else {
            check("会话代理配置可序列化", false)
        }
        let path = tmp.appendingPathComponent("agent.plist").path
        let first = AgentPlist.install(executable: exe, bundleId: AppInfo.bundleId, path: path)
        let second = AgentPlist.install(executable: exe, bundleId: AppInfo.bundleId, path: path)
        let third = AgentPlist.install(executable: exe + "2", bundleId: AppInfo.bundleId, path: path)
        check("会话代理配置：没变化不改写，路径变了才改写", first && !second && third)

        // 文件型安全存储
        let store = FileSecureStore(dir: tmp.appendingPathComponent("keys-test"))
        store.set(key: "worker.device_id", value: Data([1, 2, 3]))
        let mode = (try? FileManager.default.attributesOfItem(atPath: tmp.appendingPathComponent("keys-test/worker.device_id").path)[.posixPermissions] as? Int)
        check("安全存储：读写与 0600 权限", store.get(key: "worker.device_id") == Data([1, 2, 3]) && mode == 0o600)
        store.remove(key: "worker.device_id")
        check("安全存储：删除", store.get(key: "worker.device_id") == nil)

        if let i = CommandLine.arguments.firstIndex(of: "--bundle"), i + 1 < CommandLine.arguments.count {
            bundleChecks(URL(fileURLWithPath: CommandLine.arguments[i + 1]))
        } else {
            // 没给包：检查源码里的 Packaging 配置
            var dir = URL(fileURLWithPath: #filePath)
            for _ in 0..<4 { dir.deleteLastPathComponent() }
            bundleChecks(nil, packaging: dir.appendingPathComponent("Packaging"))
        }
    }

    private static func bundleChecks(_ app: URL?, packaging: URL? = nil) {
        let daemonPlist = app.map { $0.appendingPathComponent("Contents/Library/LaunchDaemons/\(Unattended.daemonPlistName)") }
            ?? packaging!.appendingPathComponent("LaunchDaemons/\(Unattended.daemonPlistName)")
        let agentPlist = app.map { $0.appendingPathComponent("Contents/Library/LaunchAgents/\(Unattended.agentPlistName)") }
            ?? packaging!.appendingPathComponent("LaunchAgents/\(Unattended.agentPlistName)")
        let where_ = app == nil ? "Packaging" : "包内"
        guard let dd = FileManager.default.contents(atPath: daemonPlist.path),
              let d = try? PropertyListSerialization.propertyList(from: dd, format: nil) as? [String: Any] else {
            check("\(where_)后台服务配置存在", false, daemonPlist.path)
            return
        }
        check("\(where_)后台服务配置：Label", d["Label"] as? String == Unattended.daemonLabel)
        check("\(where_)后台服务配置：BundleProgram", d["BundleProgram"] as? String == "Contents/MacOS/LemonGalaxyWorker")
        check("\(where_)后台服务配置：--daemon", (d["ProgramArguments"] as? [String])?.contains("--daemon") == true)
        check("\(where_)后台服务配置：关联应用", (d["AssociatedBundleIdentifiers"] as? [String])?.contains(AppInfo.bundleId) == true)
        check("\(where_)后台服务配置：常驻", d["KeepAlive"] as? Bool == true && d["RunAtLoad"] as? Bool == true)
        if let ad = FileManager.default.contents(atPath: agentPlist.path), let a = try? PropertyListSerialization.propertyList(from: ad, format: nil) as? [String: Any] {
            check("\(where_)会话代理模板：Label", a["Label"] as? String == Unattended.agentLabel)
            check("\(where_)会话代理模板：登录界面与用户会话", (a["LimitLoadToSessionType"] as? [String]).map { Set($0) } == ["LoginWindow", "Aqua"])
            check("\(where_)会话代理模板：--session-agent", (a["ProgramArguments"] as? [String])?.contains("--session-agent") == true)
        } else {
            check("\(where_)会话代理模板存在", false, agentPlist.path)
        }
        guard let app else { return }
        let exe = app.appendingPathComponent("Contents/MacOS/LemonGalaxyWorker")
        check("包内主程序存在（BundleProgram 指向它）", FileManager.default.isExecutableFile(atPath: exe.path))
        let info = NSDictionary(contentsOf: app.appendingPathComponent("Contents/Info.plist"))
        check("包的 Bundle ID", info?["CFBundleIdentifier"] as? String == AppInfo.bundleId)
        var st: SecStaticCode?
        if SecStaticCodeCreateWithPath(app as CFURL, [], &st) == errSecSuccess, let st {
            let flags = SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate | kSecCSCheckNestedCode)
            let r = SecStaticCodeCheckValidity(st, flags, nil)
            check("包签名有效（含 Contents/Library 下的 launchd 配置）", r == errSecSuccess, "状态 \(r)")
            var dict: CFDictionary?
            if SecCodeCopySigningInformation(st, SecCSFlags(rawValue: kSecCSSigningInformation), &dict) == errSecSuccess,
               let i = dict as? [String: Any] {
                let team = i[kSecCodeInfoTeamIdentifier as String] as? String
                print("    签名：标识 \(i[kSecCodeInfoIdentifier as String] as? String ?? "?")，团队 \(team ?? "（临时签名）")")
                check("签名标识与 Bundle ID 一致", i[kSecCodeInfoIdentifier as String] as? String == AppInfo.bundleId)
            }
        } else {
            check("读取包签名", false)
        }
    }

    // MARK: 分帧

    private static func malformed(_ sock: String, _ svc: DaemonService) {
        guard let fd = try? IPCSocket.connect(path: sock) else { check("连接后台服务", false); return }
        // 头长 0、正文 4 GB：后台服务应断开，不崩溃
        var bad = [UInt8](repeating: 0, count: 12)
        bad[4] = 0xFF; bad[5] = 0xFF; bad[6] = 0xFF; bad[7] = 0xFF
        _ = bad.withUnsafeBytes { Darwin.write(fd, $0.baseAddress, 12) }
        var buf = [UInt8](repeating: 0, count: 16)
        var tv = timeval(tv_sec: 3, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        let n = Darwin.read(fd, &buf, 16)
        check("不合法的消息长度：后台服务断开连接", n == 0)
        Darwin.close(fd)
        // 不发 hello 的连接 10 秒后被断开（这里不等），先发未知请求会被拒绝
        if let fd2 = try? IPCSocket.connect(path: sock) {
            let c = IPCConnection(fd: fd2)
            c.start()
            do {
                _ = try c.call(IPCMethod.status, IPCEmpty(), timeout: 3)
                check("没打招呼的请求被拒绝", false)
            } catch {
                check("没打招呼的请求被拒绝", true)
            }
            c.close()
        }
    }

    /// 别的程序（这里用系统自带的 nc）发会话代理的 hello：签名对不上，后台服务应拒绝
    private static func foreignBinary(_ sock: String, _ svc: DaemonService, _ tmp: URL) {
        guard CodeIdentity.ownRequirement != nil, FileManager.default.isExecutableFile(atPath: "/usr/bin/nc") else {
            print("（跳过：本进程没有签名或没有 /usr/bin/nc）")
            return
        }
        let hello = AgentHello(sessionType: .loginWindow, uid: getuid(), userName: "loginwindow", sessionId: 4242, pid: 1, onConsole: true,
                               version: AppInfo.version, build: AppInfo.build, capabilities: [])
        let header = IPCJSON.encode(IPCConnection.Header(k: .notify, i: 0, m: IPCMethod.agentHello, err: nil))
        let body = IPCJSON.encode(hello)
        var frame = Data()
        for n in [header.count, body.count, 0] { withUnsafeBytes(of: UInt32(n).littleEndian) { frame.append(contentsOf: $0) } }
        frame.append(header)
        frame.append(body)
        let file = tmp.appendingPathComponent("forged.bin")
        try? frame.write(to: file)
        // 发完 hello 后保持连接（stdin 不立刻结束），否则连接先断、消息来不及处理
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/sh")
        p.arguments = ["-c", "(cat \"$0\"; sleep 4) | /usr/bin/nc -U \"$1\"", file.path, sock]
        let out = Pipe()
        p.standardOutput = out
        p.standardError = FileHandle.nullDevice
        let before = svc.platform.rejectedAgents
        guard (try? p.run()) != nil else { check("启动 nc", false); return }
        let rejected = wait(5) { svc.platform.rejectedAgents > before }
        let registered = !svc.platform.summaries().isEmpty
        if p.isRunning { p.terminate() }
        p.waitUntilExit()
        let reply = out.fileHandleForReading.readDataToEndOfFile()
        check("别的程序（nc）冒充会话代理被拒绝（签名核对）", rejected && !registered)
        // 应用更新后还是旧程序的代理（旧包已删、签名对不上）也走这里：要让它退出、由 launchd 用新程序重启
        check("签名对不上的代理被要求退出重启", reply.range(of: Data(IPCMethod.agentExit.utf8)) != nil)
    }

    // MARK: 平台层转发

    private static func platformForwarding(_ sock: String, _ svc: DaemonService) -> (FakePlatform, AgentClient) {
        let fa = FakePlatform(tag: 0xA1)
        let sessA = Box(AgentSession(type: .loginWindow, uid: 0, userName: "loginwindow", sessionId: 100, onConsole: true))
        let ca = makeClient(sock, fa, sessA)
        ca.start()
        check("登录界面的代理连上并负责画面", wait { svc.platform.activeAgent?.sessionId == 100 })
        let dp = svc.platform

        check("列出屏幕", dp.listDisplays() == fa.displays)
        check("能力列表来自代理", dp.capabilities() == fa.caps)

        let sink = TestSink()
        let opts = CaptureOptions(maxFps: 60, bitrateKbps: 4000, codecs: [.h264])
        let started = dp.startCapture(displayId: 1, options: opts, sink: sink)
        check("开始采集返回分辨率与编码", started == CaptureStarted(width: 1920, height: 1080, codec: .h264))
        check("画面帧到达内核", wait { sink.count(tag: 0xA1) >= 30 })
        check("画面帧顺序与内容完整", sink.inOrder(tag: 0xA1))
        fa.pushBig(display: 1, size: 6 << 20)
        check("6 MB 关键帧完整到达", wait(8) { sink.bigOK })
        dp.setBitrate(displayId: 1, kbps: 2500)
        dp.requestKeyframe(displayId: 1)
        check("码率与关键帧请求转给代理", wait { fa.bitrates == [2500] && fa.keyframes >= 1 })
        dp.setCursorInVideo(displayId: 1, show: false)
        check("画面里不画光标转给代理、光标位置回到内核", wait { fa.cursorInVideo[1] == false && sink.cursors.last == CursorReport(x: 0xA1, y: 7, visible: true) })

        var sent: [InputEvent] = []
        for i in 0..<300 { sent.append(.mouseMove(x: Int32(i), y: Int32(i * 2))) }
        sent.append(.key(hidUsage: 0x04, down: true))
        sent.append(.key(hidUsage: 0x04, down: false))
        sent.append(.text(text: "登录密码 Pa55"))
        for e in sent { dp.inject(displayId: 1, event: e) }
        check("键鼠事件全部到达且不乱序", wait { fa.injected.count == sent.count } && fa.injected == sent)

        dp.clipboardSet(data: .text(text: "剪贴板"))
        check("写剪贴板", wait { fa.clipboard == .text(text: "剪贴板") })
        clipOffer(dp, fa)
        let img = ImageData(mime: "image/png", data: Data((0..<200_000).map { UInt8($0 % 251) }), width: 10, height: 20)
        fa.clipboard = .image(image: img)
        check("读剪贴板（图片）", dp.clipboardGet() == .image(image: img))
        check("截图走附件完整到达", dp.screenshot(displayId: 1, maxWidth: 800, maxHeight: 600) == fa.shot)
        check("控件树", dp.uiTree(displayId: 1, maxNodes: 10) == "{\"tag\":161}")
        check("AI 应用操作（文字）", dp.computerUse(requestJson: "{\"action\":\"list_apps\"}")?.text == "cu:161:{\"action\":\"list_apps\"}")
        check("AI 应用操作（附图）", dp.computerUse(requestJson: "shot")?.image == fa.shot)
        check("打开隐私屏", dp.setPrivacy(on: true) && fa.privacy)
        let audio = TestAudio()
        check("开始采集声音", dp.startAudio(sink: audio))
        check("声音样本到达", wait { audio.bytes >= 3 * 3840 && audio.lastTag == 0xA1 })

        // 虚拟屏：转给负责画面的代理，由代理建、代理持有
        check("虚拟屏上限来自代理的能力列表", dp.virtualDisplayMax() == 3)
        fa.needsHeadless = true
        check("有没有亮着的显示器由代理判断", dp.needsHeadlessDisplay())
        fa.needsHeadless = false
        check("有亮着的显示器时不要兜底屏", !dp.needsHeadlessDisplay())
        let vr = dp.addVirtualDisplay(width: 2560, height: 1440, refresh: 60)
        check("加虚拟屏转给代理", vr.display?.width == 2560 && vr.display?.isVirtual == true && fa.virtualIds == [vr.display?.id ?? 0],
              vr.error ?? "")
        check("虚拟屏 id 按代理对账", dp.virtualDisplayIds() == fa.virtualIds)
        let bad = dp.addVirtualDisplay(width: 1, height: 1, refresh: 60)
        check("加虚拟屏失败时带回代理的原因", bad.display == nil && bad.error?.contains("假平台拒绝") == true, bad.error ?? "")
        check("删虚拟屏", dp.removeVirtualDisplay(displayId: vr.display?.id ?? 0) == nil && fa.virtualIds.isEmpty)
        // 留一块给换会话的检查（旧代理离开屏幕时要删掉它）
        _ = dp.addVirtualDisplay(width: 1920, height: 1080, refresh: 60)
        return (fa, ca)
    }

    /// 粘贴时才传：清单转给代理；代理粘贴时经后台服务打开、读（从中间续读）、关闭主控端的内容；停止时阻塞的读随之返回
    private static func clipOffer(_ dp: DaemonPlatform, _ fa: FakePlatform) {
        let content = Data((0..<300_000).map { UInt8($0 % 241) })
        let offer = ClipOfferInfo(id: 0xFEED_FACE_CAFE_BEEF,
                                  entries: [ClipOfferEntry(path: "dir", isDir: true, size: 0, mtime: 0),
                                            ClipOfferEntry(path: "dir/a.bin", isDir: false, size: UInt64(content.count), mtime: 5),
                                            ClipOfferEntry(path: "held.bin", isDir: false, size: UInt64(content.count), mtime: 5)],
                                  totalBytes: UInt64(content.count * 2))
        let src = TestClipSource([nil, content, content], holdIndex: 2)
        check("清单转给代理", dp.clipboardSetOffer(offer: offer, source: src) == nil && fa.offer?.0 == offer)
        guard let fwd = fa.offer?.1 else { return }
        let r = try? fwd.open(index: 1, offset: 1000)
        check("代理经后台服务打开（从中间续读）", r?.size() == UInt64(content.count) && src.opened.last == [1, 1000])
        var got = Data()
        while let d = try? r?.read(max: 65_536), !d.isEmpty { got += d }
        check("代理读到的内容完整", got == content.subdata(in: 1000..<content.count), "\(got.count)")
        r?.cancel()
        check("关闭转到后台服务", wait { src.closedCount == 1 })
        var openErr = ""
        do { _ = try fwd.open(index: 0, offset: 0) } catch { openErr = String(describing: error) }
        check("打开文件夹项带回原因", openErr.contains("不是文件"), openErr)
        // 停止：读阻塞着时关掉，读随之报错返回
        let held = try? fwd.open(index: 2, offset: 0)
        let first = try? held?.read(max: 65_536)
        let blocked = Box<Bool?>(nil)
        DispatchQueue.global().async {
            do { _ = try held?.read(max: 65_536); blocked.value = false } catch { blocked.value = true }
        }
        Thread.sleep(forTimeInterval: 0.2)
        check("读到停住的地方", first?.count == 1000 && blocked.value == nil)
        held?.cancel()
        check("停止时阻塞的读报错返回", wait { blocked.value == true })
        fa.offerError = "假平台拒绝清单"
        check("代理拒绝时带回原因", dp.clipboardSetOffer(offer: offer, source: src) == "假平台拒绝清单")
        fa.offerError = nil
    }

    private static func makeClient(_ sock: String, _ fp: FakePlatform, _ sess: Box<AgentSession>) -> AgentClient {
        let c = AgentClient(socketPath: sock, platform: fp, session: { sess.value }, capabilities: { fp.caps })
        c.keyframe = { fp.requestKeyframe(displayId: $0) }
        // 不真退出：记下来并停掉这个代理（否则它会重连回来）
        c.exitHandler = { [weak c] restart in
            fp.exitRequests.append(restart)
            DispatchQueue.global().async { c?.stop() }
        }
        fp.session = sess
        return c
    }

    // MARK: 会话切换

    private static func consoleSwitch(_ sock: String, _ svc: DaemonService, _ fa: FakePlatform, _ ca: AgentClient, _ console: Box<uid_t?>) {
        let dp = svc.platform
        let fb = FakePlatform(tag: 0xB2)
        let sessB = Box(AgentSession(type: .aqua, uid: 501, userName: "alice", sessionId: 200, onConsole: false))
        let cb = makeClient(sock, fb, sessB)
        cb.start()
        check("用户会话的代理连上（不在屏幕上时不接手）", wait { dp.summaries().count == 2 } && dp.activeAgent?.sessionId == 100)

        // 用户登录：登录界面离开屏幕、用户会话到屏幕上
        fa.session?.value.onConsole = false
        fb.session?.value.onConsole = true
        console.value = 501
        ca.consoleMayHaveChanged()
        cb.consoleMayHaveChanged()
        check("登录后画面改由用户会话负责", wait { dp.activeAgent?.sessionId == 200 })
        check("新会话自动重开画面", wait { fb.capturing.contains(1) })
        check("新会话自动重开隐私屏与声音", wait { fb.privacy && fb.audioOn })
        check("旧会话停掉采集与隐私屏", wait { fa.capturing.isEmpty && !fa.privacy && !fa.audioOn })
        check("旧会话删掉自己加的虚拟屏", wait { fa.virtualIds.isEmpty })
        check("换会话后虚拟屏按新代理对账", dp.virtualDisplayIds().isEmpty)
        let sink = (fb.sinkFor(1) as? ForwardingFrameSink)
        check("新会话的画面帧到达同一个内核输出", wait { (dp.debugSink(1) as? TestSink)?.count(tag: 0xB2) ?? 0 >= 10 }, sink == nil ? "没有转发对象" : "")
        check("换会话后告诉内核新分辨率", (dp.debugSink(1) as? TestSink)?.resolutions.contains { $0 == (2560, 1600) } == true)
        check("换会话后新代理同样不画光标、位置照常回来",
              wait { fb.cursorInVideo[1] == false && (dp.debugSink(1) as? TestSink)?.cursors.last == CursorReport(x: 0xB2, y: 7, visible: true) })
        let before = fa.injected.count
        dp.inject(displayId: 1, event: .mouseMove(x: 5, y: 5))
        check("键鼠改发给用户会话", wait { fb.injected.last == .mouseMove(x: 5, y: 5) } && fa.injected.count == before)
        check("界面看到两个代理与负责画面的会话", dp.summaries().filter(\.active).map(\.userName) == ["alice"])

        // 用户注销：用户会话代理断开，登录界面回到屏幕
        cb.stop()
        console.value = nil
        fa.session?.value.onConsole = true
        ca.consoleMayHaveChanged()
        check("注销后回到登录界面的代理", wait { dp.activeAgent?.sessionId == 100 && dp.summaries().count == 1 })
        check("登录界面重新开始采集", wait { fa.capturing.contains(1) })

        // 同一会话里又来一个代理：旧的正常退出，新的接手
        let fa2 = FakePlatform(tag: 0xA3)
        let ca2 = makeClient(sock, fa2, Box(AgentSession(type: .loginWindow, uid: 0, userName: "loginwindow", sessionId: 100, onConsole: true)))
        ca2.start()
        check("同一会话的新代理顶替旧代理", wait { fa.exitRequests == [false] } && wait { dp.summaries().count == 1 && fa2.capturing.contains(1) })
        ca.stop()

        // 版本不同的代理：让它退出、由 launchd 用新程序重启
        dp.expectedBuild = "different-build"
        let fx = FakePlatform(tag: 0xC4)
        let cx = makeClient(sock, fx, Box(AgentSession(type: .aqua, uid: 502, userName: "bob", sessionId: 300, onConsole: false)))
        cx.start()
        check("版本不同的代理被要求重启", wait { fx.exitRequests == [true] })
        cx.stop()
        dp.expectedBuild = AppInfo.build
        ca2.stop()
        check("代理全部断开后没有负责画面的会话", wait { dp.activeAgent == nil })
        dp.stopCapture(displayId: 1)
        dp.stopAudio()
        _ = dp.setPrivacy(on: false)
        _ = sink
    }

    // MARK: 界面

    private static func control(_ sock: String, _ svc: DaemonService, _ tmp: URL) async {
        let r = RemoteWorkerHost(socketPath: sock)
        let infos = Box<[DaemonInfo?]>([])
        r.onInfo = { i in infos.mutate { $0.append(i) } }
        let listener = TestListener()
        r.remote.setListener(listener: listener)
        do {
            let s = try await r.start()
            check("界面连上后台服务", r.isReady && s == nil, "还没有服务器时没有被控服务")
            check("设备号来自后台服务", r.deviceId() == svc.worker.deviceId())
            check("界面是管理员（与后台服务同一用户）", r.info?.peerAdmin == true)
            let s2 = try await r.setServer(server: MockWorkerHost.demoServer)
            check("设置服务器后有被控服务", s2 != nil && r.server() == MockWorkerHost.demoServer)
            let lr = try await r.login(server: MockWorkerHost.demoServer, username: "alice", password: "password", totp: nil, newPassword: nil, systemUser: "root")
            check("登录账号", { if case .success = lr { return true }; return false }())
            check("状态推送到界面", wait { listener.lastStatus?.accounts.count == 1 })
            let acct = r.remote.accounts().first
            check("账号对应到发起登录的用户（不是 root）", acct?.policy.systemUser == NSUserName(), acct?.policy.systemUser ?? "")
            if let a = acct {
                var p = a.policy
                p.notify = false
                r.remote.updateAccountPolicy(userId: a.userId, policy: p)
                check("修改账号权限", wait { r.remote.accounts().first?.policy.notify == false })
            }
            r.remote.setLockOnEnd(on: true)
            check("远程结束后自动锁屏", r.remote.lockOnEnd())
            let assist = try await r.remote.startAssist()
            check("开启临时协助", !assist.code.isEmpty && wait { r.remote.status().assist != nil })
            (svc.worker as? MockWorkerHost)?.mockService?.simulateAssistRequest()
            check("协助请求推到界面", wait { listener.assistRequests.contains("ar-1") })
            r.remote.respondAssist(requestId: "ar-1", allow: true)
            check("允许协助后会话开始", wait { !listener.started.isEmpty })
            check("事件日志", !r.remote.recentEvents(limit: 50).isEmpty)
            if let sid = r.remote.status().sessions.first?.id {
                check("断开单个会话", r.remote.disconnectSession(id: sid) && wait { listener.ended.contains(sid) })
            }
            r.remote.setRemoteEnabled(enabled: false)
            check("禁止远程", wait { r.remote.status().remoteEnabled == false })
            r.remote.setRemoteEnabled(enabled: true)
            let up = try await r.checkUpdate(platform: "macos", current: "0.1.0")
            check("检查更新经后台服务", up?.version == MockWorkerHost.demoUpdate.version)
            if let up {
                // 模拟内核只报进度不写文件：先放一个文件在后台服务的下载位置
                let src = svc.opts.cacheDir.appendingPathComponent("updates", isDirectory: true)
                try? FileManager.default.createDirectory(at: src, withIntermediateDirectories: true)
                FileManager.default.createFile(atPath: src.appendingPathComponent(up.fileName).path, contents: Data("dmg".utf8))
                let dest = tmp.appendingPathComponent("dl.dmg").path
                let dl = TestTransfer()
                r.downloadUpdate(info: up, dest: dest, listener: dl)
                check("下载更新：进度与完成", wait(10) { dl.finished } && dl.progress > 0 && dl.error == nil, dl.error ?? "")
                check("下载更新：拷到当前用户的位置", FileManager.default.contents(atPath: dest) == Data("dmg".utf8))

                // 无人值守的自动更新：预先下载（已在后台服务的下载位置就直接就绪），进度经概况推给界面
                _ = try await r.call(IPCMethod.updateRequest, CUpdateRequest(info: up, install: false), timeout: 10)
                check("自动更新：预先下载后界面收到「已下载」", wait(5) { r.info?.update?.phase == "ready" && r.info?.update?.version == up.version },
                      "\(String(describing: r.info?.update))")
                // 「立即更新」：替换目标不是应用包，应被拒绝并说明原因，什么都不动
                do {
                    _ = try await r.call(IPCMethod.updateRequest, CUpdateRequest(info: up, install: true), timeout: 10)
                    check("自动更新：不能替换的位置拒绝「立即更新」", false)
                } catch {
                    check("自动更新：不能替换的位置拒绝「立即更新」并说明原因", describeError(error) == AppUpdateBlocker.notBundled.message, describeError(error))
                }
                check("自动更新：被拒绝后没有起助手、没有待完成的更新", svc.updater.state.pending == nil && !svc.updater.state.helperAlive && svc.updater.status?.phase != "restarting")
                // 换回过的版本不再自动预先下载
                svc.updater.state.addFailed(up.version)
                try? FileManager.default.removeItem(at: svc.updater.file(for: up))
                _ = try await r.call(IPCMethod.updateRequest, CUpdateRequest(info: up, install: false), timeout: 10)
                try? await Task.sleep(nanoseconds: 300_000_000)
                check("自动更新：换回过的版本不再自动下载", r.info?.update?.phase == "ready")
                // 零交互升级的入口在后台服务进程里交给 DaemonUpdater：替换目标不是应用包，报失败原因、不起助手
                AppUpdateEntry.startUpdateNow(info: up)
                check("自动更新：后台服务进程里的升级入口交给后台服务（不能替换时报原因）",
                      wait(5) { r.info?.update?.phase == "failed" && r.info?.update?.message?.contains(AppUpdateBlocker.notBundled.message) == true }
                      && svc.updater.state.pending == nil, "\(String(describing: r.info?.update))")
                // 后台服务模拟内核下载不写文件：下载完却没有文件时报错，不会去装
                try? FileManager.default.removeItem(at: svc.updater.state.failedURL)
                _ = try await r.call(IPCMethod.updateRequest, CUpdateRequest(info: up, install: false), timeout: 10)
                check("自动更新：下载完没有文件时报失败", wait(10) { r.info?.update?.phase == "failed" }, "\(String(describing: r.info?.update))")
            }
            try await r.logout(userId: acct?.userId ?? 0)
            check("退出账号", wait { r.remote.accounts().isEmpty })
            check("界面收到代理列表变化", infos.value.contains { $0 != nil })
        } catch {
            check("界面操作", false, describeError(error))
        }
        r.disconnect()
    }

    // MARK: 移交

    private static func migration(_ svc: DaemonService, _ tmp: URL, _ restarts: Counter) {
        // 给第一个后台服务放一份设备身份与数据，导出
        svc.keyStore.set(key: "worker.device_id", value: Data(repeating: 7, count: 16))
        svc.keyStore.set(key: "worker.device_key", value: Data(repeating: 9, count: 32))
        FileManager.default.createFile(atPath: svc.workerDir.appendingPathComponent("host.json").path, contents: Data("{\"server\":\"x\"}".utf8))
        try? FileManager.default.createDirectory(at: svc.workerDir.appendingPathComponent("sub"), withIntermediateDirectories: true)
        FileManager.default.createFile(atPath: svc.workerDir.appendingPathComponent("sub/events.log").path, contents: Data("e1\n".utf8))
        guard let fd = try? IPCSocket.connect(path: svc.opts.socketPath) else { check("连接后台服务（移交）", false); return }
        let c = IPCConnection(fd: fd)
        c.start()
        defer { c.close() }
        _ = try? c.call(IPCMethod.hello, ControlHello(version: AppInfo.version, build: AppInfo.build, locale: "zh-CN"), timeout: 3)
        guard let b = try? c.call(IPCMethod.exportData, IPCEmpty(), timeout: 5).decode(CMigration.self).bundle else {
            check("导出设备身份与数据", false)
            return
        }
        check("导出设备身份与数据（不含 keys/）", b.deviceId == Data(repeating: 7, count: 16) && b.files.map(\.path) == ["host.json", "sub/events.log"])

        // 第二个后台服务（全新）导入
        var o = DaemonService.Options()
        o.socketPath = tmp.appendingPathComponent("d2.sock").path
        o.dataRoot = tmp.appendingPathComponent("data2", isDirectory: true)
        o.cacheDir = tmp.appendingPathComponent("cache2", isDirectory: true)
        o.mock = true
        o.checkAgentUid = true
        let restarted = Counter()
        o.restart = { restarted.add() }
        let svc2 = DaemonService(o, locale: "zh-CN")
        guard (try? svc2.start()) != nil, let fd2 = try? IPCSocket.connect(path: o.socketPath) else { check("第二个后台服务", false); return }
        let c2 = IPCConnection(fd: fd2)
        c2.start()
        defer { c2.close(); svc2.stop() }
        let info = try? c2.call(IPCMethod.hello, ControlHello(version: "", build: "", locale: ""), timeout: 3).decode(DaemonInfo.self)
        check("全新的后台服务未被占用", info?.claimed == false)
        let bad = MigrationBundle(deviceId: b.deviceId, deviceKey: b.deviceKey, files: [.init(path: "../escape", data: Data())], sourceUser: "t")
        check("拒绝越出数据目录的文件", (try? c2.call(IPCMethod.importData, CMigration(bundle: bad), timeout: 5)) == nil
              && !FileManager.default.fileExists(atPath: o.dataRoot.appendingPathComponent("escape").path))
        let ok = (try? c2.call(IPCMethod.importData, CMigration(bundle: b), timeout: 5)) != nil
        check("导入设备身份与数据", ok && svc2.keyStore.get(key: "worker.device_key") == b.deviceKey
              && FileManager.default.contents(atPath: svc2.workerDir.appendingPathComponent("sub/events.log").path) == Data("e1\n".utf8))
        check("导入后要求重启后台服务", wait { restarted.value == 1 })
        let keyMode = try? FileManager.default.attributesOfItem(atPath: svc2.workerDir.path)[.posixPermissions] as? Int
        check("后台服务数据目录只给本用户（0700）", keyMode == 0o700)

        // 已有服务器的后台服务不接受导入
        _ = try? c.call(IPCMethod.setServer, CServer(server: MockWorkerHost.demoServer), timeout: 5)
        check("已占用的后台服务拒绝导入", (try? c.call(IPCMethod.importData, CMigration(bundle: b), timeout: 5)) == nil)

        // 代理报的用户与套接字对端不一致：拒绝
        let fp = FakePlatform(tag: 0xD5)
        let fake = makeClient(o.socketPath, fp, Box(AgentSession(type: .aqua, uid: getuid() == 0 ? 501 : 0, userName: "x", sessionId: 9, onConsole: true)))
        fake.start()
        Thread.sleep(forTimeInterval: 0.8)
        check("会话代理冒充别的用户被拒绝", svc2.platform.summaries().isEmpty)
        fake.stop()

        // 关闭无人值守：删除数据、代理退出
        let fp2 = FakePlatform(tag: 0xE6)
        let real = makeClient(o.socketPath, fp2, Box(AgentSession(type: .aqua, uid: getuid(), userName: NSUserName(), sessionId: 10, onConsole: true)))
        real.start()
        check("同一用户的代理可以连上", wait { svc2.platform.summaries().count == 1 })
        let rel = (try? c2.call(IPCMethod.release, CRelease(wipe: true), timeout: 5)) != nil
        check("关闭：删除后台服务的数据", rel && !FileManager.default.fileExists(atPath: svc2.workerDir.path))
        check("关闭：会话代理被要求正常退出", wait { fp2.exitRequests == [false] })
        real.stop()
        _ = restarts
    }
}

// MARK: - 测试用的小工具

final class Box<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var v: T
    init(_ v: T) { self.v = v }
    var value: T {
        get { lock.locked { v } }
        set { lock.locked { v = newValue } }
    }
    func mutate(_ f: (inout T) -> Void) { lock.locked { f(&v) } }
}

final class Counter: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    func add() { lock.locked { n += 1 } }
    var value: Int { lock.locked { n } }
}

/// 后台服务这边的假内容来源（代替内核给的主控端 ClipSource）：nil 项是文件夹；holdIndex 那项读完前 1000 字节后停住直到关掉
final class TestClipSource: ClipSource, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("假对象不能由内核指针构造") }
    let contents: [Data?]
    let holdIndex: Int?
    private let lock = NSLock()
    private var _opened: [[UInt64]] = []
    private var _closed = 0
    var opened: [[UInt64]] { lock.locked { _opened } }
    var closedCount: Int { lock.locked { _closed } }

    init(_ contents: [Data?], holdIndex: Int?) {
        self.contents = contents
        self.holdIndex = holdIndex
        super.init(noHandle: NoHandle())
    }

    func noteClosed() { lock.locked { _closed += 1 } }

    override func open(index: UInt32, offset: UInt64) throws -> ClipReader {
        lock.locked { _opened.append([UInt64(index), offset]) }
        guard Int(index) < contents.count, let d = contents[Int(index)] else { throw GalaxyError.NotFound(detail: "不是文件") }
        return TestClipReader(data: d, offset: Int(offset), hold: Int(index) == holdIndex ? 1000 : nil, owner: self)
    }
}

final class TestClipReader: ClipReader, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError("假对象不能由内核指针构造") }
    private let data: Data
    private var pos: Int
    private let hold: Int?
    private weak var owner: TestClipSource?
    private let lock = NSLock()
    private var closed = false

    init(data: Data, offset: Int, hold: Int?, owner: TestClipSource) {
        self.data = data
        self.pos = min(offset, data.count)
        self.hold = hold
        self.owner = owner
        super.init(noHandle: NoHandle())
    }

    override func size() -> UInt64 { UInt64(data.count) }

    override func read(max: UInt32) throws -> Data {
        while true {
            let (c, p) = lock.locked { (closed, pos) }
            if c { throw GalaxyError.Cancelled }
            if let h = hold, p >= h { Thread.sleep(forTimeInterval: 0.01); continue }
            let end = Swift.min(data.count, p + Int(max), hold ?? Int.max)
            lock.locked { pos = end }
            return data.subdata(in: p..<end)
        }
    }

    override func cancel() {
        let first = lock.locked { () -> Bool in defer { closed = true }; return !closed }
        if first { owner?.noteClosed() }
    }
}

/// 假的平台层：按标记字节生成画面帧与声音，记录收到的操作
final class FakePlatform: HostPlatform, @unchecked Sendable {
    let tag: UInt8
    private let lock = NSLock()
    var session: Box<AgentSession>?
    let displays: [DisplayInfo]
    let caps: [String]
    let shot: ImageData
    private var _injected: [InputEvent] = []
    private var _clipboard: ClipboardData?
    private var _privacy = false
    private var _audio: AudioPcmSink?
    private var _sinks: [UInt32: EncodedFrameSink] = [:]
    private var _keyframes = 0
    private var _bitrates: [UInt32] = []
    private var _cursorInVideo: [UInt32: Bool] = [:]
    private var _exit: [Bool] = []
    private var _virtual: [UInt32] = []
    private var _nextVirtual: UInt32 = 100
    private var _needsHeadless = false

    init(tag: UInt8) {
        self.tag = tag
        displays = [DisplayInfo(id: 1, name: "Fake \(tag)", width: 1920, height: 1080, scale: 2, primary: true, isVirtual: false)]
        caps = ["display", "screenshot", "input", "clipboard", "privacy", "audio", "tag-\(tag)", VirtualDisplays.capability(3)]
        shot = ImageData(mime: "image/png", data: Data((0..<300_000).map { UInt8(($0 + Int(tag)) % 256) }), width: 800, height: 450)
    }

    var injected: [InputEvent] { lock.locked { _injected } }
    var clipboard: ClipboardData? {
        get { lock.locked { _clipboard } }
        set { lock.locked { _clipboard = newValue } }
    }
    var privacy: Bool { lock.locked { _privacy } }
    var audioOn: Bool { lock.locked { _audio != nil } }
    var capturing: Set<UInt32> { lock.locked { Set(_sinks.keys) } }
    var keyframes: Int { lock.locked { _keyframes } }
    var bitrates: [UInt32] { lock.locked { _bitrates } }
    var cursorInVideo: [UInt32: Bool] { lock.locked { _cursorInVideo } }
    var exitRequests: [Bool] {
        get { lock.locked { _exit } }
        set { lock.locked { _exit = newValue } }
    }
    func sinkFor(_ d: UInt32) -> EncodedFrameSink? { lock.locked { _sinks[d] } }
    var virtualIds: [UInt32] { lock.locked { _virtual } }
    var needsHeadless: Bool {
        get { lock.locked { _needsHeadless } }
        set { lock.locked { _needsHeadless = newValue } }
    }

    func listDisplays() -> [DisplayInfo] { displays }
    func capabilities() -> [String] { caps }

    func startCapture(displayId: UInt32, options: CaptureOptions, sink: EncodedFrameSink) -> CaptureStarted? {
        lock.locked { _sinks[displayId] = sink }
        let size: (UInt32, UInt32) = tag == 0xB2 ? (2560, 1600) : (1920, 1080)
        let t = Thread { [weak self] in
            var seq: UInt32 = 0
            while let self, self.lock.locked({ self._sinks[displayId] === sink }) {
                var d = Data([self.tag])
                withUnsafeBytes(of: seq.littleEndian) { d.append(contentsOf: $0) }
                d.append(Data(repeating: UInt8(seq % 256), count: 1500))
                sink.push(data: d, ptsUs: UInt64(seq) * 16_000, keyframe: seq % 10 == 0)
                seq += 1
                Thread.sleep(forTimeInterval: 0.005)
            }
        }
        t.start()
        return CaptureStarted(width: size.0, height: size.1, codec: .h264)
    }

    /// 一个大关键帧（首字节 0xEE，其余按位置取值）
    func pushBig(display: UInt32, size: Int) {
        guard let s = sinkFor(display) else { return }
        var d = Data((0..<size).map { UInt8($0 % 253) })
        d[0] = 0xEE
        s.push(data: d, ptsUs: 1, keyframe: true)
    }

    func stopCapture(displayId: UInt32) { lock.locked { _ = _sinks.removeValue(forKey: displayId); _cursorInVideo[displayId] = nil } }
    /// 不画光标时报一次位置（x 为本代理的标记）
    func setCursorInVideo(displayId: UInt32, show: Bool) {
        let sink = lock.locked { () -> EncodedFrameSink? in
            _cursorInVideo[displayId] = show
            return _sinks[displayId]
        }
        if !show { sink?.setCursor(x: Int32(tag), y: 7, visible: true) }
    }
    func requestKeyframe(displayId: UInt32) { lock.locked { _keyframes += 1 } }
    func setBitrate(displayId: UInt32, kbps: UInt32) { lock.locked { _bitrates.append(kbps) } }
    func inject(displayId: UInt32, event: InputEvent) { lock.locked { _injected.append(event) } }
    func screenshot(displayId: UInt32, maxWidth: UInt32, maxHeight: UInt32) -> ImageData? { shot }
    func uiTree(displayId: UInt32, maxNodes: UInt32) -> String? { "{\"tag\":\(tag)}" }
    func computerUse(requestJson: String) -> ComputerUseReply? {
        ComputerUseReply(text: "cu:\(tag):" + requestJson, image: requestJson.contains("shot") ? shot : nil)
    }
    func clipboardGet() -> ClipboardData? { clipboard }
    func clipboardSet(data: ClipboardData) { clipboard = data }
    /// 收到的清单与内容来源（代理这边是转发给后台服务的）
    private var _offer: (ClipOfferInfo, ClipSource)?
    private var _offerError: String?
    var offer: (ClipOfferInfo, ClipSource)? { lock.locked { _offer } }
    var offerError: String? {
        get { lock.locked { _offerError } }
        set { lock.locked { _offerError = newValue } }
    }
    func clipboardSetOffer(offer: ClipOfferInfo, source: ClipSource) -> String? {
        lock.locked {
            if _offerError == nil { _offer = (offer, source) }
            return _offerError
        }
    }
    func setPrivacy(on: Bool) -> Bool { lock.locked { _privacy = on }; return true }

    func startAudio(sink: AudioPcmSink) -> Bool {
        lock.locked { _audio = sink }
        let tag = self.tag
        DispatchQueue.global().async {
            for _ in 0..<3 {
                var d = Data([tag])
                d.append(Data(repeating: 0, count: 3839))
                sink.push(pcm: d, sampleRate: 48_000, channels: 2)
                Thread.sleep(forTimeInterval: 0.01)
            }
        }
        return true
    }

    func stopAudio() { lock.locked { _audio = nil } }

    func virtualDisplayMax() -> UInt32 { 3 }
    /// 宽为 1 时假装失败（检查错误原因能带回内核）
    func addVirtualDisplay(width: UInt32, height: UInt32, refresh: UInt32) -> VirtualDisplayResult {
        if width == 1 { return VirtualDisplayResult(display: nil, error: "假平台拒绝 \(tag)") }
        let id = lock.locked { () -> UInt32 in
            defer { _nextVirtual += 1 }
            _virtual.append(_nextVirtual)
            return _nextVirtual
        }
        return VirtualDisplayResult(display: DisplayInfo(id: id, name: "Virtual \(tag)", width: width, height: height, scale: 1, primary: false, isVirtual: true),
                                    error: nil)
    }
    func removeVirtualDisplay(displayId: UInt32) -> String? {
        lock.locked { _virtual.removeAll { $0 == displayId } }
        return nil
    }
    func virtualDisplayIds() -> [UInt32] { virtualIds }
    func needsHeadlessDisplay() -> Bool { needsHeadless }
}

struct CursorReport: Equatable { var x: Int32; var y: Int32; var visible: Bool }

/// 内核那一侧的画面输出（记录帧、分辨率与光标位置）
final class TestSink: EncodedFrameSink, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError() }
    private let lock = NSLock()
    private var frames: [UInt8: [UInt32]] = [:]
    private var corrupt = false
    private var big = false
    private var res: [(UInt32, UInt32)] = []
    private var cur: [CursorReport] = []
    init() { super.init(noHandle: NoHandle()) }

    override func push(data: Data, ptsUs: UInt64, keyframe: Bool) {
        guard let first = data.first else { return }
        if first == 0xEE {
            let ok = data.count == 6 << 20 && data[data.startIndex + 1] == 1 && data[data.startIndex + data.count - 1] == UInt8((data.count - 1) % 253)
            lock.locked { big = ok }
            return
        }
        guard data.count == 1505 else { lock.locked { corrupt = true }; return }
        let seq = data.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 1, as: UInt32.self)) }
        let fill = data[data.startIndex + 5]
        lock.locked {
            if fill != UInt8(seq % 256) { corrupt = true }
            frames[first, default: []].append(seq)
        }
    }

    override func setResolution(width: UInt32, height: UInt32) { lock.locked { res.append((width, height)) } }
    override func setCursor(x: Int32, y: Int32, visible: Bool) { lock.locked { cur.append(CursorReport(x: x, y: y, visible: visible)) } }
    var cursors: [CursorReport] { lock.locked { cur } }

    func count(tag: UInt8) -> Int { lock.locked { frames[tag]?.count ?? 0 } }
    func inOrder(tag: UInt8) -> Bool {
        lock.locked {
            guard let f = frames[tag], !corrupt else { return false }
            return zip(f, f.dropFirst()).allSatisfy { $0 < $1 }
        }
    }
    var bigOK: Bool { lock.locked { big } }
    var resolutions: [(UInt32, UInt32)] { lock.locked { res } }
}

final class TestAudio: AudioPcmSink, @unchecked Sendable {
    required init(unsafeFromHandle handle: UInt64) { fatalError() }
    private let lock = NSLock()
    private var n = 0
    private var tag: UInt8 = 0
    init() { super.init(noHandle: NoHandle()) }
    override func push(pcm: Data, sampleRate: UInt32, channels: UInt32) {
        lock.locked {
            n += pcm.count
            tag = pcm.first ?? 0
        }
    }
    var bytes: Int { lock.locked { n } }
    var lastTag: UInt8 { lock.locked { tag } }
}

final class TestListener: HostListener, @unchecked Sendable {
    private let lock = NSLock()
    private var _status: HostStatus?
    private var _assist: [String] = []
    private var _started: [String] = []
    private var _ended: [String] = []
    func onStatus(status: HostStatus) { lock.locked { _status = status } }
    func onAssistRequest(requestId: String, fromUser: String, fromDevice: String) { lock.locked { _assist.append(requestId) } }
    func onSessionStarted(info: HostSessionInfo) { lock.locked { _started.append(info.id) } }
    func onSessionEnded(sessionId: String) { lock.locked { _ended.append(sessionId) } }
    func onAiActivity(sessionId: String, description: String) {}
    var lastStatus: HostStatus? { lock.locked { _status } }
    var assistRequests: [String] { lock.locked { _assist } }
    var started: [String] { lock.locked { _started } }
    var ended: [String] { lock.locked { _ended } }
}

final class TestTransfer: TransferListener, @unchecked Sendable {
    private let lock = NSLock()
    private var _p: UInt64 = 0
    private var _done = false
    private var _err: String?
    func onProgress(done: UInt64, total: UInt64) { lock.locked { _p = done } }
    func onFinished(error: String?) { lock.locked { _done = true; _err = error } }
    var progress: UInt64 { lock.locked { _p } }
    var finished: Bool { lock.locked { _done } }
    var error: String? { lock.locked { _err } }
}
