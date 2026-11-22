import Foundation

/// 模型回复的 Markdown 分块（标题、段落、列表、引用、代码块、表格、分隔线）。行内语法（粗体、行内代码、链接）交给界面层的 AttributedString。
/// 流式输出时代码块可能还没闭合：没闭合的也当代码块显示。
public enum MarkdownBlock: Equatable {
    case heading(level: Int, text: String)
    case paragraph(String)
    /// 列表项：缩进层级（0 起）、有序时的序号文字（如「1.」），无序为 nil
    case listItem(level: Int, marker: String?, text: String)
    case quote(String)
    case code(language: String, text: String)
    case table(header: [String], rows: [[String]])
    case rule
}

public enum MarkdownParser {
    public static func parse(_ source: String) -> [MarkdownBlock] {
        let lines = source.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n")
        var out: [MarkdownBlock] = []
        var para: [String] = []
        var i = 0

        func flushPara() {
            if !para.isEmpty {
                out.append(.paragraph(para.joined(separator: "\n")))
                para = []
            }
        }

        while i < lines.count {
            let line = lines[i]
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            // 代码块
            if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") {
                flushPara()
                let fence = String(trimmed.prefix(3))
                let lang = trimmed.dropFirst(3).trimmingCharacters(in: .whitespaces)
                var body: [String] = []
                i += 1
                while i < lines.count, !lines[i].trimmingCharacters(in: .whitespaces).hasPrefix(fence) {
                    body.append(lines[i])
                    i += 1
                }
                out.append(.code(language: lang, text: body.joined(separator: "\n")))
                i += 1
                continue
            }

            if trimmed.isEmpty {
                flushPara()
                i += 1
                continue
            }

            // 标题
            if trimmed.hasPrefix("#") {
                let hashes = trimmed.prefix { $0 == "#" }.count
                let rest = trimmed.dropFirst(hashes)
                if hashes <= 6, rest.isEmpty || rest.first == " " {
                    flushPara()
                    out.append(.heading(level: hashes, text: rest.trimmingCharacters(in: .whitespaces)))
                    i += 1
                    continue
                }
            }

            // 分隔线
            if isRule(trimmed) {
                flushPara()
                out.append(.rule)
                i += 1
                continue
            }

            // 表格：表头行 + 分隔行
            if trimmed.contains("|"), i + 1 < lines.count, isTableSeparator(lines[i + 1]) {
                flushPara()
                let header = cells(trimmed)
                var rows: [[String]] = []
                i += 2
                while i < lines.count {
                    let t = lines[i].trimmingCharacters(in: .whitespaces)
                    guard !t.isEmpty, t.contains("|") else { break }
                    var r = cells(t)
                    if r.count < header.count { r += Array(repeating: "", count: header.count - r.count) }
                    rows.append(Array(r.prefix(header.count)))
                    i += 1
                }
                out.append(.table(header: header, rows: rows))
                continue
            }

            // 引用
            if trimmed.hasPrefix(">") {
                flushPara()
                var q: [String] = []
                while i < lines.count {
                    let t = lines[i].trimmingCharacters(in: .whitespaces)
                    guard t.hasPrefix(">") else { break }
                    q.append(String(t.dropFirst()).trimmingCharacters(in: .whitespaces))
                    i += 1
                }
                out.append(.quote(q.joined(separator: "\n")))
                continue
            }

            // 列表
            if let item = listItem(line) {
                flushPara()
                out.append(item)
                i += 1
                continue
            }

            para.append(line)
            i += 1
        }
        flushPara()
        return out
    }

    static func isRule(_ t: String) -> Bool {
        let s = t.replacingOccurrences(of: " ", with: "")
        guard s.count >= 3, let c = s.first, "-*_".contains(c) else { return false }
        return s.allSatisfy { $0 == c }
    }

    static func isTableSeparator(_ line: String) -> Bool {
        let t = line.trimmingCharacters(in: .whitespaces)
        guard t.contains("-") else { return false }
        let parts = cells(t)
        return !parts.isEmpty && parts.allSatisfy { p in
            let s = p.trimmingCharacters(in: .whitespaces)
            return !s.isEmpty && s.allSatisfy { "-:".contains($0) } && s.contains("-")
        }
    }

    static func cells(_ row: String) -> [String] {
        var t = row.trimmingCharacters(in: .whitespaces)
        if t.hasPrefix("|") { t.removeFirst() }
        if t.hasSuffix("|") { t.removeLast() }
        return t.components(separatedBy: "|").map { $0.trimmingCharacters(in: .whitespaces) }
    }

    static func listItem(_ line: String) -> MarkdownBlock? {
        let indent = line.prefix { $0 == " " || $0 == "\t" }.reduce(0) { $0 + ($1 == "\t" ? 4 : 1) }
        let t = line.trimmingCharacters(in: .whitespaces)
        let level = indent / 2
        for m in ["- ", "* ", "+ "] where t.hasPrefix(m) {
            var text = String(t.dropFirst(2))
            // 任务列表
            if text.hasPrefix("[ ] ") { text = "☐ " + text.dropFirst(4) } else if text.hasPrefix("[x] ") || text.hasPrefix("[X] ") { text = "☑ " + text.dropFirst(4) }
            return .listItem(level: level, marker: nil, text: text)
        }
        let digits = t.prefix { $0.isNumber }
        if !digits.isEmpty, digits.count <= 4 {
            let rest = t.dropFirst(digits.count)
            if rest.hasPrefix(". ") || rest.hasPrefix(") ") {
                return .listItem(level: level, marker: digits + ".", text: String(rest.dropFirst(2)))
            }
        }
        return nil
    }
}
