import Foundation
import VideoToolbox
import CoreMedia
import CoreVideo

/// VideoToolbox 硬件编码（H.264 / H.265），低延迟实时模式、无 B 帧、按需关键帧、码率可调。
/// 输出 Annex B 码流（关键帧前带参数集），直接推给内核的 EncodedFrameSink（protocol.md 8.3）。
final class VideoEncoder {
    typealias Output = (_ annexB: Data, _ ptsUs: UInt64, _ keyframe: Bool) -> Void

    let codec: VideoCodec
    let width: Int32
    let height: Int32
    private var session: VTCompressionSession?
    private var forceKeyframe = true
    private let output: Output
    private let lock = NSLock()
    /// 截屏线程与补发关键帧的线程不能同时送帧（时间戳要有序）
    private let encodeLock = NSLock()
    private let repeatQueue = DispatchQueue(label: "galaxy.encoder.repeat")
    private var lastBuffer: CVPixelBuffer?
    private var lastPts: UInt64 = 0
    private var lastAt = Date()
    private(set) var hardwareAccelerated = false
    /// 画面停下后补编（同 galaxy_host::pacing::REFRESH_AFTER）：ScreenCaptureKit 画面不变时不出帧，
    /// 变化中的最后一帧往往不够清楚；停下后把它再编几次（P 帧补上量化误差，内容没变时几乎不占码率），几百毫秒内变清楚。
    /// 本机 VideoToolbox 实测（1080p 文字画面、12 Mbps）：补编 4 次后最后一帧 PSNR 29.9 → 32.5 dB。
    private static let refreshAfter: [Double] = [0.08, 0.2, 0.45, 0.9]
    /// 每来一帧新画面加一，补编时比对：期间又有新画面就不补旧的
    private var generation: UInt64 = 0
    private var refreshStep = 0
    private var refreshItem: DispatchWorkItem?
    /// 最后一帧新画面的时刻与时间戳（补编的节奏与时间戳都从这里算，时间戳要跟真实时间一致，码率按时间分）
    private var changeAt = Date()
    private var changePts: UInt64 = 0
    /// 这一阵变化里最大的一帧（字节）。小于 refreshMinBytes（光标闪烁、敲一个字）不补编：小块区域本来就编得清楚
    /// 由 sizeLock 保护（编码输出回调里只能拿这把：invalidate 拿着 lock 等 VideoToolbox 吐完剩下的帧，回调再拿 lock 会死锁）
    private var burstMax = 0
    private let sizeLock = NSLock()
    private static let refreshMinBytes = 4096

    init?(codec: VideoCodec, width: Int, height: Int, fps: Int, bitrateKbps: Int, output: @escaping Output) {
        self.codec = codec
        self.width = Int32(width)
        self.height = Int32(height)
        self.output = output
        var spec: [CFString: Any] = [
            kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: true,
        ]
        // 低延迟码率控制：H.264 自 macOS 11.3、H.265 自 macOS 13 支持；打开后编码器不再攒帧
        spec[kVTVideoEncoderSpecification_EnableLowLatencyRateControl] = true
        var s: VTCompressionSession?
        let type: CMVideoCodecType = codec == .h265 ? kCMVideoCodecType_HEVC : kCMVideoCodecType_H264
        let status = VTCompressionSessionCreate(allocator: nil, width: Int32(width), height: Int32(height), codecType: type,
                                                encoderSpecification: spec as CFDictionary, imageBufferAttributes: nil,
                                                compressedDataAllocator: nil, outputCallback: nil, refcon: nil,
                                                compressionSessionOut: &s)
        guard status == noErr, let s else { return nil }
        session = s
        func set(_ key: CFString, _ value: Any) { VTSessionSetProperty(s, key: key, value: value as CFTypeRef) }
        set(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue!)
        set(kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse!)
        set(kVTCompressionPropertyKey_ProfileLevel, codec == .h265 ? kVTProfileLevel_HEVC_Main_AutoLevel : kVTProfileLevel_H264_High_AutoLevel)
        set(kVTCompressionPropertyKey_ExpectedFrameRate, fps as NSNumber)
        // 关键帧只在需要时（丢包、新观众）发，平时间隔放大到 10 秒，省带宽
        set(kVTCompressionPropertyKey_MaxKeyFrameInterval, (fps * 10) as NSNumber)
        set(kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, 10 as NSNumber)
        applyBitrate(bitrateKbps)
        var hw: CFTypeRef?
        let status2 = withUnsafeMutablePointer(to: &hw) {
            VTSessionCopyProperty(s, key: kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder, allocator: nil, valueOut: UnsafeMutableRawPointer($0))
        }
        if status2 == noErr { hardwareAccelerated = (hw as? Bool) ?? false }
        VTCompressionSessionPrepareToEncodeFrames(s)
    }

    deinit { invalidate() }

    func invalidate() {
        lock.lock(); defer { lock.unlock() }
        refreshItem?.cancel()
        refreshItem = nil
        if let s = session {
            VTCompressionSessionCompleteFrames(s, untilPresentationTimeStamp: .invalid)
            VTCompressionSessionInvalidate(s)
        }
        session = nil
        lastBuffer = nil
    }

    func setBitrate(kbps: Int) {
        lock.lock(); defer { lock.unlock() }
        applyBitrate(kbps)
    }

    private func applyBitrate(_ kbps: Int) {
        guard let s = session else { return }
        let bps = max(kbps, 200) * 1000
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AverageBitRate, value: bps as NSNumber)
        // 1 秒窗口内最多 1.5 倍平均码率，防止画面突变时把链路打满
        let limits = [Double(bps) * 1.5 / 8, 1.0] as CFArray
        VTSessionSetProperty(s, key: kVTCompressionPropertyKey_DataRateLimits, value: limits)
    }

    /// 要关键帧。画面静止时截屏不出新帧，等下一帧可能要很久（主控端解锁回来一直黑着），
    /// 所以稍等一下还没有新帧就拿最后一帧重新编一个关键帧
    func requestKeyframe() {
        lock.lock(); forceKeyframe = true; lock.unlock()
        repeatQueue.asyncAfter(deadline: .now() + .milliseconds(60)) { [weak self] in
            guard let self else { return }
            self.lock.lock()
            let pending = self.forceKeyframe
            let buf = self.lastBuffer
            let pts = self.lastPts + UInt64(max(0, Date().timeIntervalSince(self.lastAt)) * 1_000_000)
            self.lock.unlock()
            if pending, let buf { self.encode(buf, ptsUs: pts) }
        }
    }

    /// 编一帧新画面（截屏线程、补发关键帧时调用），之后按 refreshAfter 补编。
    func encode(_ pixelBuffer: CVPixelBuffer, ptsUs requested: UInt64) {
        lock.lock()
        // 上一阵补编完了或停了一秒以上：新的一阵
        if Date().timeIntervalSince(changeAt) >= 1.0 || refreshStep >= Self.refreshAfter.count { sizeLock.locked { burstMax = 0 } }
        lock.unlock()
        encodeFrame(pixelBuffer, ptsUs: requested, change: true)
        lock.lock()
        generation &+= 1
        refreshStep = 0
        changeAt = lastAt
        changePts = lastPts
        scheduleRefresh(generation: generation)
        lock.unlock()
    }

    /// 安排下一次补编（调用时持有 lock）
    private func scheduleRefresh(generation gen: UInt64) {
        refreshItem?.cancel()
        guard session != nil, refreshStep < Self.refreshAfter.count else { refreshItem = nil; return }
        let due = changeAt.addingTimeInterval(Self.refreshAfter[refreshStep])
        let item = DispatchWorkItem { [weak self] in self?.refresh(generation: gen) }
        refreshItem = item
        repeatQueue.asyncAfter(deadline: .now() + max(0, due.timeIntervalSinceNow), execute: item)
    }

    private func refresh(generation gen: UInt64) {
        lock.lock()
        guard gen == generation, let buf = lastBuffer, session != nil else { lock.unlock(); return }
        let changedAt = changeAt
        let pts = changePts + UInt64(max(0, Date().timeIntervalSince(changedAt)) * 1_000_000)
        let worth = sizeLock.locked { burstMax } >= Self.refreshMinBytes
        lock.unlock()
        if worth { encodeFrame(buf, ptsUs: pts, change: false) }
        lock.lock()
        if gen == generation {
            // 跳过已经过时的补编点（线程被耽搁时不连着补好几次）
            let since = Date().timeIntervalSince(changedAt)
            refreshStep += 1
            while refreshStep < Self.refreshAfter.count && since >= Self.refreshAfter[refreshStep] { refreshStep += 1 }
            scheduleRefresh(generation: gen)
        }
        lock.unlock()
    }

    /// `change`：新画面（记下编出来的大小，决定停下后要不要补编）
    private func encodeFrame(_ pixelBuffer: CVPixelBuffer, ptsUs requested: UInt64, change: Bool) {
        encodeLock.lock()
        defer { encodeLock.unlock() }
        lock.lock()
        guard let s = session else { lock.unlock(); return }
        let key = forceKeyframe
        forceKeyframe = false
        // 时间戳必须递增（补发的关键帧可能比截屏给的时间稍晚）
        let ptsUs = max(requested, lastPts + (lastBuffer == nil ? 0 : 1))
        lastBuffer = pixelBuffer
        lastPts = ptsUs
        lastAt = Date()
        lock.unlock()
        let pts = CMTime(value: CMTimeValue(ptsUs), timescale: 1_000_000)
        let props: CFDictionary? = key ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
        let codec = self.codec
        let out = self.output
        VTCompressionSessionEncodeFrame(s, imageBuffer: pixelBuffer, presentationTimeStamp: pts, duration: .invalid,
                                        frameProperties: props, infoFlagsOut: nil) { [weak self] status, _, sample in
            guard status == noErr, let sample, let (data, keyframe) = AnnexB.fromSampleBuffer(sample, codec: codec) else { return }
            if change, let self { self.sizeLock.locked { self.burstMax = max(self.burstMax, data.count) } }
            out(data, ptsUs, keyframe)
        }
    }
}

/// Annex B 与 AVCC（长度前缀）之间的转换。
enum AnnexB {
    static let startCode: [UInt8] = [0, 0, 0, 1]

    /// 编码器输出（AVCC）→ Annex B；关键帧前补参数集（H.264：SPS/PPS；H.265：VPS/SPS/PPS）。
    static func fromSampleBuffer(_ sample: CMSampleBuffer, codec: VideoCodec) -> (Data, Bool)? {
        guard let block = CMSampleBufferGetDataBuffer(sample), let format = CMSampleBufferGetFormatDescription(sample) else { return nil }
        var keyframe = true
        if let atts = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]],
           let first = atts.first, let notSync = first[kCMSampleAttachmentKey_NotSync] as? Bool {
            keyframe = !notSync
        }
        var out = Data()
        var nalLengthSize: Int32 = 4
        if keyframe {
            var count = 0
            if codec == .h265 {
                CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(format, parameterSetIndex: 0, parameterSetPointerOut: nil,
                                                                   parameterSetSizeOut: nil, parameterSetCountOut: &count, nalUnitHeaderLengthOut: &nalLengthSize)
                for i in 0..<count {
                    var p: UnsafePointer<UInt8>?; var n = 0
                    if CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(format, parameterSetIndex: i, parameterSetPointerOut: &p,
                                                                          parameterSetSizeOut: &n, parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil) == noErr, let p {
                        out.append(contentsOf: startCode); out.append(p, count: n)
                    }
                }
            } else {
                CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: 0, parameterSetPointerOut: nil,
                                                                   parameterSetSizeOut: nil, parameterSetCountOut: &count, nalUnitHeaderLengthOut: &nalLengthSize)
                for i in 0..<count {
                    var p: UnsafePointer<UInt8>?; var n = 0
                    if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: i, parameterSetPointerOut: &p,
                                                                          parameterSetSizeOut: &n, parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil) == noErr, let p {
                        out.append(contentsOf: startCode); out.append(p, count: n)
                    }
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
            out.append(contentsOf: startCode)
            out.append(raw.advanced(by: offset).assumingMemoryBound(to: UInt8.self), count: len)
            offset += len
        }
        return (out, keyframe)
    }

    /// Annex B → NAL 单元列表（不含起始码）。
    static func split(_ data: Data) -> [Data] {
        var units: [Data] = []
        data.withUnsafeBytes { (buf: UnsafeRawBufferPointer) in
            let b = buf.bindMemory(to: UInt8.self)
            let n = b.count
            var i = 0
            var start = -1
            while i + 2 < n {
                if b[i] == 0, b[i + 1] == 0, b[i + 2] == 1 {
                    if start >= 0 {
                        var end = i
                        if end > start, b[end - 1] == 0 { end -= 1 }
                        if end > start { units.append(Data(b[start..<end])) }
                    }
                    i += 3
                    start = i
                } else {
                    i += 1
                }
            }
            if start >= 0, start < n { units.append(Data(b[start..<n])) }
        }
        return units
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

    /// 在像素缓冲上用 CoreGraphics 画（原点左上，y 向下）。
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
