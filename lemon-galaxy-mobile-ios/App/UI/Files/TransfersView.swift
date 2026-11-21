import SwiftUI
import GalaxyKit

struct TransfersView: View {
    @EnvironmentObject var transfers: TransferCenter
    @Environment(\.dismiss) private var dismiss
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc
    @State private var share: URL?

    var body: some View {
        NavigationStack {
            List {
                if transfers.items.isEmpty {
                    EmptyState(icon: "arrow.up.arrow.down", title: loc("transfers.empty")).listRowBackground(Color.clear)
                }
                ForEach(transfers.items) { item in
                    TransferRow(item: item, share: { share = $0 })
                        .galaxyRow()
                }
            }
            .galaxyList()
            .navigationTitle(loc("files.transfers"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(loc("common.close")) { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    Button(loc("transfer.clearFinished")) { transfers.clearFinished() }
                        .disabled(!transfers.items.contains { !$0.isActive })
                }
                ToolbarItem(placement: .bottomBar) { InfoButton(text: loc("transfers.info")) }
            }
            .sheet(item: Binding(get: { share.map { ShareItem(url: $0) } }, set: { if $0 == nil { share = nil } })) { s in
                ShareSheet(items: [s.url])
            }
        }
    }
}

struct TransferRow: View {
    @ObservedObject var item: TransferItem
    let share: (URL) -> Void
    @EnvironmentObject var transfers: TransferCenter
    @Environment(\.palette) private var p
    @Environment(\.loc) private var loc

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: item.direction == .upload ? "arrow.up.circle" : "arrow.down.circle")
                .font(.system(size: 22))
                .foregroundStyle(color)
            VStack(alignment: .leading, spacing: 4) {
                Text(item.name).lineLimit(1).foregroundStyle(p.text)
                if item.isActive {
                    ProgressView(value: item.fraction).tint(p.accent)
                }
                Text(detail).font(.caption.monospacedDigit()).foregroundStyle(p.secondaryText).lineLimit(2)
            }
            Spacer(minLength: 6)
            controls
        }
        .padding(.vertical, 4)
    }

    private var color: Color {
        switch item.status {
        case .done: return p.success
        case .failed: return p.danger
        case .cancelled: return p.offline
        default: return p.accent
        }
    }

    private var detail: String {
        let peer = item.direction == .upload ? loc("transfer.to", item.deviceName) : loc("transfer.from", item.deviceName)
        switch item.status {
        case .running:
            return "\(Fmt.bytes(item.done)) / \(Fmt.bytes(item.total)) · \(Fmt.rate(item.bytesPerSecond)) · \(peer)"
        case .paused: return loc("transfer.paused") + " · " + "\(Fmt.bytes(item.done)) / \(Fmt.bytes(item.total))"
        case .done: return loc("transfer.done") + " · " + Fmt.bytes(item.total) + " · " + peer
        case .failed(let e): return loc("transfer.failed", e)
        case .cancelled: return loc("transfer.cancelled")
        }
    }

    @ViewBuilder private var controls: some View {
        HStack(spacing: 14) {
            switch item.status {
            case .running:
                Button { item.pause() } label: { Image(systemName: "pause.fill") }.accessibilityLabel(loc("transfer.pause"))
                Button { item.cancel() } label: { Image(systemName: "xmark") }.accessibilityLabel(loc("transfer.cancel"))
            case .paused:
                Button { item.resume() } label: { Image(systemName: "play.fill") }.accessibilityLabel(loc("transfer.resume"))
                Button { item.cancel() } label: { Image(systemName: "xmark") }.accessibilityLabel(loc("transfer.cancel"))
            case .failed, .cancelled:
                Button { transfers.retry(item) } label: { Image(systemName: "arrow.clockwise") }.accessibilityLabel(loc("transfer.retry"))
            case .done:
                if item.direction == .download {
                    Button { share(item.localURL) } label: { Image(systemName: "square.and.arrow.up") }.accessibilityLabel(loc("common.share"))
                }
            }
        }
        .buttonStyle(.borderless)
        .foregroundStyle(p.accent)
    }
}
