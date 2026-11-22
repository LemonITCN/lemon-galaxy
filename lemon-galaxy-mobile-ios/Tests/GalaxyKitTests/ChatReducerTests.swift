import XCTest
@testable import GalaxyKit

final class ChatReducerTests: XCTestCase {
    func call(_ id: String, risk: RiskLevel = .dangerous) -> ToolCallView {
        ToolCallView(callId: id, tool: "run_command", targetId: "dev", targetName: "build", summary: "rm -rf /tmp/x", detail: "",
                     risk: risk, riskReasons: ["删除文件"], modelReason: "清理", image: nil)
    }

    func state() -> ConversationState {
        ConversationState(id: "c1", title: "", modelName: "M", vision: false, targetNames: ["build"], mode: .confirmDangerous)
    }

    func run(_ s: ConversationState, _ events: [AgentEvent]) -> ConversationState {
        events.reduce(s) { ChatReducer.reduce($0, $1) }
    }

    func testStreamingDeltasMergeIntoOneMessage() {
        let s = run(state(), [.thinking(messageId: "a1", text: "想"), .thinking(messageId: "a1", text: "一想"),
                              .assistantDelta(messageId: "a1", text: "你"), .assistantDelta(messageId: "a1", text: "好"),
                              .assistantDelta(messageId: "a2", text: "第二条")])
        XCTAssertEqual(s.items.count, 2)
        guard case .assistant(_, let t, let th, let streaming) = s.items[0] else { return XCTFail() }
        XCTAssertEqual(t, "你好")
        XCTAssertEqual(th, "想一想")
        XCTAssertTrue(streaming)
        let done = ChatReducer.reduce(s, .assistantDone(messageId: "a1"))
        guard case .assistant(_, _, _, false) = done.items[0] else { return XCTFail("应已结束流式") }
    }

    func testToolLifecycleWithApproval() {
        var s = run(state(), [.status(running: true, step: 1), .toolProposed(call: call("t1"), needsApproval: true)])
        XCTAssertTrue(s.waitingForUser)
        guard case .tool(let t0) = s.items[0] else { return XCTFail() }
        XCTAssertEqual(t0.phase, .awaiting)
        s = ChatReducer.decided(s, callId: "t1", decision: .allowOnce)
        XCTAssertFalse(s.waitingForUser)
        s = run(s, [.toolStarted(callId: "t1"), .toolOutput(callId: "t1", text: "a\n"), .toolOutput(callId: "t1", text: "b\n"),
                    .toolFinished(callId: "t1", result: ToolResultView(ok: true, summary: "退出码 0", output: "a\nb\n", image: nil, denied: false)),
                    .status(running: false, step: 1)])
        guard case .tool(let t) = s.items[0] else { return XCTFail() }
        XCTAssertEqual(t.phase, .done)
        XCTAssertEqual(t.output, "a\nb\n")
        XCTAssertEqual(t.decision, .allowOnce)
        XCTAssertFalse(s.running)
        XCTAssertEqual(s.step, 1)
    }

    func testDeniedAndFailedTools() {
        var s = run(state(), [.toolProposed(call: call("t1"), needsApproval: true), .toolProposed(call: call("t2", risk: .read), needsApproval: false)])
        s = ChatReducer.decided(s, callId: "t1", decision: .deny(reason: "不要删"))
        s = run(s, [.toolFinished(callId: "t1", result: ToolResultView(ok: false, summary: "用户拒绝", output: "", image: nil, denied: true)),
                    .toolFinished(callId: "t2", result: ToolResultView(ok: false, summary: "退出码 1", output: "err", image: nil, denied: false))])
        guard case .tool(let t1) = s.items[0], case .tool(let t2) = s.items[1] else { return XCTFail() }
        XCTAssertEqual(t1.phase, .denied)
        XCTAssertEqual(t2.phase, .failed)
        XCTAssertEqual(t2.output, "err")
        // 没确认就直接执行的工具：提出时就是运行中
        let auto = run(state(), [.toolProposed(call: call("t3", risk: .read), needsApproval: false)])
        guard case .tool(let t3) = auto.items[0] else { return XCTFail() }
        XCTAssertEqual(t3.phase, .running)
    }

    func testAskSecretUsageModeAndError() {
        var s = run(state(), [.askUser(questionId: "q1", question: "继续吗？", options: ["是", "否"]),
                              .secretRequested(requestId: "s1", prompt: "sudo 密码", targetId: "dev"),
                              .usage(promptTokens: 1200, completionTokens: 34),
                              .modeChanged(mode: .fullTrust, effective: .confirmAll, untilMs: 123),
                              .error(message: "模型超时"), .error(message: "又超时")])
        XCTAssertTrue(s.waitingForUser)
        XCTAssertEqual(s.promptTokens, 1200)
        XCTAssertEqual(s.mode, .confirmAll)
        XCTAssertEqual(s.trustUntilMs, 123)
        XCTAssertEqual(Set(s.items.map(\.id)).count, s.items.count, "条目 id 不能重复")
        s = ChatReducer.answered(s, questionId: "q1", answer: "是")
        s = ChatReducer.secretProvided(s, requestId: "s1")
        XCTAssertFalse(s.waitingForUser)
        guard case .ask(_, _, _, let a) = s.items[0] else { return XCTFail() }
        XCTAssertEqual(a, "是")
        // 重复的提问事件不重复加
        XCTAssertEqual(ChatReducer.reduce(s, .askUser(questionId: "q1", question: "继续吗？", options: [])).items.count, s.items.count)
    }

    func testUserSendSetsTitleOnce() {
        var s = ChatReducer.userSent(state(), text: "帮我看看磁盘")
        XCTAssertEqual(s.title, "帮我看看磁盘")
        XCTAssertTrue(s.running)
        s = ChatReducer.userSent(s, text: "再看看内存")
        XCTAssertEqual(s.title, "帮我看看磁盘")
        XCTAssertEqual(s.items.count, 2)
        XCTAssertNotEqual(s.items[0].id, s.items[1].id)
    }

    func testHistoryMapping() {
        let items = ChatReducer.items(fromHistory: [
            .user(text: "hi", refs: [.device("d1", label: "build")]), .assistant(id: "", text: "hello", thinking: "t"),
            .tool(call: call("t1"), result: ToolResultView(ok: true, summary: "", output: "out", image: nil, denied: false)),
            .tool(call: call("t2"), result: nil),
        ])
        XCTAssertEqual(items.count, 4)
        guard case .tool(let done) = items[2], case .tool(let pending) = items[3] else { return XCTFail() }
        XCTAssertEqual(done.phase, .done)
        XCTAssertEqual(done.output, "out")
        XCTAssertEqual(pending.phase, .failed)
        guard case .assistant(let k, _, let th, false) = items[1] else { return XCTFail() }
        XCTAssertEqual(k, "h-a-1")
        XCTAssertEqual(th, "t")
    }
}
