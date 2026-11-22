import XCTest
import CoreGraphics
@testable import GalaxyKit

final class HostTreeTests: XCTestCase {
    let data = HostsData(devices: MockData.devices(zh: false), vault: MockData.vault(zh: false))

    func testFoldersFirstAndNesting() {
        let rows = HostTree.rows(data)
        XCTAssertEqual(rows[0].folder?.id, "f-prod")
        XCTAssertEqual(rows[1].folder?.id, "f-db")
        XCTAssertEqual(rows[1].depth, 1)
        // Databases 下的两台 SSH 主机紧随其后、深度 2
        XCTAssertEqual(rows[2].item?.key, "ssh:s-dbp")
        XCTAssertEqual(rows[2].depth, 2)
        // 文件夹计数包含子文件夹里的条目：Production = 构建、GPU、web-01、web-02、WIN-SRV + Databases 的 2 台
        XCTAssertEqual(rows[0].count, 7)
    }

    func testCollapsedFolderHidesItems() {
        let rows = HostTree.rows(data)
        XCTAssertFalse(rows.contains { $0.item?.device?.id == "dev-nas" })
        let expanded = HostTree.rows(data, expandedOverride: ["f-home": true])
        XCTAssertTrue(expanded.contains { $0.item?.device?.id == "dev-nas" })
    }

    func testMobileAndSelfAreNotConnectable() {
        let ids = HostTree.items(data).compactMap { $0.device?.id }
        XCTAssertFalse(ids.contains("dev-phone"))
        XCTAssertFalse(ids.contains("dev-self"))
        XCTAssertTrue(ids.contains("dev-mbp"))
    }

    func testSearchIsFlatWithFolderPath() {
        let found = HostTree.rows(data, query: "replica")
        XCTAssertEqual(found.count, 1)
        XCTAssertEqual(found[0].path, "Production / Databases")
        XCTAssertEqual(found[0].depth, 0)
        // 多个词同时匹配
        XCTAssertEqual(HostTree.rows(data, query: "web prod").count, 2)
        // 别名与主机名都能搜到
        XCTAssertEqual(HostTree.rows(data, query: "build-server").first?.item?.title, "Build server")
    }

    func testTagAndFavoriteFilters() {
        let tagged = HostTree.rows(data, tag: "windows")
        XCTAssertFalse(tagged.isEmpty)
        XCTAssertTrue(tagged.allSatisfy { $0.item!.tags.contains("windows") })
        let favs = HostTree.rows(data, favoritesOnly: true)
        XCTAssertTrue(favs.allSatisfy { $0.item!.favorite })
        XCTAssertEqual(Set(favs.map { $0.item!.key }), ["device:dev-build", "device:dev-imac", "ssh:s-vps"])
        XCTAssertTrue(HostTree.allTags(data).contains("gpu"))
    }

    func testDescendantsAndMoveTargets() {
        XCTAssertEqual(HostTree.descendants(data, of: "f-prod"), ["f-prod", "f-db"])
        let targets = HostTree.moveTargets(data, excluding: HostTree.descendants(data, of: "f-prod"))
        XCTAssertFalse(targets.contains { $0.id == "f-db" || $0.id == "f-prod" })
        XCTAssertTrue(targets.contains { $0.path == "Office" })
    }

    func testCyclicFoldersDoNotHang() {
        var vault = data.vault
        if let i = vault.firstIndex(where: { $0.id == "f-prod" }) { vault[i].folderId = "f-db" }  // prod ↔ db 互为父子
        let rows = HostTree.rows(HostsData(devices: data.devices, vault: vault))
        XCTAssertFalse(rows.isEmpty)
    }

    func testNamesAndSubtitles() {
        XCTAssertEqual(data.deviceName("dev-build"), "Build server")
        XCTAssertEqual(data.deviceName("dev-gpu"), "gpu-node-02")
        XCTAssertEqual(data.targetName(AgentTarget(kind: .sshHost, id: "s-vps")), "vps-tokyo")
        let vps = HostTree.items(data).first { $0.key == "ssh:s-vps" }!
        XCTAssertEqual(vps.subtitle, "root@203.0.113.8:2222")
        XCTAssertEqual(vps.agentTarget?.kind, .sshHost)
        let rdp = HostTree.items(data).first { $0.key == "rdp:r-win" }!
        XCTAssertEqual(rdp.subtitle, "Administrator@10.0.3.5")
        XCTAssertNil(rdp.agentTarget)
    }
}

final class KeysTests: XCTestCase {
    func testHidCombo() {
        let e = Hid.combo(Hid.ctrl, Hid.alt, Hid.delete)
        XCTAssertEqual(e, [.key(hidUsage: Hid.ctrl, down: true), .key(hidUsage: Hid.alt, down: true),
                           .key(hidUsage: Hid.delete, down: true), .key(hidUsage: Hid.delete, down: false),
                           .key(hidUsage: Hid.alt, down: false), .key(hidUsage: Hid.ctrl, down: false)])
        XCTAssertEqual(Hid.tap(Hid.enter), [.key(hidUsage: 0x28, down: true), .key(hidUsage: 0x28, down: false)])
        XCTAssertEqual(Hid.letter("a"), 0x04)
        XCTAssertEqual(Hid.letter("Z"), 0x1D)
        XCTAssertEqual(Hid.f(1), 0x3A)
        XCTAssertEqual(Hid.f(12), 0x45)
        XCTAssertTrue(Hid.isModifier(Hid.gui))
        XCTAssertFalse(Hid.isModifier(Hid.enter))
    }

    func testUsageForCharacters() {
        XCTAssertEqual(Hid.usage(for: "c")?.0, 0x06)
        XCTAssertEqual(Hid.usage(for: "C")?.1, true)
        XCTAssertEqual(Hid.usage(for: "1")?.0, 0x1E)
        XCTAssertEqual(Hid.usage(for: "0")?.0, 0x27)
        XCTAssertEqual(Hid.usage(for: "?")?.0, 0x38)
        XCTAssertEqual(Hid.usage(for: "?")?.1, true)
        XCTAssertNil(Hid.usage(for: "中"))
    }

    func testShortcutsPerPlatform() {
        let mac = RemoteShortcuts.list(for: .macos)
        XCTAssertEqual(mac.first { $0.id == "copy" }?.events, Hid.combo(Hid.gui, Hid.letter("c")))
        XCTAssertNil(mac.first { $0.id == "ctrlAltDel" })
        let win = RemoteShortcuts.list(for: .windows)
        XCTAssertEqual(win.first { $0.id == "copy" }?.events, Hid.combo(Hid.ctrl, Hid.letter("c")))
        XCTAssertEqual(win.first { $0.id == "ctrlAltDel" }?.events, [.specialCombo(combo: .ctrlAltDel)])
        XCTAssertEqual(win.first { $0.id == "altF4" }?.events, Hid.combo(Hid.alt, Hid.f(4)))
    }

    func testTerminalKeySequences() {
        func s(_ b: [UInt8]) -> String { String(decoding: b, as: UTF8.self) }
        XCTAssertEqual(s(TerminalKeys.bytes(.up)), "\u{1b}[A")
        XCTAssertEqual(s(TerminalKeys.bytes(.up, appCursor: true)), "\u{1b}OA")
        XCTAssertEqual(s(TerminalKeys.bytes(.left, ctrl: true)), "\u{1b}[1;5D")
        XCTAssertEqual(s(TerminalKeys.bytes(.right, alt: true, shift: true)), "\u{1b}[1;4C")
        XCTAssertEqual(s(TerminalKeys.bytes(.home)), "\u{1b}[H")
        XCTAssertEqual(s(TerminalKeys.bytes(.pageDown)), "\u{1b}[6~")
        XCTAssertEqual(s(TerminalKeys.bytes(.delete, ctrl: true)), "\u{1b}[3;5~")
        XCTAssertEqual(s(TerminalKeys.bytes(.f1)), "\u{1b}OP")
        XCTAssertEqual(s(TerminalKeys.bytes(.f5)), "\u{1b}[15~")
        XCTAssertEqual(s(TerminalKeys.bytes(.f12)), "\u{1b}[24~")
        XCTAssertEqual(TerminalKeys.bytes(.tab), [0x09])
        XCTAssertEqual(s(TerminalKeys.bytes(.tab, shift: true)), "\u{1b}[Z")
        XCTAssertEqual(TerminalKeys.bytes(.esc, alt: true), [0x1b, 0x1b])
        XCTAssertEqual(TerminalKeys.bytes(.enter), [0x0d])
        XCTAssertEqual(TerminalKeys.bytes(.backspace), [0x7f])
    }

    func testCtrlAndAltText() {
        XCTAssertEqual(TerminalKeys.encode(text: "c", ctrl: true, alt: false), [0x03])
        XCTAssertEqual(TerminalKeys.encode(text: "C", ctrl: true, alt: false), [0x03])
        XCTAssertEqual(TerminalKeys.encode(text: "[", ctrl: true, alt: false), [0x1b])
        XCTAssertEqual(TerminalKeys.encode(text: " ", ctrl: true, alt: false), [0x00])
        XCTAssertEqual(TerminalKeys.encode(text: "b", ctrl: false, alt: true), [0x1b, 0x62])
        XCTAssertEqual(TerminalKeys.encode(text: "x", ctrl: true, alt: true), [0x1b, 0x18])
        // 非 ASCII 字符在 Ctrl 下原样发出
        XCTAssertEqual(TerminalKeys.encode(text: "中", ctrl: true, alt: false), Array("中".utf8))
    }
}

final class AnnexBTests: XCTestCase {
    func nal(_ type: UInt8, _ len: Int, short: Bool = false) -> Data {
        Data(short ? [0, 0, 1] : [0, 0, 0, 1]) + Data([type]) + Data(repeating: 0x55, count: len)
    }

    func testSplitAndTypes() {
        let stream = nal(7, 4) + nal(8, 2, short: true) + nal(5, 10) + nal(1, 8)
        let units = AnnexB.split(stream)
        XCTAssertEqual(units.count, 4)
        XCTAssertEqual(units.map { AnnexB.nalType($0, codec: .h264) }, [7, 8, 5, 1])
        XCTAssertEqual(units[2].count, 11)
        XCTAssertTrue(AnnexB.isParameterSet(7, codec: .h264))
        XCTAssertTrue(AnnexB.isKeyframe(5, codec: .h264))
        XCTAssertFalse(AnnexB.isKeyframe(1, codec: .h264))
        // H.265：类型在第一个字节的 1–6 位
        let vps = Data([0, 0, 0, 1, 32 << 1, 1, 0xAA])
        XCTAssertEqual(AnnexB.nalType(AnnexB.split(vps)[0], codec: .h265), 32)
        XCTAssertTrue(AnnexB.isParameterSet(32, codec: .h265))
        XCTAssertTrue(AnnexB.isKeyframe(19, codec: .h265))
    }

    func testAvccLengthPrefix() {
        let avcc = AnnexB.avcc([Data([1, 2, 3]), Data([4])])
        XCTAssertEqual([UInt8](avcc), [0, 0, 0, 3, 1, 2, 3, 0, 0, 0, 1, 4])
    }
}

final class FmtTests: XCTestCase {
    func testBytesAndModes() {
        XCTAssertEqual(Fmt.bytes(512), "512 B")
        XCTAssertEqual(Fmt.bytes(1536), "1.50 KB")
        XCTAssertEqual(Fmt.bytes(10 * 1024 * 1024), "10.0 MB")
        XCTAssertEqual(Fmt.mode(0o755, kind: .dir), "drwxr-xr-x")
        XCTAssertEqual(Fmt.mode(0o600), "-rw-------")
        XCTAssertEqual(Fmt.mode(0o777, kind: .symlink), "lrwxrwxrwx")
        XCTAssertEqual(Fmt.octal(0o644), "644")
        XCTAssertEqual(Fmt.parseOctal("755"), 0o755)
        XCTAssertNil(Fmt.parseOctal("789"))
        XCTAssertNil(Fmt.parseOctal(""))
        XCTAssertEqual(Fmt.tokens(1530), "1.5K")
    }

    func testRelativeTime() {
        let en = Strings(pack: LanguagePack(code: "en", name: "English", coreLocale: "en", strings: [
            "time.justNow": "just now", "time.minutesAgo": "{0} min ago", "time.hoursAgo": "{0} h ago", "time.daysAgo": "{0} d ago"]), fallback: nil)
        let now: Int64 = 1_800_000_000_000
        XCTAssertEqual(Fmt.relative(now - 10_000, now: now, strings: en), "just now")
        XCTAssertEqual(Fmt.relative(now - 5 * 60_000, now: now, strings: en), "5 min ago")
        XCTAssertEqual(Fmt.relative(now - 3 * 3_600_000, now: now, strings: en), "3 h ago")
        XCTAssertEqual(Fmt.relative(now - 26 * 3_600_000, now: now, strings: en), "1 d ago")
    }

    func testRemotePaths() {
        XCTAssertEqual(RemotePath.join("/home/deploy", "app"), "/home/deploy/app")
        XCTAssertEqual(RemotePath.join("/", "etc"), "/etc")
        XCTAssertEqual(RemotePath.parent("/home/deploy/app/"), "/home/deploy")
        XCTAssertEqual(RemotePath.parent("/home"), "/")
        XCTAssertEqual(RemotePath.parent("/"), "/")
        XCTAssertEqual(RemotePath.name("/var/log/syslog"), "syslog")
        XCTAssertEqual(RemotePath.crumbs("/home/deploy").map(\.path), ["/", "/home", "/home/deploy"])
        XCTAssertTrue(RemotePath.isValidName("notes.txt"))
        XCTAssertFalse(RemotePath.isValidName("a/b"))
        XCTAssertFalse(RemotePath.isValidName(".."))
        XCTAssertFalse(RemotePath.isValidName("  "))
    }

    func testFileSortKeepsFoldersFirst() {
        func e(_ n: String, _ k: FileKind, _ size: UInt64, _ m: Int64) -> FileEntry {
            FileEntry(name: n, path: "/x/" + n, kind: k, size: size, mode: 0o644, modifiedAt: m, owner: "", group: "", linkTarget: nil)
        }
        let list = [e("b.txt", .file, 10, 3), e(".hidden", .file, 1, 1), e("src", .dir, 0, 2), e("a.log", .file, 99, 1), e("lib", .dir, 0, 5)]
        XCTAssertEqual(FileSort.sorted(list, by: .name, descending: false, showHidden: false).map(\.name), ["lib", "src", "a.log", "b.txt"])
        XCTAssertEqual(FileSort.sorted(list, by: .size, descending: true, showHidden: true).map(\.name), ["src", "lib", "a.log", "b.txt", ".hidden"])
        XCTAssertEqual(FileSort.sorted(list, by: .modified, descending: false, showHidden: false).map(\.name), ["src", "lib", "a.log", "b.txt"])
    }

    func testQualityAndModes() {
        XCTAssertEqual(VideoQuality.auto.bitrateKbps, 0)
        XCTAssertEqual(VideoQuality.smooth.maxFps, 30)
        XCTAssertEqual(AiMode.selectable(maxMode: .confirmAll), [.readOnly, .confirmAll])
        XCTAssertEqual(AiMode.clamp(.fullTrust, to: .confirmDangerous), .confirmDangerous)
        XCTAssertEqual(AiMode.clamp(.readOnly, to: .fullTrust), .readOnly)
        XCTAssertEqual(AiMode.fromKey("confirmAll"), .confirmAll)
    }
}

final class LocalizationTests: XCTestCase {
    func load(_ code: String) throws -> LanguagePack {
        let url = AppResources.url("Locales/\(code).json")
        return try XCTUnwrap(LanguagePack.load(url), "缺语言包 \(url.path)")
    }

    func testPacksHaveSameKeysAndPlaceholders() throws {
        let en = try load("en"), zh = try load("zh-Hans")
        XCTAssertEqual(zh.coreLocale, "zh-CN")
        XCTAssertEqual(Set(en.strings.keys).symmetricDifference(zh.strings.keys), [])
        let re = try NSRegularExpression(pattern: "\\{\\d\\}")
        func placeholders(_ s: String) -> Set<String> {
            Set(re.matches(in: s, range: NSRange(s.startIndex..., in: s)).map { String(s[Range($0.range, in: s)!]) })
        }
        for (k, v) in en.strings {
            XCTAssertEqual(placeholders(v), placeholders(zh.strings[k] ?? ""), "占位符不一致：\(k)")
        }
    }

    func testFormattingAndFallback() throws {
        let en = try load("en"), zh = try load("zh-Hans")
        let s = Strings(pack: zh, fallback: en)
        XCTAssertEqual(s("files.deleteTitle", "a.txt"), "删除「a.txt」？")
        XCTAssertEqual(s("no.such.key"), "no.such.key")
        let partial = Strings(pack: LanguagePack(code: "xx", name: "", coreLocale: "xx", strings: [:]), fallback: en)
        XCTAssertEqual(partial("common.cancel"), "Cancel")
    }

    func testResolveLanguage() throws {
        let packs = [try load("en"), try load("zh-Hans")]
        XCTAssertEqual(Localization.resolve("system", packs: packs, preferred: ["zh-Hant-TW"]), "zh-Hans")
        XCTAssertEqual(Localization.resolve("system", packs: packs, preferred: ["fr-FR", "en-US"]), "en")
        XCTAssertEqual(Localization.resolve("system", packs: packs, preferred: ["de-DE"]), "en")
        XCTAssertEqual(Localization.resolve("en", packs: packs, preferred: ["zh-Hans"]), "en")
    }
}
