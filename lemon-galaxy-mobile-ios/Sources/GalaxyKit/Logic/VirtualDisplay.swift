import Foundation

// 主控「加虚拟屏」的纯逻辑（与界面无关，可在 macOS 上单元测试）：加屏表单的档位与校验、「我加的」集合、删屏后切去哪块。
// 合法范围与内核 galaxy-proto virtual_display_mode_ok 一致；交互与安卓端（RemoteDock.kt 的显示面板）一致。

/// 虚拟屏分辨率（像素）
public struct VirtualDisplaySize: Hashable {
    public let width: UInt32
    public let height: UInt32

    public init(_ width: UInt32, _ height: UInt32) {
        self.width = width
        self.height = height
    }

    /// 「2560×1440」
    public var label: String { "\(width)×\(height)" }
}

/// 加虚拟屏的合法范围（与内核一致，界面不传 0）
public enum VirtualDisplayLimits {
    public static let width: ClosedRange<UInt32> = 640...7680
    public static let height: ClosedRange<UInt32> = 480...4320
    public static let refresh: ClosedRange<UInt32> = 24...240

    public static func sizeOk(_ s: VirtualDisplaySize) -> Bool { width.contains(s.width) && height.contains(s.height) }
    public static func refreshOk(_ r: UInt32) -> Bool { refresh.contains(r) }
}

/// 一次加屏请求（已校验合法）
public struct VirtualDisplayRequest: Equatable {
    public let width: UInt32
    public let height: UInt32
    public let refresh: UInt32

    public init(width: UInt32, height: UInt32, refresh: UInt32) {
        self.width = width
        self.height = height
        self.refresh = refresh
    }
}

/// 加屏表单：分辨率是常用档位 + 本机屏幕 + 自定义（宽、高），刷新率是四档 + 自定义。默认 1920×1080 @ 60。
public struct VirtualDisplayForm: Equatable {
    public enum Resolution: Hashable {
        case preset(VirtualDisplaySize)
        /// 本机屏幕（本机主屏物理像素，横屏方向）
        case local
        case custom
    }

    public enum Refresh: Hashable {
        case preset(UInt32)
        case custom
    }

    public static let sizePresets: [VirtualDisplaySize] = [
        VirtualDisplaySize(1280, 720), VirtualDisplaySize(1920, 1080), VirtualDisplaySize(2560, 1440),
        VirtualDisplaySize(3840, 2160), VirtualDisplaySize(1920, 1200), VirtualDisplaySize(2560, 1600),
    ]
    public static let refreshPresets: [UInt32] = [30, 60, 120, 144]
    public static let defaultSize = VirtualDisplaySize(1920, 1080)
    public static let defaultRefresh: UInt32 = 60
    /// 输入框最多几位（宽高 4 位、刷新率 3 位，再多一定超出范围）
    public static let sizeDigits = 4
    public static let refreshDigits = 3

    /// 本机屏幕档位；不在合法范围时为 nil，不显示这一项
    public let local: VirtualDisplaySize?
    public var resolution: Resolution = .preset(defaultSize)
    public var refresh: Refresh = .preset(defaultRefresh)
    public var customWidth = ""
    public var customHeight = ""
    public var customRefresh = ""

    public init(local: VirtualDisplaySize? = nil) {
        self.local = local
    }

    /// 本机屏幕：物理像素取横屏方向（长边为宽），超出合法范围返回 nil
    public static func localSize(pixelWidth: Double, pixelHeight: Double) -> VirtualDisplaySize? {
        guard pixelWidth.isFinite, pixelHeight.isFinite, pixelWidth >= 1, pixelHeight >= 1,
              max(pixelWidth, pixelHeight) < Double(UInt32.max) else { return nil }
        let long = UInt32(max(pixelWidth, pixelHeight).rounded())
        let short = UInt32(min(pixelWidth, pixelHeight).rounded())
        let s = VirtualDisplaySize(long, short)
        return VirtualDisplayLimits.sizeOk(s) ? s : nil
    }

    /// 分辨率单选的各项（即显示顺序）：常用档位、本机屏幕（合法才有）、自定义
    public var resolutionChoices: [Resolution] {
        Self.sizePresets.map { .preset($0) } + (local == nil ? [] : [.local]) + [.custom]
    }

    /// 刷新率单选的各项：四档、自定义
    public var refreshChoices: [Refresh] {
        Self.refreshPresets.map { .preset($0) } + [.custom]
    }

    /// 选分辨率。选自定义时输入框还空着就先填上当前选中的分辨率，改一改就行（与安卓端一致）
    public mutating func selectResolution(_ r: Resolution) {
        guard r != resolution else { return }
        if r == .custom, customWidth.isEmpty, customHeight.isEmpty, let current = size {
            customWidth = String(current.width)
            customHeight = String(current.height)
        }
        resolution = r
    }

    /// 选刷新率。选自定义时输入框还空着就先填上当前档位
    public mutating func selectRefresh(_ r: Refresh) {
        guard r != refresh else { return }
        if r == .custom, customRefresh.isEmpty, let current = refreshRate {
            customRefresh = String(current)
        }
        refresh = r
    }

    /// 输入框改动（只留数字，宽高最多 4 位、刷新率最多 3 位）
    public mutating func setCustomWidth(_ text: String) { customWidth = Self.digits(text, max: Self.sizeDigits) }
    public mutating func setCustomHeight(_ text: String) { customHeight = Self.digits(text, max: Self.sizeDigits) }
    public mutating func setCustomRefresh(_ text: String) { customRefresh = Self.digits(text, max: Self.refreshDigits) }

    /// 选中的分辨率；自定义没填或不合法时为 nil
    public var size: VirtualDisplaySize? {
        switch resolution {
        case .preset(let s): return s
        case .local: return local
        case .custom:
            guard let w = Self.number(customWidth), let h = Self.number(customHeight) else { return nil }
            let s = VirtualDisplaySize(w, h)
            return VirtualDisplayLimits.sizeOk(s) ? s : nil
        }
    }

    /// 选中的刷新率；自定义没填或不合法时为 nil
    public var refreshRate: UInt32? {
        switch refresh {
        case .preset(let r): return r
        case .custom:
            guard let r = Self.number(customRefresh), VirtualDisplayLimits.refreshOk(r) else { return nil }
            return r
        }
    }

    /// 自定义分辨率不合法（含没填）：按钮置灰，输入框下方说明范围
    public var sizeInvalid: Bool { resolution == .custom && size == nil }
    /// 自定义刷新率不合法（含没填）
    public var refreshInvalid: Bool { refresh == .custom && refreshRate == nil }

    /// 范围说明放在哪组输入框下面：分辨率不合法放分辨率下面；只有刷新率不合法才放刷新率下面（同一句话不出现两次）
    public var rangeHintAfterSize: Bool { sizeInvalid }
    public var rangeHintAfterRefresh: Bool { refreshInvalid && !sizeInvalid }

    /// 范围说明用醒目颜色：已经输入了内容但不合法（还没填时只是提示）
    public var rangeHintIsError: Bool {
        (sizeInvalid && !(customWidth.isEmpty && customHeight.isEmpty)) || (refreshInvalid && !customRefresh.isEmpty)
    }

    /// 合法时的加屏请求
    public var request: VirtualDisplayRequest? {
        guard let s = size, let r = refreshRate else { return nil }
        return VirtualDisplayRequest(width: s.width, height: s.height, refresh: r)
    }

    /// 主按钮能不能点：表单合法、还没加满、不在进行中
    public func canSubmit(quota: VirtualDisplayQuota, busy: Bool) -> Bool {
        request != nil && !quota.full && !busy
    }

    /// 输入框内容：只留数字，最多 maxDigits 位
    public static func digits(_ text: String, max maxDigits: Int) -> String {
        String(text.filter { $0.isASCII && $0.isNumber }.prefix(maxDigits))
    }

    /// 输入框里的数字：只认纯数字（前后空白忽略），空或超出 UInt32 时为 nil
    static func number(_ text: String) -> UInt32? {
        let t = text.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty, t.allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
        return UInt32(t)
    }
}

/// 加屏计数：「已加 n / N」，n >= N 即加满
public struct VirtualDisplayQuota: Equatable {
    public let added: Int
    public let max: Int

    public init(added: Int, max: Int) {
        self.added = added
        self.max = max
    }

    /// 被控端支持且有控制权限（max > 0）才显示入口
    public var available: Bool { max > 0 }
    public var full: Bool { added >= max }
}

/// 本会话加的虚拟屏。DisplayInfo 不带归属，只能主控自己记；删除只对这些开放。会话换了或断开时清空。
public struct MyVirtualDisplays: Equatable {
    public private(set) var ids: [UInt32] = []

    public init() {}

    public var count: Int { ids.count }
    public func contains(_ id: UInt32) -> Bool { ids.contains(id) }

    public mutating func add(_ id: UInt32) {
        if !ids.contains(id) { ids.append(id) }
    }

    public mutating func remove(_ id: UInt32) {
        ids.removeAll { $0 == id }
    }

    public mutating func reset() {
        ids.removeAll()
    }

    /// 屏幕列表刷新后：被控端那边已经没有的（驱动重启、无人值守换会话后消失）不再算我加的，计数随之腾出来
    public mutating func keepOnly(listed: [UInt32]) {
        ids.removeAll { !listed.contains($0) }
    }

    /// 删掉正在看的虚拟屏后切去哪块：主屏，没有主屏就第一块（被删的那块不算）
    public static func fallback(removing id: UInt32, in displays: [DisplayInfo]) -> UInt32? {
        let rest = displays.filter { $0.id != id }
        return (rest.first(where: \.primary) ?? rest.first)?.id
    }
}
