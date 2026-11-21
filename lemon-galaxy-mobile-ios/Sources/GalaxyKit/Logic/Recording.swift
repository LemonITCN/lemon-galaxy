import Foundation

/// 远程桌面录像的文件名与保存位置。
public enum RecordingFiles {
    /// 文稿/Recordings：在「文件」App 的「我的 iPhone › LemonGalaxy › Recordings」里能看到
    public static var directory: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Recordings", isDirectory: true)
    }

    /// LemonGalaxy-<设备名>-<yyyyMMdd-HHmmss>.mp4
    public static func fileName(device: String, date: Date = Date(), suffix: Int = 0) -> String {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.calendar = Calendar(identifier: .gregorian)
        f.dateFormat = "yyyyMMdd-HHmmss"
        let tail = suffix > 0 ? "-\(suffix)" : ""
        return "LemonGalaxy-\(sanitize(device))-\(f.string(from: date))\(tail).mp4"
    }

    /// 新录像的保存路径（建好目录；同一秒里重名时加序号）
    public static func newURL(device: String, in dir: URL = RecordingFiles.directory, date: Date = Date()) throws -> URL {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        var n = 0
        while true {
            let url = dir.appendingPathComponent(fileName(device: device, date: date, suffix: n))
            if !FileManager.default.fileExists(atPath: url.path) { return url }
            n += 1
        }
    }

    /// 设备名里文件系统不允许或容易出问题的字符换成 -
    static func sanitize(_ name: String) -> String {
        let bad = CharacterSet(charactersIn: "/\\:*?\"<>|").union(.controlCharacters).union(.newlines)
        let scalars = name.unicodeScalars.map { bad.contains($0) ? "-" : Character($0) }
        let s = String(scalars).trimmingCharacters(in: CharacterSet.whitespaces.union(CharacterSet(charactersIn: ".")))
        return s.isEmpty ? "device" : s
    }
}

extension Fmt {
    /// 录像计时：不到 1 小时 mm:ss，否则 h:mm:ss
    public static func elapsed(_ seconds: Int) -> String {
        let s = max(0, seconds)
        let h = s / 3600, m = s / 60 % 60, sec = s % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, sec) : String(format: "%02d:%02d", m, sec)
    }
}

/// 远程开机结果的提示文字。内核返回代发设备名，空字符串表示本机。
public enum WakeText {
    public static func sent(_ via: [String], strings: Strings = L) -> String {
        if via.isEmpty || via.allSatisfy(\.isEmpty) { return strings("wake.sentLocal") }
        var names: [String] = []
        for v in via {
            let n = v.isEmpty ? strings("wake.self") : v
            if !names.contains(n) { names.append(n) }
        }
        return strings("wake.sentVia", names.joined(separator: strings.isChinese ? "、" : ", "))
    }
}
