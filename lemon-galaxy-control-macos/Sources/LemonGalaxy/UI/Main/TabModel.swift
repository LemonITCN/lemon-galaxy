import SwiftUI

enum TabKind { case terminal, sftp, remote }

/// 工作区标签（终端、SFTP、远程桌面）的基类。AI 对话不开标签，在 AI 页面里切换。
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
        }
    }

    func close() {}
}
