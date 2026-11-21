import Foundation
import VideoToolbox
import CoreMedia
import CoreVideo

/// VideoToolbox 编码（模拟内核的假远程桌面用）：低延迟、无 B 帧、按需关键帧。输出 Annex B（关键帧前带参数集）。
final class VideoEncoder {
    typealias Output = (_ annexB: Data, _ ptsUs: UInt64, _ keyframe: Bool) -> Void

    let codec: VideoCodec
    private var session: VTCompressionSession?
    private var forceKeyframe = true
    private let output: Output
    private let lock = NSLock()

    init?(codec: VideoCodec, width: Int, height: Int, fps: Int, bitrateKbps: Int, output: @escaping Output) {
        self.codec = codec
        self.output = output
        var s: VTCompressionSession?
        let type: CMVideoCodecType = codec == .h265 ? kCMVideoCodecType_HEVC : kCMVideoCodecType_H264
        let status = VTCompressionSessionCreate(allocator: nil, width: Int32(width), height: Int32(height), codecType: type,
                                                encoderSpecification: nil, imageBufferAttributes: nil,
                                                compressedDataAllocator: nil, outputCallback: nil, refcon: nil,
                                                compressionSessionOut: &s)
        guard status == noErr, let s else { return nil }
        session = s
        func set(_ key: CFString, _ value: Any) { VTSessionSetProperty(s, key: key, value: value as CFTypeRef) }
        set(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue!)
        set(kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse!)
        set(kVTCompressionPropertyKey_ProfileLevel, codec == .h265 ? kVTProfileLevel_HEVC_Main_AutoLevel : kVTProfileLevel_H264_Main_AutoLevel)
        set(kVTCompressionPropertyKey_ExpectedFrameRate, fps as NSNumber)
        set(kVTCompressionPropertyKey_MaxKeyFrameInterval, (fps * 10) as NSNumber)
        set(kVTCompressionPropertyKey_AverageBitRate, (max(bitrateKbps, 1500) * 1000) as NSNumber)
        VTCompressionSessionPrepareToEncodeFrames(s)
    }

    deinit { invalidate() }

    func invalidate() {
        lock.lock(); defer { lock.unlock() }
        if let s = session {
            VTCompressionSessionCompleteFrames(s, untilPresentationTimeStamp: .invalid)
            VTCompressionSessionInvalidate(s)
        }
        session = nil
    }

    func setBitrate(kbps: Int) {
        lock.lock(); defer { lock.unlock() }
        guard let s = session else { return }
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AverageBitRate, value: (max(kbps, 200) * 1000) as NSNumber)
    }

    func requestKeyframe() {
        lock.lock(); forceKeyframe = true; lock.unlock()
    }

    func encode(_ pixelBuffer: CVPixelBuffer, ptsUs: UInt64) {
        lock.lock()
        guard let s = session else { lock.unlock(); return }
        let key = forceKeyframe
        forceKeyframe = false
        lock.unlock()
        let pts = CMTime(value: CMTimeValue(ptsUs), timescale: 1_000_000)
        let props: CFDictionary? = key ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
        let codec = self.codec
        let out = self.output
        VTCompressionSessionEncodeFrame(s, imageBuffer: pixelBuffer, presentationTimeStamp: pts, duration: .invalid,
                                        frameProperties: props, infoFlagsOut: nil) { status, _, sample in
            guard status == noErr, let sample, let (data, keyframe) = Self.annexB(from: sample, codec: codec) else { return }
            out(data, ptsUs, keyframe)
        }
    }

    /// 编码器输出（AVCC）→ Annex B；关键帧前补参数集
    static func annexB(from sample: CMSampleBuffer, codec: VideoCodec) -> (Data, Bool)? {
        guard let block = CMSampleBufferGetDataBuffer(sample), let format = CMSampleBufferGetFormatDescription(sample) else { return nil }
        var keyframe = true
        if let atts = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]],
           let first = atts.first, let notSync = first[kCMSampleAttachmentKey_NotSync] as? Bool {
            keyframe = !notSync
        }
        var out = Data()
        var nalLengthSize: Int32 = 4
        var count = 0
        if codec == .h265 {
            CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(format, parameterSetIndex: 0, parameterSetPointerOut: nil,
                                                               parameterSetSizeOut: nil, parameterSetCountOut: &count, nalUnitHeaderLengthOut: &nalLengthSize)
        } else {
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: 0, parameterSetPointerOut: nil,
                                                               parameterSetSizeOut: nil, parameterSetCountOut: &count, nalUnitHeaderLengthOut: &nalLengthSize)
        }
        if keyframe {
            for i in 0..<count {
                var p: UnsafePointer<UInt8>?
                var n = 0
                let st = codec == .h265
                    ? CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(format, parameterSetIndex: i, parameterSetPointerOut: &p,
                                                                         parameterSetSizeOut: &n, parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil)
                    : CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: i, parameterSetPointerOut: &p,
                                                                         parameterSetSizeOut: &n, parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil)
                if st == noErr, let p {
                    out.append(contentsOf: AnnexB.startCode)
                    out.append(p, count: n)
                }
            }
        }
        var total = 0
        var ptr: UnsafeMutablePointer<CChar>?
        guard CMBlockBufferGetDataPointer(block, atOffset: 0, lengthAtOffsetOut: nil, totalLengthOut: &total, dataPointerOut: &ptr) == noErr,
              let base = ptr else { return nil }
        let raw = UnsafeRawPointer(base)
        var offset = 0
        let ls = Int(nalLengthSize)
        while offset + ls <= total {
            var len = 0
            for k in 0..<ls { len = (len << 8) | Int(raw.load(fromByteOffset: offset + k, as: UInt8.self)) }
            offset += ls
            guard len > 0, offset + len <= total else { break }
            out.append(contentsOf: AnnexB.startCode)
            out.append(raw.advanced(by: offset).assumingMemoryBound(to: UInt8.self), count: len)
            offset += len
        }
        return (out, keyframe)
    }
}

/// 生成 BGRA 像素缓冲（IOSurface 支持），编码器直接吃。
enum PixelBuffers {
    static func make(width: Int, height: Int) -> CVPixelBuffer? {
        var pb: CVPixelBuffer?
        let attrs: [CFString: Any] = [
            kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
            kCVPixelBufferCGImageCompatibilityKey: true,
            kCVPixelBufferCGBitmapContextCompatibilityKey: true,
        ]
        CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_32BGRA, attrs as CFDictionary, &pb)
        return pb
    }

    /// 在像素缓冲上用 CoreGraphics 画（原点左上，y 向下）
    static func draw(_ pb: CVPixelBuffer, _ body: (CGContext) -> Void) {
        CVPixelBufferLockBaseAddress(pb, [])
        defer { CVPixelBufferUnlockBaseAddress(pb, []) }
        guard let base = CVPixelBufferGetBaseAddress(pb) else { return }
        let w = CVPixelBufferGetWidth(pb), h = CVPixelBufferGetHeight(pb)
        guard let ctx = CGContext(data: base, width: w, height: h, bitsPerComponent: 8,
                                  bytesPerRow: CVPixelBufferGetBytesPerRow(pb), space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue) else { return }
        ctx.translateBy(x: 0, y: CGFloat(h))
        ctx.scaleBy(x: 1, y: -1)
        body(ctx)
    }
}
