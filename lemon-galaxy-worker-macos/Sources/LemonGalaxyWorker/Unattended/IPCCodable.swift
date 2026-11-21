import Foundation

// 内核类型（uniffi 生成，真内核与 BridgeMirror 逐字相同）在本机进程之间传递时的 JSON 编码，与消息正文结构。
// 内核类型在别的文件里声明，不能自动合成 Codable，这里逐个手写；字段名与内核一致。

/// 不带参数的枚举按字符串编码
private func ipcDecodeEnum<T>(_ d: Decoder, _ cases: [(String, T)]) throws -> T {
    let s = try d.singleValueContainer().decode(String.self)
    guard let v = cases.first(where: { $0.0 == s })?.1 else {
        throw DecodingError.dataCorrupted(.init(codingPath: d.codingPath, debugDescription: "未知取值 \(s)"))
    }
    return v
}

private func ipcEncodeEnum<T: Equatable>(_ e: Encoder, _ v: T, _ cases: [(String, T)]) throws {
    var c = e.singleValueContainer()
    try c.encode(cases.first(where: { $0.1 == v })?.0 ?? "")
}

extension VideoCodec: Codable {
    fileprivate static let ipcCases: [(String, VideoCodec)] = [("h264", .h264), ("h265", .h265)]
    public init(from d: Decoder) throws { self = try ipcDecodeEnum(d, Self.ipcCases) }
    public func encode(to e: Encoder) throws { try ipcEncodeEnum(e, self, Self.ipcCases) }
}

extension PointerButton: Codable {
    fileprivate static let ipcCases: [(String, PointerButton)] = [("left", .left), ("right", .right), ("middle", .middle), ("back", .back), ("forward", .forward)]
    public init(from d: Decoder) throws { self = try ipcDecodeEnum(d, Self.ipcCases) }
    public func encode(to e: Encoder) throws { try ipcEncodeEnum(e, self, Self.ipcCases) }
}

extension TouchPhase: Codable {
    fileprivate static let ipcCases: [(String, TouchPhase)] = [("began", .began), ("moved", .moved), ("ended", .ended), ("cancelled", .cancelled)]
    public init(from d: Decoder) throws { self = try ipcDecodeEnum(d, Self.ipcCases) }
    public func encode(to e: Encoder) throws { try ipcEncodeEnum(e, self, Self.ipcCases) }
}

extension ComboKey: Codable {
    fileprivate static let ipcCases: [(String, ComboKey)] = [("ctrlAltDel", .ctrlAltDel), ("lockScreen", .lockScreen), ("altTab", .altTab), ("winKey", .winKey), ("taskManager", .taskManager), ("showDesktop", .showDesktop)]
    public init(from d: Decoder) throws { self = try ipcDecodeEnum(d, Self.ipcCases) }
    public func encode(to e: Encoder) throws { try ipcEncodeEnum(e, self, Self.ipcCases) }
}

extension AiMode: Codable {
    fileprivate static let ipcCases: [(String, AiMode)] = [("off", .off), ("readOnly", .readOnly), ("confirmAll", .confirmAll), ("confirmDangerous", .confirmDangerous), ("fullTrust", .fullTrust)]
    public init(from d: Decoder) throws { self = try ipcDecodeEnum(d, Self.ipcCases) }
    public func encode(to e: Encoder) throws { try ipcEncodeEnum(e, self, Self.ipcCases) }
}

extension AccountStatus: Codable {
    fileprivate static let ipcCases: [(String, AccountStatus)] = [("active", .active), ("needsRelogin", .needsRelogin)]
    public init(from d: Decoder) throws { self = try ipcDecodeEnum(d, Self.ipcCases) }
    public func encode(to e: Encoder) throws { try ipcEncodeEnum(e, self, Self.ipcCases) }
}

extension EventLevel: Codable {
    fileprivate static let ipcCases: [(String, EventLevel)] = [("info", .info), ("warning", .warning)]
    public init(from d: Decoder) throws { self = try ipcDecodeEnum(d, Self.ipcCases) }
    public func encode(to e: Encoder) throws { try ipcEncodeEnum(e, self, Self.ipcCases) }
}

extension ConnMode: Codable {
    fileprivate static let ipcCases: [(String, ConnMode)] = [("control", .control), ("assist", .assist)]
    public init(from d: Decoder) throws { self = try ipcDecodeEnum(d, Self.ipcCases) }
    public func encode(to e: Encoder) throws { try ipcEncodeEnum(e, self, Self.ipcCases) }
}

extension PathKind: Codable {
    fileprivate static let ipcCases: [(String, PathKind)] = [("connecting", .connecting), ("lan", .lan), ("direct", .direct), ("relayUdp", .relayUdp), ("relayTcp", .relayTcp), ("closed", .closed)]
    public init(from d: Decoder) throws { self = try ipcDecodeEnum(d, Self.ipcCases) }
    public func encode(to e: Encoder) throws { try ipcEncodeEnum(e, self, Self.ipcCases) }
}

/// 手写 Codable 用的键
struct IPCKey: CodingKey {
    var stringValue: String
    var intValue: Int? { nil }
    init(_ s: String) { stringValue = s }
    init?(stringValue: String) { self.stringValue = stringValue }
    init?(intValue: Int) { nil }
}

private typealias KC = KeyedDecodingContainer<IPCKey>
private typealias KE = KeyedEncodingContainer<IPCKey>

private extension KeyedDecodingContainer where K == IPCKey {
    func get<T: Decodable>(_ k: String) throws -> T { try decode(T.self, forKey: IPCKey(k)) }
    func opt<T: Decodable>(_ k: String) throws -> T? { try decodeIfPresent(T.self, forKey: IPCKey(k)) }
}

private extension KeyedEncodingContainer where K == IPCKey {
    mutating func put<T: Encodable>(_ v: T, _ k: String) throws { try encode(v, forKey: IPCKey(k)) }
    mutating func opt<T: Encodable>(_ v: T?, _ k: String) throws { try encodeIfPresent(v, forKey: IPCKey(k)) }
}

// MARK: - 画面相关

extension DisplayInfo: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(id: try c.get("id"), name: try c.get("name"), width: try c.get("width"), height: try c.get("height"),
                  scale: try c.get("scale"), primary: try c.get("primary"), isVirtual: try c.get("isVirtual"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(id, "id"); try c.put(name, "name"); try c.put(width, "width"); try c.put(height, "height")
        try c.put(scale, "scale"); try c.put(primary, "primary"); try c.put(isVirtual, "isVirtual")
    }
}

extension CaptureOptions: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(maxFps: try c.get("maxFps"), bitrateKbps: try c.get("bitrateKbps"), codecs: try c.get("codecs"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(maxFps, "maxFps"); try c.put(bitrateKbps, "bitrateKbps"); try c.put(codecs, "codecs")
    }
}

extension CaptureStarted: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(width: try c.get("width"), height: try c.get("height"), codec: try c.get("codec"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(width, "width"); try c.put(height, "height"); try c.put(codec, "codec")
    }
}

extension ImageData: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(mime: try c.get("mime"), data: try c.get("data"), width: try c.get("width"), height: try c.get("height"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(mime, "mime"); try c.put(data, "data"); try c.put(width, "width"); try c.put(height, "height")
    }
}

extension ClipboardData: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        switch try c.get("t") as String {
        case "text": self = .text(text: try c.get("text"))
        case "image": self = .image(image: try c.get("image"))
        case "files": self = .files(paths: try c.get("paths"))
        default: throw DecodingError.dataCorrupted(.init(codingPath: d.codingPath, debugDescription: "未知剪贴板类型"))
        }
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        switch self {
        case .text(let t): try c.put("text", "t"); try c.put(t, "text")
        case .image(let i): try c.put("image", "t"); try c.put(i, "image")
        case .files(let p): try c.put("files", "t"); try c.put(p, "paths")
        }
    }
}

extension InputEvent: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        switch try c.get("t") as String {
        case "move": self = .mouseMove(x: try c.get("x"), y: try c.get("y"))
        case "button": self = .mouseButton(button: try c.get("b"), down: try c.get("down"), x: try c.get("x"), y: try c.get("y"))
        case "wheel": self = .wheel(dx: try c.get("dx"), dy: try c.get("dy"))
        case "key": self = .key(hidUsage: try c.get("hid"), down: try c.get("down"))
        case "text": self = .text(text: try c.get("text"))
        case "touch": self = .touch(id: try c.get("id"), phase: try c.get("phase"), x: try c.get("x"), y: try c.get("y"))
        case "combo": self = .specialCombo(combo: try c.get("combo"))
        default: throw DecodingError.dataCorrupted(.init(codingPath: d.codingPath, debugDescription: "未知输入事件"))
        }
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        switch self {
        case .mouseMove(let x, let y): try c.put("move", "t"); try c.put(x, "x"); try c.put(y, "y")
        case .mouseButton(let b, let down, let x, let y):
            try c.put("button", "t"); try c.put(b, "b"); try c.put(down, "down"); try c.put(x, "x"); try c.put(y, "y")
        case .wheel(let dx, let dy): try c.put("wheel", "t"); try c.put(dx, "dx"); try c.put(dy, "dy")
        case .key(let hid, let down): try c.put("key", "t"); try c.put(hid, "hid"); try c.put(down, "down")
        case .text(let t): try c.put("text", "t"); try c.put(t, "text")
        case .touch(let id, let phase, let x, let y):
            try c.put("touch", "t"); try c.put(id, "id"); try c.put(phase, "phase"); try c.put(x, "x"); try c.put(y, "y")
        case .specialCombo(let combo): try c.put("combo", "t"); try c.put(combo, "combo")
        }
    }
}

// MARK: - 被控服务状态

extension HostPolicy: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(systemUser: try c.get("systemUser"), view: try c.get("view"), control: try c.get("control"), files: try c.get("files"),
                  terminal: try c.get("terminal"), clipboard: try c.get("clipboard"), audio: try c.get("audio"),
                  aiCap: try c.get("aiCap"), notify: try c.get("notify"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(systemUser, "systemUser"); try c.put(view, "view"); try c.put(control, "control"); try c.put(files, "files")
        try c.put(terminal, "terminal"); try c.put(clipboard, "clipboard"); try c.put(audio, "audio")
        try c.put(aiCap, "aiCap"); try c.put(notify, "notify")
    }
}

extension HostAccount: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(userId: try c.get("userId"), username: try c.get("username"), displayName: try c.get("displayName"),
                  status: try c.get("status"), policy: try c.get("policy"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(userId, "userId"); try c.put(username, "username"); try c.put(displayName, "displayName")
        try c.put(status, "status"); try c.put(policy, "policy")
    }
}

extension HostSessionInfo: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(id: try c.get("id"), fromUser: try c.get("fromUser"), fromDevice: try c.get("fromDevice"), mode: try c.get("mode"),
                  path: try c.get("path"), startedAt: try c.get("startedAt"), aiActive: try c.get("aiActive"), accountId: try c.opt("accountId"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(id, "id"); try c.put(fromUser, "fromUser"); try c.put(fromDevice, "fromDevice"); try c.put(mode, "mode")
        try c.put(path, "path"); try c.put(startedAt, "startedAt"); try c.put(aiActive, "aiActive"); try c.opt(accountId, "accountId")
    }
}

extension AssistInfo: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(deviceCode: try c.get("deviceCode"), code: try c.get("code"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(deviceCode, "deviceCode"); try c.put(code, "code")
    }
}

extension HostStatus: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(online: try c.get("online"), remoteEnabled: try c.get("remoteEnabled"), deviceId: try c.get("deviceId"),
                  publicIp: try c.get("publicIp"), accounts: try c.get("accounts"), sessions: try c.get("sessions"), assist: try c.opt("assist"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(online, "online"); try c.put(remoteEnabled, "remoteEnabled"); try c.put(deviceId, "deviceId")
        try c.put(publicIp, "publicIp"); try c.put(accounts, "accounts"); try c.put(sessions, "sessions"); try c.opt(assist, "assist")
    }
}

extension HostEvent: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(at: try c.get("at"), session: try c.get("session"), user: try c.get("user"), kind: try c.get("kind"),
                  detail: try c.get("detail"), ai: try c.get("ai"), level: try c.get("level"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(at, "at"); try c.put(session, "session"); try c.put(user, "user"); try c.put(kind, "kind")
        try c.put(detail, "detail"); try c.put(ai, "ai"); try c.put(level, "level")
    }
}

extension UpdateInfo: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        self.init(version: try c.get("version"), fileName: try c.get("fileName"), size: try c.get("size"),
                  sha256: try c.get("sha256"), url: try c.get("url"), notes: try c.get("notes"))
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        try c.put(version, "version"); try c.put(fileName, "fileName"); try c.put(size, "size")
        try c.put(sha256, "sha256"); try c.put(url, "url"); try c.put(notes, "notes")
    }
}

extension WorkerLoginResult: Codable {
    public init(from d: Decoder) throws {
        let c = try d.container(keyedBy: IPCKey.self)
        switch try c.get("t") as String {
        case "success": self = .success(userId: try c.get("userId"), username: try c.get("username"))
        case "needTotp": self = .needTotp
        case "mustChangePassword": self = .mustChangePassword
        default: throw DecodingError.dataCorrupted(.init(codingPath: d.codingPath, debugDescription: "未知登录结果"))
        }
    }
    public func encode(to e: Encoder) throws {
        var c = e.container(keyedBy: IPCKey.self)
        switch self {
        case .success(let id, let name): try c.put("success", "t"); try c.put(id, "userId"); try c.put(name, "username")
        case .needTotp: try c.put("needTotp", "t")
        case .mustChangePassword: try c.put("mustChangePassword", "t")
        }
    }
}

// MARK: - 消息正文：后台服务 ↔ 会话代理（平台层转发）

enum AgentSessionType: String, Codable {
    /// 登录界面（开机还没人登录、注销后、快速切换用户时的登录窗口），代理以 root 运行
    case loginWindow
    /// 已登录用户的图形会话，代理以该用户运行
    case aqua
}

struct AgentHello: Codable, Equatable {
    var sessionType: AgentSessionType
    var uid: UInt32
    var userName: String
    /// 安全会话号（审计会话号），同一会话里只留一个代理
    var sessionId: UInt32
    var pid: Int32
    var onConsole: Bool
    var version: String
    var build: String
    var capabilities: [String]
}

struct AgentConsoleState: Codable, Equatable {
    var onConsole: Bool
    var capabilities: [String]
}

struct PDisplay: Codable { var display: UInt32 }
struct PStartCapture: Codable { var display: UInt32; var options: CaptureOptions }
struct PStarted: Codable { var started: CaptureStarted? }
struct PBitrate: Codable { var display: UInt32; var kbps: UInt32 }
struct PInject: Codable { var display: UInt32; var event: InputEvent }
struct PScreenshot: Codable { var display: UInt32; var maxWidth: UInt32; var maxHeight: UInt32 }
struct PImageMeta: Codable { var mime: String; var width: UInt32; var height: UInt32 }
struct PUiTree: Codable { var display: UInt32; var maxNodes: UInt32 }
struct PText: Codable { var text: String? }
struct PClipboard: Codable { var data: ClipboardData? }
struct POn: Codable { var on: Bool }
struct PBool: Codable { var value: Bool }
struct PStrings: Codable { var values: [String] }
struct AFrame: Codable { var display: UInt32; var pts: UInt64; var key: Bool }
struct AResolution: Codable { var display: UInt32; var width: UInt32; var height: UInt32 }
struct AAudio: Codable { var rate: UInt32; var channels: UInt32 }
struct ALog: Codable { var text: String; var error: Bool }

// MARK: - 消息正文：后台服务 ↔ 菜单栏应用（界面）

struct ControlHello: Codable {
    var version: String
    var build: String
    var locale: String
}

struct AgentSummary: Codable, Equatable {
    var sessionType: AgentSessionType
    var userName: String
    var uid: UInt32
    var onConsole: Bool
    var active: Bool
    var version: String
}

/// 后台服务的概况（连上时与变化时推给界面）
struct DaemonInfo: Codable, Equatable {
    var version: String
    var build: String
    var coreVersion: String
    var deviceId: String
    var server: String?
    var hasService: Bool
    /// 已经有服务器或账号（不能再导入别的设备身份）
    var claimed: Bool
    var peerAdmin: Bool
    var peerConsole: Bool
    var peerUser: String
    var agents: [AgentSummary]
    var isMock: Bool

    var activeAgent: AgentSummary? { agents.first { $0.active } }
}

struct CLogin: Codable {
    var server: String
    var username: String
    var password: String
    var totp: String?
    var newPassword: String?
}
struct CLoginResult: Codable { var result: WorkerLoginResult; var info: DaemonInfo }
struct CUser: Codable { var userId: UInt64 }
struct CServer: Codable { var server: String }
struct CInfo: Codable { var info: DaemonInfo }
struct CUpdateCheck: Codable { var platform: String; var current: String }
struct CUpdateInfo: Codable { var info: UpdateInfo? }
struct CDownload: Codable { var token: String; var info: UpdateInfo }
struct CDownloadEvent: Codable { var token: String; var done: UInt64; var total: UInt64; var finished: Bool; var error: String?; var path: String? }
struct CLimit: Codable { var limit: UInt32 }
struct CEvents: Codable { var events: [HostEvent] }
struct CEnabled: Codable { var enabled: Bool }
struct CId: Codable { var id: String }
struct CAssistResponse: Codable { var requestId: String; var allow: Bool }
struct CCode: Codable { var code: String? }
struct CAssist: Codable { var info: AssistInfo }
struct CPolicy: Codable { var userId: UInt64; var policy: HostPolicy }
struct CLocale: Codable { var locale: String }
struct CStatus: Codable { var status: HostStatus }
struct LAssistRequest: Codable { var requestId: String; var fromUser: String; var fromDevice: String }
struct LAiActivity: Codable { var sessionId: String; var description: String }

/// 移交：设备身份 + 数据目录里的文件（账号令牌、事件日志等）
struct MigrationBundle: Codable, Equatable {
    struct File: Codable, Equatable {
        var path: String
        var data: Data
    }
    var deviceId: Data
    var deviceKey: Data
    var files: [File]
    var sourceUser: String
}
struct CMigration: Codable { var bundle: MigrationBundle }
struct CRelease: Codable { var wipe: Bool }

/// 方法名
enum IPCMethod {
    // 代理 → 后台服务
    static let agentHello = "a.hello"
    static let agentConsole = "a.console"
    static let agentFrame = "a.frame"
    static let agentResolution = "a.resolution"
    static let agentAudio = "a.audio"
    static let agentLog = "a.log"
    // 后台服务 → 代理
    static let displays = "p.displays"
    static let startCapture = "p.startCapture"
    static let stopCapture = "p.stopCapture"
    static let keyframe = "p.keyframe"
    static let bitrate = "p.bitrate"
    static let inject = "p.inject"
    static let screenshot = "p.screenshot"
    static let uiTree = "p.uiTree"
    static let clipboardGet = "p.clipboardGet"
    static let clipboardSet = "p.clipboardSet"
    static let privacy = "p.privacy"
    static let startAudio = "p.startAudio"
    static let stopAudio = "p.stopAudio"
    static let capabilities = "p.capabilities"
    static let stopAll = "p.stopAll"
    static let agentExit = "p.exit"
    // 界面 → 后台服务
    static let hello = "c.hello"
    static let setLocale = "c.setLocale"
    static let exportData = "c.export"
    static let importData = "c.import"
    static let release = "c.release"
    static let restart = "c.restart"
    static let login = "w.login"
    static let logout = "w.logout"
    static let setServer = "w.setServer"
    static let start = "w.start"
    static let checkUpdate = "w.checkUpdate"
    static let downloadUpdate = "w.downloadUpdate"
    static let status = "s.status"
    static let events = "s.events"
    static let lockOnEnd = "s.lockOnEnd"
    static let setLockOnEnd = "s.setLockOnEnd"
    static let setRemoteEnabled = "s.setRemoteEnabled"
    static let disconnectAll = "s.disconnectAll"
    static let disconnectSession = "s.disconnectSession"
    static let respondAssist = "s.respondAssist"
    static let setAssistCode = "s.setAssistCode"
    static let startAssist = "s.startAssist"
    static let stopAssist = "s.stopAssist"
    static let stopAi = "s.stopAi"
    static let updatePolicy = "s.updatePolicy"
    // 后台服务 → 界面（推送）
    static let pushInfo = "l.info"
    static let pushStatus = "l.status"
    static let pushAssist = "l.assist"
    static let pushStarted = "l.started"
    static let pushEnded = "l.ended"
    static let pushAi = "l.ai"
    static let pushDownload = "l.download"
}
