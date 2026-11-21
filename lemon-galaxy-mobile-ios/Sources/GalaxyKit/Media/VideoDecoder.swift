import Foundation
import VideoToolbox
import CoreMedia
import CoreVideo

/// VideoSink 收到的 Annex B 码流 → AVCC → VTDecompressionSession 硬件解码 → CVPixelBuffer（IOSurface 支持，直接交给显示层）。
/// 参数集（SPS/PPS，H.265 另有 VPS）变化时重建会话；解码失败或还没等到关键帧时请求关键帧。
public final class VideoDecoder: @unchecked Sendable {
    /// 解码出的一帧（在解码线程上回调）
    public var onFrame: ((CVPixelBuffer, UInt64) -> Void)?
    /// 需要关键帧（在解码线程上回调；调用方自己限频）
    public var onNeedKeyframe: (() -> Void)?

    private let queue = DispatchQueue(label: "galaxy.video.decode", qos: .userInteractive)
    private var codec: VideoCodec = .h264
    private var session: VTDecompressionSession?
    private var format: CMVideoFormatDescription?
    private var paramSets: [Data] = []
    private var waitingKeyframe = true
    private var lastKeyframeRequest = Date.distantPast

    public init() {}

    /// 设备能硬件解码的编码（H.265 看硬件）
    public static var supportedCodecs: [VideoCodec] {
        VTIsHardwareDecodeSupported(kCMVideoCodecType_HEVC) ? [.h265, .h264] : [.h264]
    }

    public func configure(codec: VideoCodec) {
        queue.async {
            if self.codec != codec { self.reset() }
            self.codec = codec
        }
    }

    public func decode(_ data: Data, ptsUs: UInt64, keyframe: Bool) {
        queue.async { self.decodeSync(data, ptsUs: ptsUs, keyframe: keyframe) }
    }

    public func invalidate() {
        queue.sync { reset() }
    }

    private func reset() {
        if let s = session { VTDecompressionSessionInvalidate(s) }
        session = nil
        format = nil
        paramSets = []
        waitingKeyframe = true
    }

    private func needKeyframe() {
        // 1 秒内最多请求一次，避免丢包时把被控端压垮
        let now = Date()
        guard now.timeIntervalSince(lastKeyframeRequest) > 1 else { return }
        lastKeyframeRequest = now
        onNeedKeyframe?()
    }

    private func decodeSync(_ data: Data, ptsUs: UInt64, keyframe: Bool) {
        let nals = AnnexB.split(data)
        var params: [Data] = []
        var vcl: [Data] = []
        for n in nals {
            let t = AnnexB.nalType(n, codec: codec)
            if AnnexB.isParameterSet(t, codec: codec) {
                params.append(n)
            } else if AnnexB.isVCL(t, codec: codec) || (codec == .h264 && t == 6) || (codec == .h265 && (t == 39 || t == 40)) {
                vcl.append(n)
            }
        }
        if !params.isEmpty, params != paramSets {
            paramSets = params
            makeSession()
        }
        if keyframe { waitingKeyframe = false }
        guard let session, let format, !vcl.isEmpty, !waitingKeyframe else {
            if session == nil || waitingKeyframe { needKeyframe() }
            return
        }
        let avcc = AnnexB.avcc(vcl)
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
                self.queue.async {
                    self.waitingKeyframe = true
                    self.needKeyframe()
                }
                return
            }
            if let image { self.onFrame?(image, UInt64(max(0, pts.seconds * 1_000_000))) }
        }
        if status == kVTInvalidSessionErr {
            // 切后台后系统会让解码会话失效：重建并等下一个关键帧
            reset()
            needKeyframe()
        } else if status != noErr {
            waitingKeyframe = true
            needKeyframe()
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
        var s: VTDecompressionSession?
        if VTDecompressionSessionCreate(allocator: nil, formatDescription: desc, decoderSpecification: nil,
                                        imageBufferAttributes: attrs as CFDictionary, outputCallback: nil, decompressionSessionOut: &s) == noErr {
            session = s
            if let s { VTSessionSetProperty(s, key: kVTDecompressionPropertyKey_RealTime, value: kCFBooleanTrue) }
        }
    }
}

extension Array where Element == Data {
    /// 把一组 Data 的指针与长度一起交给 C 接口（指针只在闭包内有效）
    func withPointers<R>(_ body: ([UnsafePointer<UInt8>], [Int]) -> R) -> R {
        var ptrs: [UnsafePointer<UInt8>] = []
        var sizes: [Int] = []
        var buffers: [UnsafeMutablePointer<UInt8>] = []
        for d in self {
            let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: Swift.max(d.count, 1))
            d.copyBytes(to: buf, count: d.count)
            buffers.append(buf)
            ptrs.append(UnsafePointer(buf))
            sizes.append(d.count)
        }
        defer { buffers.forEach { $0.deallocate() } }
        return body(ptrs, sizes)
    }
}
