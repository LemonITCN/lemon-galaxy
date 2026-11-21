import SwiftUI
import GalaxyKit

/// 临时协助：被控端没登录账号时显示「设备码 + 验证码」，输入后等对方本人点允许。
struct AssistConnectView: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var deviceCode = ""
    @State private var code = ""
    @State private var connecting = false
    @State private var error: String?
    @State private var connected: String?
    @State private var task: Task<Void, Never>?

    var body: some View {
        NavigationStack {
            Form {
                if let id = connected {
                    Section {
                        Label(loc("assist.connected", app.account.deviceName(id)), systemImage: "checkmark.circle.fill").foregroundStyle(p.success)
                    }
                    Section {
                        Button { dismiss(); app.openRemoteDesktop(id) } label: { Label(loc("action.remoteDesktop"), systemImage: "display") }
                        Button { dismiss(); app.openTerminal(.device(id), ask: false) } label: { Label(loc("action.terminal"), systemImage: "terminal") }
                        Button { dismiss(); app.openFiles(.device(id)) } label: { Label(loc("action.files"), systemImage: "folder") }
                    }
                    .galaxyRow()
                } else {
                    Section {
                        TextField(loc("assist.deviceCode"), text: $deviceCode)
                            .keyboardType(.numberPad)
                            .font(.title3.monospacedDigit())
                        TextField(loc("assist.verification"), text: $code)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .font(.title3.monospaced())
                    } footer: {
                        HStack { Spacer(); InfoButton(text: loc("assist.info"), size: 13) }
                    }
                    .galaxyRow()
                    if connecting {
                        Section {
                            HStack(spacing: 12) {
                                ProgressView()
                                Text(loc("assist.waiting")).foregroundStyle(p.secondaryText)
                            }
                        }
                        .galaxyRow()
                    }
                    if let error {
                        Section { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(p.danger) }.galaxyRow()
                    }
                }
            }
            .galaxyList()
            .navigationTitle(loc("assist.title"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(loc("common.cancel")) {
                        task?.cancel()
                        dismiss()
                    }
                }
                if connected == nil {
                    ToolbarItem(placement: .confirmationAction) {
                        Button(loc("assist.connect")) { connect() }
                            .disabled(connecting || deviceCode.filter(\.isNumber).count != 9 || code.trimmingCharacters(in: .whitespaces).count < 6)
                            .fontWeight(.semibold)
                    }
                }
            }
        }
    }

    private func connect() {
        connecting = true
        error = nil
        // 设备码是 9 位数字，允许用户按 3-3-3 分组输入
        let dc = deviceCode.filter(\.isNumber)
        let vc = code.trimmingCharacters(in: .whitespaces)
        task = Task {
            do {
                connected = try await app.connectAssist(deviceCode: dc, code: vc)
            } catch {
                if !isCancellation(error) { self.error = describeError(error) }
            }
            connecting = false
        }
    }
}

/// SSH 认证：密码 / 口令 / 键盘交互的回答，以及主机密钥确认（密钥变了要醒目警告）。
struct SecretPromptSheet: View {
    let request: SecretRequest
    @EnvironmentObject var app: AppModel
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var value = ""

    var body: some View {
        NavigationStack {
            Form {
                switch request {
                case .ask(_, let title, let prompt, let echo, _):
                    Section {
                        Text(prompt).font(.callout).foregroundStyle(p.text)
                        if echo {
                            TextField(loc("secret.answer"), text: $value).textInputAutocapitalization(.never).autocorrectionDisabled()
                        } else {
                            SecureField(loc("secret.hint"), text: $value).textContentType(.password)
                        }
                    } header: {
                        Text(title)
                    } footer: {
                        HStack { Spacer(); InfoButton(text: loc("ssh.secretInfo"), size: 13) }
                    }
                    .galaxyRow()
                case .hostKey(_, let host, let port, let type, let fp, let changed, _):
                    Section {
                        if changed {
                            Label(loc("hostKey.changedWarning"), systemImage: "exclamationmark.octagon.fill")
                                .foregroundStyle(p.danger).font(.callout.weight(.semibold))
                        }
                        LabeledContent(loc("field.host"), value: port == 22 ? host : "\(host):\(port)")
                        LabeledContent(loc("hostKey.type"), value: type)
                        Text(fp).font(.footnote.monospaced()).foregroundStyle(p.secondaryText).textSelection(.enabled)
                    } footer: {
                        HStack { Spacer(); InfoButton(text: loc(changed ? "hostKey.changedInfo" : "hostKey.info"), size: 13) }
                    }
                    .galaxyRow()
                }
            }
            .galaxyList()
            .navigationTitle(navTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(loc("common.cancel")) { cancel() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    switch request {
                    case .ask: Button(loc("common.ok")) { app.answerSecret(value) }.fontWeight(.semibold)
                    case .hostKey(_, _, _, _, _, let changed, _):
                        Button(loc(changed ? "hostKey.trustAnyway" : "hostKey.trust")) { app.answerHostKey(true) }
                            .fontWeight(.semibold)
                            .tint(changed ? p.danger : p.accent)
                    }
                }
            }
        }
        .interactiveDismissDisabled()
    }

    private var navTitle: String {
        switch request {
        case .ask: return loc("secret.title")
        case .hostKey(_, _, _, _, _, let changed, _): return loc(changed ? "hostKey.changedTitle" : "hostKey.title")
        }
    }

    private func cancel() {
        switch request {
        case .ask: app.answerSecret(nil)
        case .hostKey: app.answerHostKey(false)
        }
    }
}
