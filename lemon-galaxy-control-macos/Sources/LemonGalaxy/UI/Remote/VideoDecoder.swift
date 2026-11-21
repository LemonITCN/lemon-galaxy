import Foundation
import VideoToolbox
import CoreMedia
import CoreVideo

/// VideoSink 收到的 Annex B 码流 → AVCC → VTDecompressionSession 硬件解码 → CVPixelBuffer（IOSurface 支持，直接给图层显示）。
/// 参数集（SPS/PPS，H.265 另有 VPS）变化时重建会话；解码失败时请求关键帧。
final class VideoDecoder {
    var onFrame: ((CVPixelBuffer, UInt64) -> Void)?
    var onNeedKeyframe: (() -> Void)?

    private let queue = DispatchQueue(label: "galaxy.video.decode", qos: .userInteractive)
    private var codec: VideoCodec = .h264
    private var session: VTDecompressionSession?
    private var format: CMVideoFormatDescription?
    private var paramSets: [Data] = []
    private var waitingKeyframe = true

    func configure(codec: VideoCodec) {
        queue.async {
            if self.codec != codec { self.reset() }
            self.codec = codec
        }
    }

    func decode(_ data: Data, ptsUs: UInt64, keyframe: Bool) {
        queue.async { self.decodeSync(data, ptsUs: ptsUs, keyframe: keyframe) }
    }

    func invalidate() {
        queue.sync { reset() }
    }

    private func reset() {
        if let s = session { VTDecompressionSessionInvalidate(s) }
        session = nil
        format = nil
        paramSets = []
        waitingKeyframe = true
    }

    private func nalType(_ nal: Data) -> Int {
        guard let b = nal.first else { return -1 }
        return codec == .h265 ? Int((b >> 1) & 0x3f) : Int(b & 0x1f)
    }

    private func isParameterSet(_ t: Int) -> Bool { codec == .h265 ? (t == 32 || t == 33 || t == 34) : (t == 7 || t == 8) }
    private func isVCL(_ t: Int) -> Bool { codec == .h265 ? (t >= 0 && t <= 31) : (t >= 1 && t <= 5) }

    private func decodeSync(_ data: Data, ptsUs: UInt64, keyframe: Bool) {
        let nals = AnnexB.split(data)
        var params: [Data] = []
        var vcl: [Data] = []
        for n in nals {
            let t = nalType(n)
            if isParameterSet(t) { params.append(n) } else if isVCL(t) || t == 6 || t == 39 || t == 40 { vcl.append(n) }
        }
        if !params.isEmpty, params != paramSets {
            paramSets = params
            makeSession()
        }
        if keyframe { waitingKeyframe = false }
        guard let session, let format, !vcl.isEmpty, !waitingKeyframe else {
            if session == nil || waitingKeyframe { onNeedKeyframe?() }
            return
        }
        // AVCC：每个 NAL 前面 4 字节大端长度
        var avcc = Data()
        for n in vcl {
            var len = UInt32(n.count).bigEndian
            withUnsafeBytes(of: &len) { avcc.append(contentsOf: $0) }
            avcc.append(n)
        }
        var block: CMBlockBuffer?
        let size = avcc.count
        guard CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: size, blockAllocator: nil,
                                                 customBlockSource: nil, offsetToData: 0, dataLength: size, flags: 0, blockBufferOut: &block) == noErr,
              let block else { return }
        avcc.withUnsafeBytes { raw in
            _ = CMBlockBufferReplaceDataBytes(with: raw.baseAddress!, blockBuffer: block, offsetIntoDestination: 0, dataLength: size)
        }
        var sample: CMSampleBuffer?
        var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: CMTime(value: CMTimeValue(ptsUs), timescale: 1_000_000),
                                        decodeTimeStamp: .invalid)
        var sampleSize = size
        guard CMSampleBufferCreateReady(allocator: nil, dataBuffer: block, formatDescription: format, sampleCount: 1,
                                        sampleTimingEntryCount: 1, sampleTimingArray: &timing, sampleSizeEntryCount: 1,
                                        sampleSizeArray: &sampleSize, sampleBufferOut: &sample) == noErr, let sample else { return }
        let status = VTDecompressionSessionDecodeFrame(session, sampleBuffer: sample, flags: [], infoFlagsOut: nil) { [weak self] status, _, image, pts, _ in
            guard let self else { return }
            if status != noErr {
                self.waitingKeyframe = true
                self.onNeedKeyframe?()
                return
            }
            if let image { self.onFrame?(image, UInt64(max(0, pts.seconds * 1_000_000))) }
        }
        if status == kVTInvalidSessionErr {
            reset()
            onNeedKeyframe?()
        }
    }

    private func makeSession() {
        if let s = session { VTDecompressionSessionInvalidate(s) }
        session = nil
        format = nil
        var desc: CMVideoFormatDescription?
        let status: OSStatus = paramSets.withPointers { ptrs, sizes in
            if codec == .h265 {
                return CMVideoFormatDescriptionCreateFromHEVCParameterSets(allocator: nil, parameterSetCount: ptrs.count, parameterSetPointers: ptrs,
                                                                          parameterSetSizes: sizes, nalUnitHeaderLength: 4, extensions: nil,
                                                                          formatDescriptionOut: &desc)
            }
            return CMVideoFormatDescriptionCreateFromH264ParameterSets(allocator: nil, parameterSetCount: ptrs.count, parameterSetPointers: ptrs,
                                                                      parameterSetSizes: sizes, nalUnitHeaderLength: 4, formatDescriptionOut: &desc)
        }
        guard status == noErr, let desc else { return }
        format = desc
        let attrs: [CFString: Any] = [
            kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
            kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
            kCVPixelBufferMetalCompatibilityKey: true,
        ]
        let spec: [CFString: Any] = [kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder: true]
        var s: VTDecompressionSession?
        if VTDecompressionSessionCreate(allocator: nil, formatDescription: desc, decoderSpecification: spec as CFDictionary,
                                        imageBufferAttributes: attrs as CFDictionary, outputCallback: nil, decompressionSessionOut: &s) == noErr {
            session = s
            if let s { VTSessionSetProperty(s, key: kVTDecompressionPropertyKey_RealTime, value: kCFBooleanTrue) }
        }
    }
}

private extension Array where Element == Data {
    /// 把一组 Data 的指针与长度一起交给 C 接口（指针只在闭包内有效）
    func withPointers<R>(_ body: ([UnsafePointer<UInt8>], [Int]) -> R) -> R {
        var ptrs: [UnsafePointer<UInt8>] = []
        var sizes: [Int] = []
        var buffers: [UnsafeMutablePointer<UInt8>] = []
        for d in self {
            let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: d.count)
            d.copyBytes(to: buf, count: d.count)
            buffers.append(buf)
            ptrs.append(UnsafePointer(buf))
            sizes.append(d.count)
        }
        defer { buffers.forEach { $0.deallocate() } }
        return body(ptrs, sizes)
    }
}
