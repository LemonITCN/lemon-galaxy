import SwiftUI
import AppKit

/// 输入框里交给 @ 候选处理的按键
enum ComposerKey { case up, down, choose, cancel }

/// AI 输入框的多行文本框：Enter 发送、Shift+Enter 换行，最多长到约 8 行后内部滚动；
/// 输入 @ 时把过滤文字和光标位置报给外面弹候选。输入法组字期间回车交给输入法。
struct ComposerTextView: NSViewRepresentable {
    @Binding var text: String
    @Binding var height: CGFloat
    var enabled: Bool
    var focusToken: Int
    var caretRequest: Int?
    var onCaretApplied: () -> Void
    var onSubmit: () -> Void
    var onMention: (MentionState?) -> Void
    var onMentionKey: (ComposerKey) -> Bool

    static let font = NSFont.systemFont(ofSize: 13)
    static let maxLines: CGFloat = 8

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.borderType = .noBorder
        let tv = NSTextView()
        tv.delegate = context.coordinator
        tv.isRichText = false
        tv.importsGraphics = false
        tv.allowsUndo = true
        tv.font = Self.font
        tv.drawsBackground = false
        tv.isAutomaticQuoteSubstitutionEnabled = false
        tv.isAutomaticDashSubstitutionEnabled = false
        tv.isAutomaticTextReplacementEnabled = false
        tv.textContainerInset = NSSize(width: 0, height: 1)
        tv.textContainer?.lineFragmentPadding = 5
        tv.isVerticallyResizable = true
        tv.isHorizontallyResizable = false
        tv.autoresizingMask = [.width]
        tv.textContainer?.widthTracksTextView = true
        tv.string = text
        scroll.documentView = tv
        context.coordinator.textView = tv
        context.coordinator.lastFocusToken = focusToken
        DispatchQueue.main.async {
            context.coordinator.recalcHeight()
            tv.window?.makeFirstResponder(tv)
        }
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let c = context.coordinator
        c.parent = self
        guard let tv = c.textView else { return }
        tv.isEditable = enabled
        tv.textColor = NSColor.labelColor
        if tv.string != text {
            tv.string = text
            c.recalcHeight()
        }
        if let caret = caretRequest {
            let n = (tv.string as NSString).length
            tv.setSelectedRange(NSRange(location: min(caret, n), length: 0))
            DispatchQueue.main.async { onCaretApplied() }
        }
        if c.lastFocusToken != focusToken {
            c.lastFocusToken = focusToken
            DispatchQueue.main.async { tv.window?.makeFirstResponder(tv) }
        }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: ComposerTextView
        weak var textView: NSTextView?
        var lastFocusToken = 0
        private var lastMention: MentionState?

        init(_ parent: ComposerTextView) { self.parent = parent }

        func textDidChange(_ notification: Notification) {
            guard let tv = textView else { return }
            if parent.text != tv.string { parent.text = tv.string }
            recalcHeight()
            updateMention()
        }

        func textViewDidChangeSelection(_ notification: Notification) {
            updateMention()
        }

        func textView(_ textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            // 输入法正在组字：回车、方向键交给输入法
            if textView.hasMarkedText() { return false }
            if lastMention != nil {
                switch selector {
                case #selector(NSResponder.moveUp(_:)): return parent.onMentionKey(.up)
                case #selector(NSResponder.moveDown(_:)): return parent.onMentionKey(.down)
                case #selector(NSResponder.insertNewline(_:)), #selector(NSResponder.insertTab(_:)): return parent.onMentionKey(.choose)
                case #selector(NSResponder.cancelOperation(_:)):
                    lastMention = nil
                    return parent.onMentionKey(.cancel)
                default: break
                }
            }
            if selector == #selector(NSResponder.insertNewline(_:)) {
                if NSApp.currentEvent?.modifierFlags.contains(.shift) == true {
                    textView.insertNewlineIgnoringFieldEditor(nil)
                } else {
                    parent.onSubmit()
                }
                return true
            }
            return false
        }

        func recalcHeight() {
            guard let tv = textView, let lm = tv.layoutManager, let tc = tv.textContainer else { return }
            lm.ensureLayout(for: tc)
            let line = lm.defaultLineHeight(for: ComposerTextView.font)
            let used = lm.usedRect(for: tc).height
            let content = max(line, used) + tv.textContainerInset.height * 2
            let h = min(content, line * ComposerTextView.maxLines + tv.textContainerInset.height * 2).rounded(.up)
            if abs(h - parent.height) > 0.5 {
                DispatchQueue.main.async { self.parent.height = h }
            }
        }

        /// 光标前是「@过滤文字」（@ 在行首或空白后，中间没有空白）时报给外面
        private func updateMention() {
            guard let tv = textView else { return }
            let sel = tv.selectedRange()
            var found: MentionState?
            if sel.length == 0, !tv.hasMarkedText() {
                let ns = tv.string as NSString
                var i = sel.location - 1
                let ws = CharacterSet.whitespacesAndNewlines
                while i >= 0, sel.location - i <= 40 {
                    let ch = ns.character(at: i)
                    guard let scalar = UnicodeScalar(ch) else { break }
                    if ws.contains(scalar) { break }
                    if ch == 0x40 /* @ */ {
                        let before: unichar? = i > 0 ? ns.character(at: i - 1) : nil
                        if before == nil || UnicodeScalar(before!).map({ ws.contains($0) }) == true {
                            let range = NSRange(location: i, length: sel.location - i)
                            let query = ns.substring(with: NSRange(location: i + 1, length: sel.location - i - 1))
                            found = MentionState(range: range, query: query, caret: caretPoint(i))
                        }
                        break
                    }
                    i -= 1
                }
            }
            if found?.range != lastMention?.range || found?.query != lastMention?.query {
                lastMention = found
                let f = found
                DispatchQueue.main.async { self.parent.onMention(f) }
            }
        }

        /// 第 index 个字符左上角在整个输入框（滚动视图）里的坐标
        private func caretPoint(_ index: Int) -> CGPoint {
            guard let tv = textView, let lm = tv.layoutManager, let tc = tv.textContainer, let scroll = tv.enclosingScrollView else { return .zero }
            let glyphs = lm.glyphRange(forCharacterRange: NSRange(location: index, length: 1), actualCharacterRange: nil)
            var rect = lm.boundingRect(forGlyphRange: glyphs, in: tc)
            rect.origin.x += tv.textContainerOrigin.x
            rect.origin.y += tv.textContainerOrigin.y
            let p = tv.convert(rect.origin, to: scroll)
            // NSScrollView 不是翻转坐标：换成自上而下
            let y = scroll.isFlipped ? p.y : scroll.bounds.height - p.y
            return CGPoint(x: max(0, p.x), y: max(0, y))
        }
    }
}
