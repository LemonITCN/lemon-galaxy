import SwiftUI
import Photos
import GalaxyKit

/// 录像停止后的「已保存」提示：分享（系统分享面板里也有「存储到文件」）、存储到相册。
struct RecordingSavedAlert: ViewModifier {
    @ObservedObject var session: RemoteSession
    @EnvironmentObject var app: AppModel
    @Environment(\.loc) private var loc
    @State private var share: SavedRecording?

    func body(content: Content) -> some View {
        content
            .alert(loc("remote.recordSaved"), isPresented: shown, presenting: session.savedRecording) { rec in
                Button(loc("common.share")) { share = rec }
                Button(loc("remote.saveToPhotos")) { RecordingExport.saveToPhotos(rec.urls, app: app) }
                Button(loc("common.ok"), role: .cancel) {}
            } message: { rec in
                Text(message(rec))
            }
            .sheet(item: $share) { rec in
                ShareSheet(items: rec.urls)
            }
    }

    private var shown: Binding<Bool> {
        Binding(get: { session.savedRecording != nil }, set: { if !$0 { session.savedRecording = nil } })
    }

    private func message(_ rec: SavedRecording) -> String {
        let length: String = Fmt.elapsed(Int(rec.durationMs / 1000))
        return loc("remote.recordSavedMessage", length, Fmt.bytes(rec.bytes))
    }
}

/// 存到系统相册：只要「添加照片」权限（NSPhotoLibraryAddUsageDescription），不读相册
@MainActor
enum RecordingExport {
    static func saveToPhotos(_ urls: [URL], app: AppModel) {
        PHPhotoLibrary.requestAuthorization(for: .addOnly) { status in
            DispatchQueue.main.async {
                guard status == .authorized || status == .limited else {
                    app.showToast(L("remote.photosDenied"), error: true)
                    return
                }
                PHPhotoLibrary.shared().performChanges({
                    for u in urls { _ = PHAssetChangeRequest.creationRequestForAssetFromVideo(atFileURL: u) }
                }, completionHandler: { ok, error in
                    DispatchQueue.main.async {
                        if ok {
                            app.showToast(L("remote.savedToPhotos"))
                        } else {
                            app.showToast(L("remote.photosFailed", error?.localizedDescription ?? ""), error: true)
                        }
                    }
                })
            }
        }
    }
}
