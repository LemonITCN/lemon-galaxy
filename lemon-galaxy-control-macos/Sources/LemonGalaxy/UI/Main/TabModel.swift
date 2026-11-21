import SwiftUI

enum TabKind { case terminal, sftp, remote, ai }

/// 工作区标签（终端、SFTP、远程桌面、AI 对话）的基类。
@MainActor
class TabModel: ObservableObject, Identifiable {
    let id = UUID().uuidString
    let kind: TabKind
    @Published var title: String
    /// 标签上的状态点：nil 不显示，true 正常，false 断开
    @Published var live: Bool?
    var deviceId: String?
    weak var app: AppModel?

    init(app: AppModel, kind: TabKind, title: String) {
        self.app = app
        self.kind = kind
        self.title = title
    }

    var icon: String {
        switch kind {
        case .terminal: return "terminal"
        case .sftp: return "folder"
        case .remote: return "display"
        case .ai: return "wand.and.stars"
        }
    }

    func close() {}
}
