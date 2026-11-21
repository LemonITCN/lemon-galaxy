import Foundation
import AVFoundation
import CoreVideo

/// 模拟内核的录像：把假桌面的每一帧用 AVAssetWriter 编成真正能播放的 MP4（真内核是把收到的 H.264 原样封装）。
/// append 在模拟画面的队列上调用，finish 在调用方线程（此时已不会再有 append）。
final class MockRecorder {
    let url: URL
    private let writer: AVAssetWriter
    private let input: AVAssetWriterInput
    private let adaptor: AVAssetWriterInputPixelBufferAdaptor
    private var firstTime: CFTimeInterval?
    private var lastTime: CFTimeInterval = -1

    init(url: URL, width: Int, height: Int) throws {
        self.url = url
        try? FileManager.default.removeItem(at: url)
        writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
        let settings: [String: Any] = [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height]
        input = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
        input.expectsMediaDataInRealTime = true
        adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: nil)
        guard writer.canAdd(input) else { throw GalaxyError.Other(detail: "无法创建录像文件") }
        writer.add(input)
        guard writer.startWriting() else {
            throw GalaxyError.Other(detail: writer.error.map { "无法创建录像文件：\($0.localizedDescription)" } ?? "无法创建录像文件")
        }
    }

    func append(_ pb: CVPixelBuffer, at time: CFTimeInterval) {
        if firstTime == nil {
            firstTime = time
            writer.startSession(atSourceTime: .zero)
        }
        guard let first = firstTime, input.isReadyForMoreMediaData, time > lastTime else { return }
        if adaptor.append(pb, withPresentationTime: CMTime(seconds: time - first, preferredTimescale: 600)) {
            lastTime = time
        }
    }

    /// 结束并返回写出的文件；一帧都没录到时不留文件
    func finish() -> RecordingInfo {
        guard let first = firstTime, lastTime >= first else {
            writer.cancelWriting()
            try? FileManager.default.removeItem(at: url)
            return RecordingInfo(paths: [], durationMs: 0, bytes: 0)
        }
        input.markAsFinished()
        let done = DispatchSemaphore(value: 0)
        writer.finishWriting { done.signal() }
        done.wait()
        let size = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.uint64Value ?? 0
        return RecordingInfo(paths: [url.path], durationMs: UInt64(max(0, lastTime - first) * 1000), bytes: size)
    }
}
