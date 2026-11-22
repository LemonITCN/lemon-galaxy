import SwiftUI
import UIKit
import GalaxyKit

struct RemoteCanvasHost: UIViewRepresentable {
    let session: RemoteSession
    func makeUIView(context: Context) -> UIView {
        let v = UIView()
        v.backgroundColor = .black
        return v
    }
    func updateUIView(_ container: UIView, context: Context) {
        if session.canvas.superview !== container {
            container.subviews.forEach { $0.removeFromSuperview() }
            session.canvas.frame = container.bounds
            session.canvas.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            container.addSubview(session.canvas)
        }
    }
}

/// 右侧菜单的二级面板
enum RemotePanel: Equatable {
    case display, shortcuts, tools
}

/// 右侧菜单里要由画面层处理的操作（其余直接调 RemoteSession）
struct DockActions {
    let switchDisplay: (Int) -> Void
    let pickDisplay: (UInt32) -> Void
    let keyboard: () -> Void
    let mouseMode: () -> Void
    let fit: () -> Void
    let actualSize: () -> Void
    /// 只有 iPhone 有（横屏 / 竖屏切换）
    let rotate: ((_ portraitNow: Bool) -> Void)?
    let files: () -> Void
    let disconnect: () -> Void
    /// iPhone 全屏层：收起画面回到列表（会话照常运行）
    let minimize: (() -> Void)?
}

/// 远程桌面：画面占满（始终保持被控屏幕比例）+ 手势 + 右侧贴边菜单（与安卓端一致）。
/// 收起时是贴右边缘的把手：轻点展开，上下滑动切换显示器，长按后拖动改位置；
/// 展开是右侧竖排菜单，二级面板（显示器、快捷键、工具）从菜单左侧弹出。
struct RemoteScreen: View {
    @ObservedObject var session: RemoteSession
    var onMinimize: (() -> Void)?
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) private var loc
    @State private var expanded = false
    @State private var panel: RemotePanel?

    var body: some View {
        ZStack {
            // 只铺满容器安全区以外的区域：弹出键盘时画面让出键盘的位置（保持倍数与中心）
            RemoteCanvasHost(session: session)
                .ignoresSafeArea(.container)
            statusOverlay
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            recordingBadge
            RemoteNoticeLayer(hud: session.hud)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                .padding(.top, 16)
            RemoteDock(session: session, actions: dockActions, expanded: $expanded, panel: $panel)
        }
        .background(Color.black.ignoresSafeArea())
        .environment(\.palette, Palettes.make("galaxy", dark: true))
        .statusBarHidden(true)
        .persistentSystemOverlays(.hidden)
        .onAppear { session.canvas.onHideKeyboard = { setKeyboard(false) } }
        .onDisappear {
            // 离开画面（收起、切到别的会话）就停止录像，并解除方向锁定
            session.stopRecording(.left)
            OrientationLock.release()
        }
        .modifier(RecordingSavedAlert(session: session))
    }

    private func setKeyboard(_ on: Bool) {
        session.keyboardShown = on
        if on { _ = session.canvas.becomeFirstResponder() } else { _ = session.canvas.resignFirstResponder() }
    }

    /// 菜单收起时左上角一直显示录像中（红点 + 计时）
    @ViewBuilder private var recordingBadge: some View {
        if let start = session.recordingStartedAt, !expanded {
            RecordingBadge(start: start)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .padding(12)
        }
    }

    @ViewBuilder private var statusOverlay: some View {
        switch session.status {
        case .streaming:
            EmptyView()
        case .connecting:
            VStack(spacing: 12) {
                ProgressView().tint(.white)
                Text(loc("remote.connecting")).font(.subheadline).foregroundStyle(.white.opacity(0.8))
            }
        case .failed(let m):
            failedView(m)
        }
    }

    private func failedView(_ m: String) -> some View {
        VStack(spacing: 14) {
            Image(systemName: "display.trianglebadge.exclamationmark").font(.system(size: 34)).foregroundStyle(.white.opacity(0.8))
            Text(m.isEmpty ? loc("remote.failed") : m).font(.subheadline).foregroundStyle(.white).multilineTextAlignment(.center)
            HStack {
                Button(loc("common.retry")) { session.retry() }.buttonStyle(.borderedProminent)
                Button(loc("remote.disconnect")) { app.sessions.close(session.id) }.buttonStyle(.bordered).tint(.white)
            }
        }
        .padding(24)
        .frame(maxWidth: 340)
    }

    // MARK: 菜单操作

    private var dockActions: DockActions {
        DockActions(
            switchDisplay: switchDisplay,
            pickDisplay: pickDisplay,
            keyboard: toggleKeyboard,
            mouseMode: toggleMouseMode,
            fit: { session.canvas.fitToScreen() },
            actualSize: { session.canvas.actualSize() },
            rotate: OrientationLock.supported ? { OrientationLock.toggle(portraitNow: $0) } : nil,
            files: { app.openFiles(.device(session.deviceId)) },
            disconnect: { app.sessions.close(session.id) },
            minimize: onMinimize
        )
    }

    /// 循环切换：两块屏时上滑下滑都能换过去
    private func switchDisplay(_ step: Int) {
        let ids = session.displays.map(\.id)
        guard let next = DisplayCycle.next(after: session.displayId, in: ids, step: step) else { return }
        pickDisplay(next)
    }

    private func pickDisplay(_ id: UInt32) {
        if let d = session.displays.first(where: { $0.id == id }) { showDisplay(d) }
        session.switchDisplay(id)
    }

    /// 「显示器 2 / 3」+「名称 · 分辨率 · 主屏」
    private func showDisplay(_ d: DisplayInfo) {
        let ids = session.displays.map(\.id)
        let i = DisplayCycle.index(of: d.id, in: ids)
        var sub: String = d.name + " · \(d.width)×\(d.height)"
        if d.primary { sub += " · " + loc("remote.primary") }
        session.hud.show(loc("remote.displayN", i + 1, ids.count), sub)
    }

    private func toggleKeyboard() {
        setKeyboard(!session.keyboardShown)
        expanded = false
        panel = nil
    }

    private func toggleMouseMode() {
        let on = !session.mouseMode
        session.mouseMode = on
        app.settings.remoteMouseMode = on
        session.hud.show(loc(on ? "remote.modeMouse" : "remote.modeTouch"), loc(on ? "remote.modeMouseHint" : "remote.modeTouchHint"))
    }
}
