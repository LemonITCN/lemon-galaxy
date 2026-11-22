import XCTest
@testable import GalaxyKit

final class AiChatTests: XCTestCase {
    // MARK: @ 引用

    func testMentionQuery() {
        XCTAssertEqual(MentionParser.query(in: "@"), "")
        XCTAssertEqual(MentionParser.query(in: "看看 @bui"), "bui")
        XCTAssertEqual(MentionParser.query(in: "第一行\n@nas"), "nas")
        XCTAssertNil(MentionParser.query(in: "mail@example.com"), "@ 前面不是空白不算")
        XCTAssertNil(MentionParser.query(in: "@build 的磁盘"), "过滤词后面有空白就结束了")
        XCTAssertNil(MentionParser.query(in: "没有引用"))
        XCTAssertEqual(MentionParser.removeQuery(from: "看看 @bui"), "看看 ")
        XCTAssertEqual(MentionParser.removeQuery(from: "@"), "")
        XCTAssertEqual(MentionParser.removeQuery(from: "a@b"), "a@b")
    }

    func testMentionFilterOnlineFirst() {
        struct C { let name: String; let online: Bool }
        let all = [C(name: "nas", online: false), C(name: "Build server", online: true), C(name: "office-pc", online: true)]
        let r = MentionParser.filter(all, query: "", name: \.name, online: \.online).map(\.name)
        XCTAssertEqual(r, ["Build server", "office-pc", "nas"])
        XCTAssertEqual(MentionParser.filter(all, query: "BUI", name: \.name, online: \.online).map(\.name), ["Build server"])
        XCTAssertEqual(MentionParser.filter(all, query: "zzz", name: \.name, online: \.online).count, 0)
    }

    // MARK: 对话按日期分组

    func testConversationGrouping() {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(identifier: "Asia/Shanghai")!
        // 2026-11-22 10:00 +08:00
        let now = Int64(DateComponents(calendar: cal, timeZone: cal.timeZone, year: 2026, month: 11, day: 22, hour: 10).date!.timeIntervalSince1970 * 1000)
        let h: Int64 = 3_600_000
        XCTAssertEqual(ConversationGroup.of(now - h, now: now, calendar: cal), .today)
        XCTAssertEqual(ConversationGroup.of(now - 10 * h, now: now, calendar: cal), .today, "当天 0 点之后")
        XCTAssertEqual(ConversationGroup.of(now - 11 * h, now: now, calendar: cal), .yesterday, "前一天 23 点")
        XCTAssertEqual(ConversationGroup.of(now - 34 * h, now: now, calendar: cal), .yesterday)
        XCTAssertEqual(ConversationGroup.of(now - 35 * h, now: now, calendar: cal), .last7Days)
        XCTAssertEqual(ConversationGroup.of(now - 6 * 24 * h, now: now, calendar: cal), .last7Days)
        XCTAssertEqual(ConversationGroup.of(now - 7 * 24 * h, now: now, calendar: cal), .earlier)
        XCTAssertEqual(ConversationGroup.of(now + h, now: now, calendar: cal), .today, "时钟偏差算今天")

        let items: [(String, Int64)] = [("old", now - 30 * 24 * h), ("t1", now - h), ("y", now - 20 * h), ("t2", now - 2 * h + 1)]
        let g = ConversationGroup.group(items, updatedAt: \.1, now: now, calendar: cal)
        XCTAssertEqual(g.map(\.0), [.today, .yesterday, .earlier], "空组不出现")
        XCTAssertEqual(g[0].1.map(\.0), ["t1", "t2"], "组内新的在前")
    }

    // MARK: 默认模型与档位

    func model(_ p: UInt64, _ id: String) -> AiModel {
        AiModel(providerId: p, providerName: "P\(p)", id: id, name: id.uppercased(), vision: false, tools: true, contextLength: 1000)
    }

    func testDefaultModel() {
        let ms = [model(1, "a"), model(1, "b"), model(2, "a")]
        XCTAssertEqual(ModelChoice.key(ms[2]), "2/a")
        XCTAssertEqual(ModelChoice.pick(ms, remembered: "2/a").map(ModelChoice.key), "2/a", "同名模型按供应商区分")
        XCTAssertEqual(ModelChoice.pick(ms, remembered: "9/x").map(ModelChoice.key), "1/a", "记住的已不可用取第一个")
        XCTAssertEqual(ModelChoice.pick(ms, remembered: nil).map(ModelChoice.key), "1/a")
        XCTAssertNil(ModelChoice.pick([], remembered: "1/a"))
        XCTAssertEqual(ModelChoice.grouped(ms).map(\.provider), ["P1", "P2"])
        XCTAssertEqual(ModelChoice.grouped(ms)[0].models.count, 2)
    }

    func testDefaultMode() {
        XCTAssertEqual(AiMode.initial(saved: nil, maxMode: .fullTrust), .confirmDangerous)
        XCTAssertEqual(AiMode.initial(saved: nil, maxMode: .confirmAll), .confirmAll, "不超过管理员上限")
        XCTAssertEqual(AiMode.initial(saved: .fullTrust, maxMode: .fullTrust), .fullTrust)
        XCTAssertEqual(AiMode.initial(saved: .readOnly, maxMode: .confirmDangerous), .readOnly)
        XCTAssertEqual(AiMode.chatModes, [.readOnly, .confirmAll, .confirmDangerous, .fullTrust])
        XCTAssertFalse(AiMode.fullTrust.allowed(maxMode: .confirmDangerous))
        XCTAssertTrue(AiMode.readOnly.allowed(maxMode: .readOnly))
    }

    func testAvailability() {
        let ok = AiModels(enabled: true, maxMode: .fullTrust, items: [model(1, "a")])
        XCTAssertEqual(AiAvailability.of(models: nil, error: nil, loading: true, userAiEnabled: true), .loading)
        XCTAssertEqual(AiAvailability.of(models: nil, error: "x", loading: false, userAiEnabled: true), .failed("x"))
        XCTAssertEqual(AiAvailability.of(models: nil, error: "x", loading: true, userAiEnabled: true), .loading, "重试中显示加载")
        XCTAssertEqual(AiAvailability.of(models: ok, error: nil, loading: false, userAiEnabled: false), .disabled)
        XCTAssertEqual(AiAvailability.of(models: AiModels(enabled: false, maxMode: .off, items: []), error: nil, loading: false, userAiEnabled: nil), .disabled)
        XCTAssertEqual(AiAvailability.of(models: AiModels(enabled: true, maxMode: .fullTrust, items: []), error: nil, loading: false, userAiEnabled: true), .noModels)
        XCTAssertEqual(AiAvailability.of(models: ok, error: nil, loading: false, userAiEnabled: true), .ready)
    }

    // MARK: 引用

    func testRefs() {
        XCTAssertEqual(ContextRef.file(target: "dev1", path: "/etc/nginx/nginx.conf"),
                       ContextRef(kind: .file, targetId: "dev1", id: "/etc/nginx/nginx.conf", label: "nginx.conf"))
        XCTAssertEqual(ContextRef.folder(target: "s1", path: "/var/log/").label, "log")
        XCTAssertEqual(ContextRef.folder(target: "s1", path: "/").label, "/")
        XCTAssertEqual(ContextRef.activity(label: "最近连接记录"), ContextRef(kind: .activity, targetId: "", id: "", label: "最近连接记录"))
        XCTAssertEqual(ContextRef.device("d1", label: "x").targetId, "")
        var refs: [ContextRef] = []
        refs.addRef(.device("d1", label: "build"))
        refs.addRef(.device("d1", label: "build（改名）"))
        refs.addRef(.file(target: "d1", path: "/a"))
        refs.addRef(.file(target: "d2", path: "/a"))
        XCTAssertEqual(refs.count, 3)
        XCTAssertEqual(ContextRef.agentTarget(AgentTarget(kind: .sshHost, id: "s1"), label: "vps").kind, .sshHost)
    }

    func testUserSentTitleAndRefs() {
        let s0 = ConversationState(id: "c", title: "", modelName: "M", vision: false, targetNames: [], mode: .confirmDangerous)
        let s = ChatReducer.userSent(s0, text: "", refs: [.device("d1", label: "build")])
        XCTAssertEqual(s.title, "build", "只有引用时用引用名作标题")
        guard case .user(_, "", let refs) = s.items[0] else { return XCTFail() }
        XCTAssertEqual(refs.count, 1)
        XCTAssertEqual(ChatReducer.userSent(s0, text: "看看   磁盘\n占用").title, "看看 磁盘 占用")
    }

    // MARK: Markdown

    func testMarkdownBlocks() {
        let md = """
        # 标题
        一段**粗体**和 `code`
        第二行

        - 一
          - 二
        1. 甲
        > 引用
        ---
        | 列 A | 列 B |
        |---|:--:|
        | 1 | 2 |
        | 3 |
        ```bash
        df -h
        ```
        """
        let b = MarkdownParser.parse(md)
        XCTAssertEqual(b, [
            .heading(level: 1, text: "标题"),
            .paragraph("一段**粗体**和 `code`\n第二行"),
            .listItem(level: 0, marker: nil, text: "一"),
            .listItem(level: 1, marker: nil, text: "二"),
            .listItem(level: 0, marker: "1.", text: "甲"),
            .quote("引用"),
            .rule,
            .table(header: ["列 A", "列 B"], rows: [["1", "2"], ["3", ""]]),
            .code(language: "bash", text: "df -h"),
        ])
    }

    func testMarkdownStreamingUnclosedFence() {
        XCTAssertEqual(MarkdownParser.parse("看：\n```py\nprint(1)"), [.paragraph("看："), .code(language: "py", text: "print(1)")])
        XCTAssertEqual(MarkdownParser.parse("#标签不是标题"), [.paragraph("#标签不是标题")])
        XCTAssertEqual(MarkdownParser.parse(""), [])
    }
}
