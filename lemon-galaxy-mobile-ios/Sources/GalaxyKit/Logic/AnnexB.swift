import Foundation

/// Annex B 码流（起始码 00 00 01 / 00 00 00 01 分隔的 NAL 单元）的拆分与识别。
public enum AnnexB {
    public static let startCode: [UInt8] = [0, 0, 0, 1]

    /// Annex B → NAL 单元列表（不含起始码）
    public static func split(_ data: Data) -> [Data] {
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

    public static func nalType(_ nal: Data, codec: VideoCodec) -> Int {
        guard let b = nal.first else { return -1 }
        return codec == .h265 ? Int((b >> 1) & 0x3f) : Int(b & 0x1f)
    }

    /// 参数集：H.264 SPS/PPS；H.265 VPS/SPS/PPS
    public static func isParameterSet(_ t: Int, codec: VideoCodec) -> Bool {
        codec == .h265 ? (t == 32 || t == 33 || t == 34) : (t == 7 || t == 8)
    }

    /// 视频编码层（需要送进解码器的图像数据）
    public static func isVCL(_ t: Int, codec: VideoCodec) -> Bool {
        codec == .h265 ? (0...31).contains(t) : (1...5).contains(t)
    }

    /// 是不是关键帧（IDR / IRAP）
    public static func isKeyframe(_ t: Int, codec: VideoCodec) -> Bool {
        codec == .h265 ? (16...21).contains(t) : t == 5
    }

    /// NAL 单元 → AVCC（每个前面 4 字节大端长度）
    public static func avcc(_ nals: [Data]) -> Data {
        var out = Data()
        for n in nals {
            var len = UInt32(n.count).bigEndian
            withUnsafeBytes(of: &len) { out.append(contentsOf: $0) }
            out.append(n)
        }
        return out
    }
}
