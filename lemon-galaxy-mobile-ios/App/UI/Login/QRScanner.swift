import SwiftUI
import AVFoundation
import GalaxyKit

/// 扫管理后台「概览」页上的服务器二维码（AVFoundation，iOS 16 起都可用）。
struct QRScannerSheet: View {
    let onResult: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.loc) private var loc
    @State private var denied = false

    var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()
                if denied {
                    EmptyState(icon: "camera", title: loc("scan.denied"))
                        .environment(\.colorScheme, .dark)
                } else {
                    QRScannerView(onResult: onResult, onDenied: { denied = true })
                        .ignoresSafeArea()
                    RoundedRectangle(cornerRadius: 24).stroke(Color.white.opacity(0.85), lineWidth: 3).frame(width: 240, height: 240)
                    VStack {
                        Spacer()
                        Text(loc("login.scanPrompt")).font(.callout).foregroundStyle(.white)
                            .padding(.horizontal, 16).padding(.vertical, 10)
                            .background(.ultraThinMaterial, in: Capsule())
                            .padding(.bottom, 40)
                    }
                }
            }
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button(loc("common.cancel")) { dismiss() }.tint(.white) } }
        }
    }
}

struct QRScannerView: UIViewControllerRepresentable {
    let onResult: (String) -> Void
    let onDenied: () -> Void

    func makeUIViewController(context: Context) -> ScannerController {
        let c = ScannerController()
        c.onResult = onResult
        c.onDenied = onDenied
        return c
    }

    func updateUIViewController(_ vc: ScannerController, context: Context) {}

    final class ScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
        var onResult: ((String) -> Void)?
        var onDenied: (() -> Void)?
        private let session = AVCaptureSession()
        private var preview: AVCaptureVideoPreviewLayer?
        private var done = false

        override func viewDidLoad() {
            super.viewDidLoad()
            view.backgroundColor = .black
            switch AVCaptureDevice.authorizationStatus(for: .video) {
            case .authorized: configure()
            case .notDetermined:
                AVCaptureDevice.requestAccess(for: .video) { ok in
                    DispatchQueue.main.async { ok ? self.configure() : self.onDenied?() }
                }
            default: onDenied?()
            }
        }

        private func configure() {
            guard let device = AVCaptureDevice.default(for: .video), let input = try? AVCaptureDeviceInput(device: device),
                  session.canAddInput(input) else { onDenied?(); return }
            session.addInput(input)
            let output = AVCaptureMetadataOutput()
            guard session.canAddOutput(output) else { return }
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: .main)
            output.metadataObjectTypes = [.qr]
            let layer = AVCaptureVideoPreviewLayer(session: session)
            layer.videoGravity = .resizeAspectFill
            layer.frame = view.bounds
            view.layer.addSublayer(layer)
            preview = layer
            DispatchQueue.global(qos: .userInitiated).async { self.session.startRunning() }
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            preview?.frame = view.bounds
        }

        override func viewWillDisappear(_ animated: Bool) {
            super.viewWillDisappear(animated)
            if session.isRunning { DispatchQueue.global().async { self.session.stopRunning() } }
        }

        func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
            guard !done, let obj = objects.first as? AVMetadataMachineReadableCodeObject, let s = obj.stringValue else { return }
            done = true
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            onResult?(s)
        }
    }
}
