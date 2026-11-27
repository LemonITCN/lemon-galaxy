import Foundation
import Security

/// 自动安装更新的自检（AppUpdateInstaller.swift）。两个 Mac 应用的 --selftest 都会跑（CI 的 Mac 任务里也跑），
/// --update-selftest 单独跑并逐项打印；另给 --dmg <发布的 DMG> --current <旧版本的 .app> 时再拿真实发布包走一遍
/// 「挂载、核对签名、拷贝、换包、换回」。
/// 不碰「应用程序」文件夹、不登记任何系统服务、不打开任何界面：全部在临时目录里用应用副本做，
/// 「打开新版本」用 --update-probe 代替（直接运行副本里的程序，让它像新版本一样报告启动成功或故意不报告）。
enum AppUpdateSelfTest {
    final class Report {
        let verbose: Bool
        var problems: [String] = []
        var passed = 0
        init(verbose: Bool) { self.verbose = verbose }

        func check(_ name: String, _ ok: Bool, _ detail: @autoclosure () -> String = "") {
            let d = detail()
            if ok { passed += 1 } else { problems.append("自动更新：\(name)" + (d.isEmpty ? "" : "（\(d)）")) }
            if verbose { print("\(ok ? "[通过]" : "[失败]") \(name)\(d.isEmpty ? "" : "：" + d)") }
        }

        func skip(_ what: String) {
            if verbose { print("（跳过）\(what)") }
        }
    }

    static func run(verbose: Bool, dmg: String? = nil, currentApp: String? = nil) -> [String] {
        let rep = Report(verbose: verbose)
        let tmp = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("lg-update-test-\(getpid())", isDirectory: true)
        try? FileManager.default.removeItem(at: tmp)
        try? FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmp) }
        if verbose {
            print("临时目录 \(tmp.path)")
            print("本进程指定要求：\(AppUpdateCodeSign.ownIsCertSigned ? AppUpdateCodeSign.text(AppUpdateCodeSign.ownRequirement) : "（没有证书签名，签名与 DMG 相关检查跳过）")")
        }
        basics(rep, tmp)
        exchange(rep, tmp)
        if let src = sourceBundle(rep, tmp) {
            helperFlow(rep, tmp, src, fail: false)
            helperFlow(rep, tmp, src, fail: true)
        }
        signedChecks(rep, tmp)
        if let dmg, let currentApp { realRelease(rep, tmp, dmg: URL(fileURLWithPath: dmg), current: URL(fileURLWithPath: currentApp)) }
        if verbose { print("\nupdate-selftest：通过 \(rep.passed) 项，失败 \(rep.problems.count) 项") }
        return rep.problems
    }

    // MARK: 工具

    /// 测试用的副本：同一个 APFS 卷上直接克隆（瞬间完成，CI 上自检不拖时间），不行再 ditto
    static func ditto(_ from: URL, _ to: URL) -> Bool {
        if clonefile(from.path, to.path, 0) == 0 { return true }
        return AppUpdateProcess.run("/usr/bin/ditto", ["--noqtn", from.path, to.path]).status == 0
    }

    static func inode(_ url: URL) -> UInt64 {
        var st = stat()
        return stat(url.path, &st) == 0 ? UInt64(st.st_ino) : 0
    }

    static func waitUntil(_ timeout: Double, _ cond: () -> Bool) -> Bool {
        let end = Date().addingTimeInterval(timeout)
        while Date() < end {
            if cond() { return true }
            usleep(100_000)
        }
        return cond()
    }

    /// 本进程挂着的磁盘映像里有没有这个挂载点
    static func mounted(_ mount: URL) -> Bool {
        AppUpdateProcess.run("/sbin/mount", []).output.contains(" on \(AppUpdateProcess.realPath(mount.path)) ")
            || AppUpdateProcess.run("/sbin/mount", []).output.contains(" on \(mount.path) ")
    }

    /// 测试用的应用：正式包时整个包的副本（签名完好）；开发构建（swift build）时把可执行文件放进合成的包里
    static func sourceBundle(_ rep: Report, _ tmp: URL) -> URL? {
        let src = tmp.appendingPathComponent("src/Probe.app", isDirectory: true)
        try? FileManager.default.createDirectory(at: src.deletingLastPathComponent(), withIntermediateDirectories: true)
        if Bundle.main.bundleURL.pathExtension == "app" {
            guard ditto(Bundle.main.bundleURL, src) else { rep.check("复制本应用做测试副本", false); return nil }
            return src
        }
        guard let exe = Bundle.main.executablePath else { rep.check("找到本程序", false); return nil }
        let name = (exe as NSString).lastPathComponent
        let macos = src.appendingPathComponent("Contents/MacOS", isDirectory: true)
        try? FileManager.default.createDirectory(at: macos, withIntermediateDirectories: true)
        guard (try? FileManager.default.copyItem(atPath: exe, toPath: macos.appendingPathComponent(name).path)) != nil else {
            rep.check("复制本程序做测试副本", false)
            return nil
        }
        let plist: [String: Any] = ["CFBundleIdentifier": "cn.lemonit.galaxy.update-selftest", "CFBundleExecutable": name,
                                    "CFBundleShortVersionString": "0.0.0.1", "CFBundleVersion": "selftest", "CFBundlePackageType": "APPL"]
        (plist as NSDictionary).write(to: src.appendingPathComponent("Contents/Info.plist"), atomically: true)
        return src
    }

    // MARK: 基础

    static func basics(_ rep: Report, _ tmp: URL) {
        rep.check("版本号比较", AppVersion.newer("0.1.0.76", than: "0.1.0.75") && AppVersion.newer("0.1.0.100", than: "0.1.0.99")
                  && AppVersion.newer("0.1.1", than: "0.1.0.99") && !AppVersion.newer("0.1.0", than: "0.1.0.0")
                  && !AppVersion.newer("0.1.0.75", than: "0.1.0.76") && AppVersion("0.1.0") == AppVersion("0.1.0.0"))
        rep.check("不合法的版本号不算新", !AppVersion.newer("dev", than: "0.1.0") && !AppVersion.newer("1.2.x", than: "0.1")
                  && AppVersion("") == nil && AppVersion("1..2") == nil && AppVersion("-1") == nil)
        rep.check("从文件名取版本号", AppUpdate.version(inFileName: "LemonGalaxyWorker-0.1.0.76-macOS.dmg") == "0.1.0.76"
                  && AppUpdate.version(inFileName: "LemonGalaxy-0.1.0.75-macOS.part") == "0.1.0.75" && AppUpdate.version(inFileName: "x.dmg") == nil)

        // 缓存目录：只留比当前新的安装包
        let cache = tmp.appendingPathComponent("cache", isDirectory: true)
        try? FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
        for n in ["LemonGalaxy-0.1.0.74-macOS.dmg", "LemonGalaxy-0.1.0.75-macOS.dmg", "LemonGalaxy-0.1.0.76-macOS.dmg", "LemonGalaxy-0.1.0.77-macOS.part", "x.dmg", "keep.txt"] {
            FileManager.default.createFile(atPath: cache.appendingPathComponent(n).path, contents: Data([1]))
        }
        AppUpdate.pruneDownloads(cache, current: "0.1.0.75")
        let left = Set((try? FileManager.default.contentsOfDirectory(atPath: cache.path)) ?? [])
        rep.check("清理缓存里的旧安装包", left == ["LemonGalaxy-0.1.0.76-macOS.dmg", "LemonGalaxy-0.1.0.77-macOS.part", "keep.txt"], "\(left.sorted())")

        // 状态目录：失败的版本记下、结果只提示一次
        let state = AppUpdateState(dir: tmp.appendingPathComponent("state-basics", isDirectory: true))
        state.addFailed("0.1.0.9")
        state.addFailed("0.1.0.9")
        state.addFailed("0.1.0.10")
        rep.check("记下不再自动下载的版本", state.failedVersions == ["0.1.0.9", "0.1.0.10"])
        try? state.write(AppUpdateResult(outcome: "rolledBack", plan: "p", fromVersion: "1", toVersion: "2", reason: "r", at: 0, seen: false), state.resultURL)
        rep.check("更新结果只提示一次", state.takeResult()?.outcome == "rolledBack" && state.takeResult() == nil)
        let perm = (try? FileManager.default.attributesOfItem(atPath: state.dir.path))?[.posixPermissions] as? Int
        rep.check("状态目录只有本用户能读写（0700）", perm == 0o700, String(format: "%o", perm ?? 0))

        // 不能自动替换的位置
        if Bundle.main.bundleURL.pathExtension != "app" {
            rep.check("开发构建（不是 .app）不自动安装", AppUpdate.blocker() == .notBundled)
        }
        let fake = tmp.appendingPathComponent("NotAnApp", isDirectory: true)
        rep.check("不是应用包时不自动安装", AppUpdate.blocker(for: fake, requireCertSigned: false) == .notBundled)
        rep.check("应用隔离运行（App Translocation）时不自动安装",
                  AppUpdate.blocker(for: URL(fileURLWithPath: "/private/var/folders/x/AppTranslocation/ABC/d/LemonGalaxy.app"), requireCertSigned: false) != nil)

        // 上次中断留下的隐藏包会被清理，别的东西不动
        let apps = tmp.appendingPathComponent("leftover", isDirectory: true)
        let target = apps.appendingPathComponent("Thing.app", isDirectory: true)
        let stale = AppUpdate.stagingURL(for: target)
        let other = apps.appendingPathComponent(".Other-update-1234.app", isDirectory: true)
        for d in [target, stale, other] { try? FileManager.default.createDirectory(at: d.appendingPathComponent("Contents"), withIntermediateDirectories: true) }
        AppUpdate.cleanupLeftovers(target: target, state: AppUpdateState(dir: tmp.appendingPathComponent("state-leftover", isDirectory: true)))
        let fm = FileManager.default
        rep.check("清理上次中断留下的隐藏包", !fm.fileExists(atPath: stale.path) && fm.fileExists(atPath: target.path) && fm.fileExists(atPath: other.path))
    }

    static func exchange(_ rep: Report, _ tmp: URL) {
        let fm = FileManager.default
        let a = tmp.appendingPathComponent("swap/A.app", isDirectory: true), b = tmp.appendingPathComponent("swap/B.app", isDirectory: true)
        try? fm.createDirectory(at: a, withIntermediateDirectories: true)
        try? fm.createDirectory(at: b, withIntermediateDirectories: true)
        fm.createFile(atPath: a.appendingPathComponent("a").path, contents: Data("a".utf8))
        fm.createFile(atPath: b.appendingPathComponent("b").path, contents: Data("b".utf8))
        let ia = inode(a), ib = inode(b)
        let ok1 = (try? AppUpdate.exchange(a, b)) != nil
        rep.check("原子对换两个包", ok1 && inode(a) == ib && inode(b) == ia && fm.fileExists(atPath: b.appendingPathComponent("a").path))
        let ok2 = (try? AppUpdate.exchange(a, b)) != nil
        rep.check("再对换一次换回原样", ok2 && inode(a) == ia && inode(b) == ib)
        let missing = tmp.appendingPathComponent("swap/Missing.app")
        rep.check("对换不存在的包报错、原包不动", (try? AppUpdate.exchange(missing, a)) == nil && inode(a) == ia)
    }

    // MARK: 助手

    /// 起真的助手进程（本程序 --update-helper）：fail 为假时新版本报告启动成功、旧包被删；为真时新版本不报告，助手换回旧版本
    static func helperFlow(_ rep: Report, _ tmp: URL, _ src: URL, fail: Bool) {
        let fm = FileManager.default
        let base = tmp.appendingPathComponent(fail ? "flow-rollback" : "flow-ok", isDirectory: true)
        let target = base.appendingPathComponent("Apps/Probe.app", isDirectory: true)
        let backup = AppUpdate.stagingURL(for: target)
        try? fm.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
        guard ditto(src, target), ditto(src, backup), let info = AppBundleInfo.read(target) else {
            rep.check("准备新旧两份副本", false)
            return
        }
        let newIno = inode(target), oldIno = inode(backup)
        let state = AppUpdateState(dir: base.appendingPathComponent("state", isDirectory: true))
        state.ensure()
        // 「旧进程」：一个很快退出的子进程
        let old = Process()
        old.executableURL = URL(fileURLWithPath: "/bin/sleep")
        old.arguments = ["0.5"]
        try? old.run()
        let plan = AppUpdatePlan(id: UUID().uuidString, mode: "app", launch: fail ? "probe-fail" : "probe", target: target.path, backup: backup.path,
                                 oldPid: old.processIdentifier, fromVersion: "1.0.0", fromBuild: "old", toVersion: "2.0.0", toBuild: info.build,
                                 timeout: fail ? 3 : 30, launchdLabel: nil, createdAt: Date().timeIntervalSince1970)
        do {
            try state.write(plan, state.pendingURL)
            try AppUpdate.spawnHelper(plan, state: state)
        } catch {
            rep.check("起助手", false, "\(error)")
            return
        }
        old.waitUntilExit()
        let finished = waitUntil(60) { state.pending == nil && !state.helperAlive }
        let r = state.result
        let what = fail ? "新版本没报告启动成功时自动换回旧版本" : "新版本报告启动成功后删掉旧版本"
        guard finished, let r else {
            rep.check(what, false, "助手没有结束；日志：\((try? String(contentsOf: state.logURL, encoding: .utf8))?.suffix(800) ?? "")")
            return
        }
        let probes = ((try? String(contentsOf: state.probeLogURL, encoding: .utf8)) ?? "").split(separator: "\n")
        if fail {
            rep.check(what, r.outcome == "rolledBack" && inode(target) == oldIno && !fm.fileExists(atPath: backup.path),
                      "结果 \(r.outcome)，目标是\(inode(target) == oldIno ? "旧" : "新")版本，备份\(fm.fileExists(atPath: backup.path) ? "还在" : "已删")")
            rep.check("换回后记下这个版本不再自动下载", state.failedVersions.contains("2.0.0"))
            rep.check("换回后重新打开了旧版本", probes.count == 2 && probes.first?.hasSuffix("fail") == true && probes.last?.hasSuffix("ok") == true, "\(probes)")
        } else {
            rep.check(what, r.outcome == "ok" && inode(target) == newIno && !fm.fileExists(atPath: backup.path) && state.marker?.plan == plan.id,
                      "结果 \(r.outcome)，备份\(fm.fileExists(atPath: backup.path) ? "还在" : "已删")")
            rep.check("新版本只打开一次", probes.count == 1, "\(probes)")
        }
    }

    // MARK: 签名、DMG、整个安装流程（正式签名的包才做）

    static func signedChecks(_ rep: Report, _ tmp: URL) {
        let fm = FileManager.default
        guard Bundle.main.bundleURL.pathExtension == "app", AppUpdateCodeSign.ownIsCertSigned, let req = AppUpdateCodeSign.ownRequirement,
              let bundleId = Bundle.main.bundleIdentifier else {
            rep.skip("签名与 DMG：本进程不是用证书签名的应用包")
            return
        }
        let dir = tmp.appendingPathComponent("signed", isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        let name = Bundle.main.bundleURL.lastPathComponent
        let copy = dir.appendingPathComponent("copy/" + name, isDirectory: true)
        try? fm.createDirectory(at: copy.deletingLastPathComponent(), withIntermediateDirectories: true)
        guard ditto(Bundle.main.bundleURL, copy) else { rep.check("复制本应用", false); return }
        rep.check("本应用的副本满足自己的指定要求（严格、所有架构）", (try? AppUpdateCodeSign.verify(copy, requirement: req)) != nil)
        rep.check("本应用位置可以自动替换（测试副本所在的临时目录）", AppUpdate.blocker(for: copy) == nil, "\(String(describing: AppUpdate.blocker(for: copy)))")
        do {
            try AppUpdate.checkBundle(copy, bundleId: bundleId, newerThan: AppInfo.version, requirement: req)
            rep.check("版本不比当前新时拒绝", false)
        } catch let e as AppUpdateError {
            rep.check("版本不比当前新时拒绝", e.kind == .notNewer)
        } catch {
            rep.check("版本不比当前新时拒绝", false, "\(error)")
        }
        let tampered = dir.appendingPathComponent("tampered/" + name, isDirectory: true)
        try? fm.createDirectory(at: tampered.deletingLastPathComponent(), withIntermediateDirectories: true)
        if ditto(copy, tampered) {
            fm.createFile(atPath: tampered.appendingPathComponent("Contents/Resources/injected.txt").path, contents: Data("x".utf8))
            rep.check("包里多了文件时签名核对不过", (try? AppUpdateCodeSign.verify(tampered, requirement: req)) == nil)
        }
        let adhoc = dir.appendingPathComponent("adhoc/" + name, isDirectory: true)
        try? fm.createDirectory(at: adhoc.deletingLastPathComponent(), withIntermediateDirectories: true)
        if ditto(copy, adhoc), AppUpdateProcess.run("/usr/bin/codesign", ["--force", "--deep", "--sign", "-", adhoc.path]).status == 0 {
            rep.check("换成别的签名（临时签名）时拒绝", (try? AppUpdateCodeSign.verify(adhoc, requirement: req)) == nil)
        }

        // 用本应用的副本做一个和发布包同样结构的 DMG（应用 + 指向「应用程序」的替身）
        let stage = dir.appendingPathComponent("dmg-src", isDirectory: true)
        try? fm.createDirectory(at: stage, withIntermediateDirectories: true)
        let dmg = dir.appendingPathComponent("Test.dmg")
        guard ditto(copy, stage.appendingPathComponent(name)),
              (try? fm.createSymbolicLink(atPath: stage.appendingPathComponent("Applications").path, withDestinationPath: "/Applications")) != nil,
              AppUpdateProcess.run("/usr/bin/hdiutil", ["create", "-volname", "LGUpdateTest", "-srcfolder", stage.path, "-fs", "HFS+", "-format", "UDRO", "-quiet", dmg.path], timeout: 300).status == 0 else {
            rep.check("做测试用的 DMG", false)
            return
        }
        try? fm.removeItem(at: stage)

        // 挂载后包在只读卷上：不能自动替换
        let mnt = dir.appendingPathComponent("mnt", isDirectory: true)
        if (try? AppUpdate.attach(dmg, at: mnt)) != nil {
            rep.check("从 DMG 里直接运行时不自动安装（只读卷）", AppUpdate.blocker(for: mnt.appendingPathComponent(name)) == .readOnly)
            rep.check("挂载不出现在访达（-nobrowse）、能找到包里的应用", AppUpdate.findApp(in: mnt, bundleId: bundleId) != nil)
            AppUpdate.detach(mnt)
            rep.check("卸载 DMG", !mounted(mnt) && !fm.fileExists(atPath: mnt.path))
        } else {
            rep.check("挂载测试用的 DMG", false)
        }

        // 整个安装流程：当前应用是副本、「新版本」是 DMG 里的同一份程序（把当前版本当成 0.0.0.1），
        // 助手用 --update-probe 代替真的打开
        let apps = dir.appendingPathComponent("Apps", isDirectory: true)
        let target = apps.appendingPathComponent(name, isDirectory: true)
        try? fm.createDirectory(at: apps, withIntermediateDirectories: true)
        guard ditto(copy, target) else { rep.check("准备当前应用的副本", false); return }
        let oldIno = inode(target)
        let state = AppUpdateState(dir: dir.appendingPathComponent("state", isDirectory: true))
        let work = dir.appendingPathComponent("work", isDirectory: true)
        try? fm.createDirectory(at: work, withIntermediateDirectories: true)
        var r = AppUpdate.Request(dmg: dmg, target: target, bundleId: bundleId, currentVersion: AppInfo.version, currentBuild: AppInfo.build,
                                  requirement: req, state: state)
        r.workDir = work
        r.launch = "probe"
        do {
            _ = try AppUpdate.install(r)
            rep.check("版本相同的安装包被拒绝", false)
        } catch let e as AppUpdateError {
            rep.check("版本相同的安装包被拒绝、什么都没动", e.kind == .notNewer && inode(target) == oldIno && hiddenLeft(apps).isEmpty && state.pending == nil)
        } catch {
            rep.check("版本相同的安装包被拒绝", false, "\(error)")
        }
        rep.check("拒绝后没有挂着的 DMG", ((try? fm.contentsOfDirectory(atPath: work.path)) ?? []).isEmpty)

        let old = Process()
        old.executableURL = URL(fileURLWithPath: "/bin/sleep")
        old.arguments = ["0.5"]
        try? old.run()
        r.currentVersion = "0.0.0.1"
        r.oldPid = old.processIdentifier
        r.timeout = 30
        var steps: [AppUpdate.Step] = []
        do {
            let plan = try AppUpdate.install(r) { steps.append($0) }
            old.waitUntilExit()
            rep.check("安装：核对、替换、重启三步", steps == [.verifying, .replacing, .restarting])
            rep.check("安装：换包后目标是新包、旧包在隐藏位置", inode(target) != oldIno && inode(URL(fileURLWithPath: plan.backup)) == oldIno)
            let done = waitUntil(60) { state.pending == nil && !state.helperAlive }
            rep.check("安装：新版本报告启动成功后旧包删掉", done && state.result?.outcome == "ok" && hiddenLeft(apps).isEmpty,
                      "结果 \(state.result?.outcome ?? "—")，剩下 \(hiddenLeft(apps))")
            rep.check("安装：装好的应用签名完好", (try? AppUpdateCodeSign.verify(target, requirement: req)) != nil)
            rep.check("安装：没有挂着的 DMG", ((try? fm.contentsOfDirectory(atPath: work.path)) ?? []).isEmpty)
            rep.check("安装：没有隔离标记", !AppUpdateProcess.run("/usr/bin/xattr", ["-r", "-l", target.path]).output.contains("com.apple.quarantine"))
        } catch {
            old.waitUntilExit()
            rep.check("安装流程", false, "\(error)")
        }
    }

    static func hiddenLeft(_ dir: URL) -> [String] {
        ((try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []).filter { $0.hasPrefix(".") && $0.hasSuffix(".app") }
    }

    // MARK: 真实发布包

    /// current：旧版本的 .app（当成「当前应用」，取它的指定要求）；dmg：新版本的发布 DMG。
    /// 只做挂载、核对、拷贝、换包、换回，不起助手（发布包里的旧程序不认 --update-helper，起了会打开界面）
    static func realRelease(_ rep: Report, _ tmp: URL, dmg: URL, current: URL) {
        let fm = FileManager.default
        guard let req = AppUpdateCodeSign.designatedRequirement(of: current), let cur = AppBundleInfo.read(current) else {
            rep.check("读取当前应用 \(current.path) 的签名与版本", false)
            return
        }
        if rep.verbose { print("真实发布包：当前 \(cur.identifier) \(cur.version)，要求 \(AppUpdateCodeSign.text(req))") }
        let dir = tmp.appendingPathComponent("real", isDirectory: true)
        let apps = dir.appendingPathComponent("Apps", isDirectory: true)
        let target = apps.appendingPathComponent(current.lastPathComponent, isDirectory: true)
        try? fm.createDirectory(at: apps, withIntermediateDirectories: true)
        guard ditto(current, target) else { rep.check("复制当前应用", false); return }
        let oldIno = inode(target)
        let work = dir.appendingPathComponent("work", isDirectory: true)
        try? fm.createDirectory(at: work, withIntermediateDirectories: true)
        var r = AppUpdate.Request(dmg: dmg, target: target, bundleId: cur.identifier, currentVersion: cur.version, currentBuild: cur.build,
                                  requirement: req, state: AppUpdateState(dir: dir.appendingPathComponent("state", isDirectory: true)))
        r.workDir = work
        let t0 = Date()
        do {
            let (staged, info) = try AppUpdate.stage(r)
            rep.check("真实发布包：挂载、核对签名与版本、拷到目标旁边（\(String(format: "%.1f", Date().timeIntervalSince(t0))) 秒）",
                      AppVersion.newer(info.version, than: cur.version) && staged.deletingLastPathComponent().path == apps.path,
                      "\(cur.version) → \(info.version)")
            rep.check("真实发布包：DMG 已卸载", ((try? fm.contentsOfDirectory(atPath: work.path)) ?? []).isEmpty)
            rep.check("真实发布包：拷出来的没有隔离标记", !AppUpdateProcess.run("/usr/bin/xattr", ["-r", "-l", staged.path]).output.contains("com.apple.quarantine"))
            try AppUpdate.exchange(staged, target)
            rep.check("真实发布包：换包后目标是新版本、签名完好", AppBundleInfo.read(target)?.version == info.version
                      && (try? AppUpdateCodeSign.verify(target, requirement: req)) != nil && inode(staged) == oldIno)
            try AppUpdate.exchange(staged, target)
            rep.check("真实发布包：换回旧版本", AppBundleInfo.read(target)?.version == cur.version && inode(target) == oldIno
                      && (try? AppUpdateCodeSign.verify(target, requirement: req)) != nil)
            try? fm.removeItem(at: staged)
        } catch {
            rep.check("真实发布包：挂载、核对、拷贝", false, "\(error)")
        }
        // 同一个包当成「当前已是新版本」：拒绝
        r.currentVersion = AppVersion.newer("999.0", than: cur.version) ? "999.0" : cur.version
        rep.check("真实发布包：不比当前新时拒绝、不留临时文件", (try? AppUpdate.stage(r)) == nil && hiddenLeft(apps).isEmpty
                  && ((try? fm.contentsOfDirectory(atPath: work.path)) ?? []).isEmpty)
        // 别的签名身份当「当前应用」：拒绝
        if let own = AppUpdateCodeSign.ownRequirement, AppUpdateCodeSign.text(own) != AppUpdateCodeSign.text(req) {
            r.currentVersion = cur.version
            r.requirement = own
            do {
                _ = try AppUpdate.stage(r)
                rep.check("真实发布包：签名身份与当前应用不同时拒绝", false)
            } catch let e as AppUpdateError {
                rep.check("真实发布包：签名身份与当前应用不同时拒绝", e.kind == .signature && hiddenLeft(apps).isEmpty, e.detail)
            } catch {
                rep.check("真实发布包：签名身份与当前应用不同时拒绝", false, "\(error)")
            }
        }
    }

    /// --update-selftest [--dmg 发布的 DMG --current 旧版本的 .app]
    static func main(_ args: [String]) -> Int32 {
        setvbuf(stdout, nil, _IOLBF, 0)
        func arg(_ n: String) -> String? {
            guard let i = args.firstIndex(of: n), i + 1 < args.count else { return nil }
            return args[i + 1]
        }
        let problems = run(verbose: true, dmg: arg("--dmg"), currentApp: arg("--current"))
        for p in problems { print("问题：" + p) }
        return problems.isEmpty ? 0 : 1
    }
}
