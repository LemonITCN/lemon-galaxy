using System.Diagnostics;
using System.Text;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;

namespace LemonGalaxy.UI;

/// <summary>
/// 模型回复的 Markdown 渲染（常用子集）：标题、段落、粗体 / 斜体、行内代码、代码块（语言名 + 复制）、有序 / 无序列表（可嵌套）、
/// 引用、表格、分隔线、链接。整条回复画在一个只读 RichTextBox 里，可以跨段落选中复制。流式输出时末尾显示闪烁的光标。
/// </summary>
internal static class Md
{
    // ---------- 块级解析 ----------

    internal abstract class Block { }
    internal sealed class Heading : Block { public int Level; public string Text = ""; }
    internal sealed class Para : Block { public readonly List<string> Lines = new(); }
    internal sealed class Code : Block { public string Lang = ""; public string Text = ""; }
    internal sealed class Quote : Block { public readonly List<string> Lines = new(); }
    internal sealed class Rule : Block { }
    internal sealed class ListBlock : Block { public bool Ordered; public int Start = 1; public readonly List<Item> Items = new(); }
    internal sealed class Item { public readonly List<string> Lines = new(); public ListBlock? Child; }
    internal sealed class Table : Block { public string[] Header = Array.Empty<string>(); public TextAlignment[] Align = Array.Empty<TextAlignment>(); public readonly List<string[]> Rows = new(); }

    private static readonly Regex ListRe = new(@"^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$", RegexOptions.Compiled);
    private static readonly Regex HeadingRe = new(@"^(#{1,6})\s+(.*?)(?:\s+#+)?\s*$", RegexOptions.Compiled);
    private static readonly Regex RuleRe = new(@"^\s{0,3}([-*_])(\s*\1){2,}\s*$", RegexOptions.Compiled);
    private static readonly Regex TableSepRe = new(@"^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$", RegexOptions.Compiled);
    private static readonly Regex FenceRe = new(@"^\s{0,3}(`{3,}|~{3,})\s*([^`\s]*)", RegexOptions.Compiled);

    public static List<Block> Parse(string text)
    {
        var lines = text.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n');
        var blocks = new List<Block>();
        var i = 0;
        while (i < lines.Length)
        {
            var line = lines[i];
            if (line.Trim().Length == 0) { i++; continue; }
            var fence = FenceRe.Match(line);
            if (fence.Success)
            {
                var marker = fence.Groups[1].Value;
                var code = new Code { Lang = fence.Groups[2].Value };
                var sb = new List<string>();
                i++;
                // 流式输出时代码块可能还没结束：剩下的都算代码
                while (i < lines.Length && !lines[i].TrimStart().StartsWith(marker, StringComparison.Ordinal)) sb.Add(lines[i++]);
                i++;
                code.Text = string.Join("\n", sb);
                blocks.Add(code);
                continue;
            }
            var h = HeadingRe.Match(line);
            if (h.Success)
            {
                blocks.Add(new Heading { Level = h.Groups[1].Value.Length, Text = h.Groups[2].Value });
                i++;
                continue;
            }
            if (RuleRe.IsMatch(line))
            {
                blocks.Add(new Rule());
                i++;
                continue;
            }
            if (line.TrimStart().StartsWith(">", StringComparison.Ordinal))
            {
                var q = new Quote();
                while (i < lines.Length && lines[i].TrimStart().StartsWith(">", StringComparison.Ordinal))
                {
                    var t = lines[i].TrimStart().Substring(1);
                    q.Lines.Add(t.StartsWith(" ", StringComparison.Ordinal) ? t.Substring(1) : t);
                    i++;
                }
                blocks.Add(q);
                continue;
            }
            if (line.Contains('|') && i + 1 < lines.Length && TableSepRe.IsMatch(lines[i + 1]) && lines[i + 1].Contains('-'))
            {
                var t = new Table { Header = Cells(line) };
                t.Align = Cells(lines[i + 1]).Select(c =>
                {
                    var l = c.StartsWith(":", StringComparison.Ordinal);
                    var r = c.EndsWith(":", StringComparison.Ordinal);
                    return l && r ? TextAlignment.Center : r ? TextAlignment.Right : TextAlignment.Left;
                }).ToArray();
                i += 2;
                while (i < lines.Length && lines[i].Contains('|') && lines[i].Trim().Length > 0) t.Rows.Add(Cells(lines[i++]));
                blocks.Add(t);
                continue;
            }
            if (ListRe.IsMatch(line))
            {
                blocks.Add(ParseList(lines, ref i, Indent(line)));
                continue;
            }
            var p = new Para();
            while (i < lines.Length)
            {
                var l = lines[i];
                if (l.Trim().Length == 0 || FenceRe.IsMatch(l) || HeadingRe.IsMatch(l) || l.TrimStart().StartsWith(">", StringComparison.Ordinal)) break;
                if (p.Lines.Count > 0 && (ListRe.IsMatch(l) || RuleRe.IsMatch(l))) break;
                if (l.Contains('|') && i + 1 < lines.Length && TableSepRe.IsMatch(lines[i + 1]) && lines[i + 1].Contains('-')) break;
                p.Lines.Add(l.Trim());
                i++;
            }
            if (p.Lines.Count == 0) { p.Lines.Add(line.Trim()); i++; }
            blocks.Add(p);
        }
        return blocks;
    }

    private static int Indent(string line)
    {
        var n = 0;
        foreach (var c in line)
        {
            if (c == ' ') n++;
            else if (c == '\t') n += 4;
            else break;
        }
        return n;
    }

    private static ListBlock ParseList(string[] lines, ref int i, int baseIndent)
    {
        var first = ListRe.Match(lines[i]);
        var marker = first.Groups[2].Value;
        var list = new ListBlock { Ordered = char.IsDigit(marker[0]) };
        if (list.Ordered && int.TryParse(marker.TrimEnd('.', ')'), out var start)) list.Start = start;
        Item? cur = null;
        while (i < lines.Length)
        {
            var line = lines[i];
            if (line.Trim().Length == 0)
            {
                // 空行后面还是同级列表项才继续
                var j = i + 1;
                while (j < lines.Length && lines[j].Trim().Length == 0) j++;
                if (j < lines.Length && ListRe.Match(lines[j]) is { Success: true } nm && Indent(lines[j]) >= baseIndent && IsOrdered(nm) == list.Ordered) { i = j; continue; }
                if (j < lines.Length && cur != null && Indent(lines[j]) > baseIndent && !ListRe.IsMatch(lines[j])) { i = j; continue; }
                break;
            }
            var m = ListRe.Match(line);
            var indent = Indent(line);
            if (m.Success)
            {
                if (indent >= baseIndent + 2 && cur != null)
                {
                    cur.Child = ParseList(lines, ref i, indent);
                    continue;
                }
                if (indent < baseIndent) break;
                if (IsOrdered(m) != list.Ordered && list.Items.Count > 0) break;
                cur = new Item();
                cur.Lines.Add(m.Groups[3].Value);
                list.Items.Add(cur);
                i++;
                continue;
            }
            if (cur == null || FenceRe.IsMatch(line) || HeadingRe.IsMatch(line)) break;
            // 列表项的续行
            cur.Lines.Add(line.Trim());
            i++;
        }
        return list;
    }

    private static bool IsOrdered(Match m) => char.IsDigit(m.Groups[2].Value[0]);

    private static string[] Cells(string line)
    {
        var t = line.Trim();
        if (t.StartsWith("|", StringComparison.Ordinal)) t = t.Substring(1);
        if (t.EndsWith("|", StringComparison.Ordinal) && !t.EndsWith("\\|", StringComparison.Ordinal)) t = t.Substring(0, t.Length - 1);
        var cells = new List<string>();
        var sb = new StringBuilder();
        for (var k = 0; k < t.Length; k++)
        {
            if (t[k] == '\\' && k + 1 < t.Length && t[k + 1] == '|') { sb.Append('|'); k++; continue; }
            if (t[k] == '|') { cells.Add(sb.ToString().Trim()); sb.Clear(); continue; }
            sb.Append(t[k]);
        }
        cells.Add(sb.ToString().Trim());
        return cells.ToArray();
    }

    // ---------- 行内解析 ----------

    private static readonly Regex UrlRe = new(@"^https?://[^\s<>()\[\]]+[^\s<>()\[\].,;:!?'""]", RegexOptions.Compiled);

    public static List<Inline> Inlines(string s)
    {
        var result = new List<Inline>();
        var sb = new StringBuilder();
        void Flush()
        {
            if (sb.Length == 0) return;
            result.Add(new Run(sb.ToString()));
            sb.Clear();
        }
        var i = 0;
        while (i < s.Length)
        {
            var c = s[i];
            if (c == '\\' && i + 1 < s.Length && "\\`*_[]()#+-.!|>~".IndexOf(s[i + 1]) >= 0)
            {
                sb.Append(s[i + 1]);
                i += 2;
                continue;
            }
            if (c == '`')
            {
                var ticks = 1;
                while (i + ticks < s.Length && s[i + ticks] == '`') ticks++;
                var fence = new string('`', ticks);
                var end = s.IndexOf(fence, i + ticks, StringComparison.Ordinal);
                if (end > i)
                {
                    Flush();
                    result.Add(CodeRun(s.Substring(i + ticks, end - i - ticks).Trim()));
                    i = end + ticks;
                    continue;
                }
            }
            if ((c == '*' || c == '_') && i + 1 < s.Length && s[i + 1] == c)
            {
                var mark = new string(c, 2);
                var end = s.IndexOf(mark, i + 2, StringComparison.Ordinal);
                if (end > i + 2)
                {
                    Flush();
                    var b = new Bold();
                    b.Inlines.AddRange(Inlines(s.Substring(i + 2, end - i - 2)));
                    result.Add(b);
                    i = end + 2;
                    continue;
                }
            }
            if (c == '~' && i + 1 < s.Length && s[i + 1] == '~')
            {
                var end = s.IndexOf("~~", i + 2, StringComparison.Ordinal);
                if (end > i + 2)
                {
                    Flush();
                    var span = new Span { TextDecorations = TextDecorations.Strikethrough };
                    span.Inlines.AddRange(Inlines(s.Substring(i + 2, end - i - 2)));
                    result.Add(span);
                    i = end + 2;
                    continue;
                }
            }
            // 斜体只认 *x*（下划线在路径、变量名里太常见）
            if (c == '*' && i + 1 < s.Length && s[i + 1] != ' ' && s[i + 1] != '*')
            {
                var end = s.IndexOf('*', i + 1);
                if (end > i + 1 && s[end - 1] != ' ')
                {
                    Flush();
                    var it = new Italic();
                    it.Inlines.AddRange(Inlines(s.Substring(i + 1, end - i - 1)));
                    result.Add(it);
                    i = end + 1;
                    continue;
                }
            }
            if (c == '[')
            {
                var close = FindClose(s, i);
                if (close > i && close + 1 < s.Length && s[close + 1] == '(')
                {
                    var paren = s.IndexOf(')', close + 2);
                    if (paren > close + 1)
                    {
                        Flush();
                        var url = s.Substring(close + 2, paren - close - 2).Trim();
                        var sp = url.IndexOf(' ');
                        if (sp > 0) url = url.Substring(0, sp);
                        result.Add(Link(Inlines(s.Substring(i + 1, close - i - 1)), url));
                        i = paren + 1;
                        continue;
                    }
                }
            }
            if ((c == 'h' || c == 'H') && (i == 0 || !char.IsLetterOrDigit(s[i - 1])))
            {
                var m = UrlRe.Match(s.Substring(i));
                if (m.Success)
                {
                    Flush();
                    result.Add(Link(new List<Inline> { new Run(m.Value) }, m.Value));
                    i += m.Length;
                    continue;
                }
            }
            sb.Append(c);
            i++;
        }
        Flush();
        return result;
    }

    private static int FindClose(string s, int open)
    {
        var depth = 0;
        for (var k = open; k < s.Length; k++)
        {
            if (s[k] == '[') depth++;
            else if (s[k] == ']' && --depth == 0) return k;
        }
        return -1;
    }

    private static Inline CodeRun(string code) => new Run(code)
    {
        FontFamily = Th.MonoFont, FontSize = 12.5, Background = Th.CodeBg, Foreground = Th.Text,
    };

    private static Inline Link(List<Inline> content, string url)
    {
        var h = new Hyperlink { Foreground = Th.Accent, TextDecorations = null, Cursor = Cursors.Hand, ToolTip = url };
        h.Inlines.AddRange(content);
        h.MouseEnter += (_, _) => h.TextDecorations = TextDecorations.Underline;
        h.MouseLeave += (_, _) => h.TextDecorations = null;
        h.Click += (_, _) => OpenUrl(url);
        return h;
    }

    /// <summary>只打开 http(s) 与 mailto 链接，其余（file:、javascript: 等）忽略</summary>
    public static void OpenUrl(string url)
    {
        if (AppModel.Current?.SelfTest == true) return;
        if (!Uri.TryCreate(url, UriKind.Absolute, out var u) || u.Scheme is not ("http" or "https" or "mailto")) return;
        try { Process.Start(new ProcessStartInfo(u.AbsoluteUri) { UseShellExecute = true }); }
        catch (Exception e) { AppModel.Current?.ShowError(e); }
    }

    // ---------- 渲染 ----------

    private const double BodySize = 13.5;

    public static FlowDocument Document(string text, bool streaming)
    {
        var doc = new FlowDocument
        {
            FontFamily = Th.UiFont, FontSize = BodySize, Foreground = Th.Text, PagePadding = new Thickness(0), TextAlignment = TextAlignment.Left,
            LineHeight = 22, IsOptimalParagraphEnabled = false,
        };
        foreach (var b in Parse(text)) doc.Blocks.Add(Render(b, 0));
        if (streaming)
        {
            if (doc.Blocks.LastBlock is Paragraph last) last.Inlines.Add(Caret());
            else doc.Blocks.Add(new Paragraph(Caret()) { Margin = new Thickness(0) });
        }
        // 最后一块不留下边距，消息之间的间距由外层控制
        if (doc.Blocks.LastBlock is { } lb) lb.Margin = new Thickness(lb.Margin.Left, lb.Margin.Top, lb.Margin.Right, 0);
        return doc;
    }

    private static System.Windows.Documents.Block Render(Block b, int depth)
    {
        switch (b)
        {
            case Heading h:
            {
                var size = h.Level switch { 1 => 20.0, 2 => 17.5, 3 => 15.5, _ => 14.0 };
                var p = new Paragraph { FontSize = size, FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, h.Level <= 2 ? 14 : 10, 0, 6), LineHeight = size * 1.4 };
                p.Inlines.AddRange(Inlines(h.Text));
                return p;
            }
            case Para para:
            {
                var p = new Paragraph { Margin = new Thickness(0, 0, 0, 10) };
                for (var k = 0; k < para.Lines.Count; k++)
                {
                    if (k > 0) p.Inlines.Add(new LineBreak());
                    p.Inlines.AddRange(Inlines(para.Lines[k]));
                }
                return p;
            }
            case Code c:
                return new BlockUIContainer(CodeBlock(c.Lang, c.Text)) { Margin = new Thickness(0, 2, 0, 12) };
            case Quote q:
            {
                var sec = new Section
                {
                    BorderBrush = Th.B(Th.P.Border), BorderThickness = new Thickness(3, 0, 0, 0), Padding = new Thickness(12, 0, 0, 0), Margin = new Thickness(0, 0, 0, 10),
                    Foreground = Th.Secondary,
                };
                foreach (var inner in Parse(string.Join("\n", q.Lines))) sec.Blocks.Add(Render(inner, depth));
                if (sec.Blocks.LastBlock is { } lb) lb.Margin = new Thickness(lb.Margin.Left, lb.Margin.Top, lb.Margin.Right, 0);
                return sec;
            }
            case Rule:
                return new BlockUIContainer(new Border { Height = 1, Background = Th.Separator, Margin = new Thickness(0, 4, 0, 4) }) { Margin = new Thickness(0, 4, 0, 12) };
            case ListBlock l:
            {
                var list = new List
                {
                    MarkerStyle = l.Ordered ? TextMarkerStyle.Decimal : depth == 0 ? TextMarkerStyle.Disc : TextMarkerStyle.Circle,
                    StartIndex = Math.Max(1, l.Start), Padding = new Thickness(22, 0, 0, 0), Margin = new Thickness(0, 0, 0, depth == 0 ? 10 : 2),
                    MarkerOffset = 8,
                };
                foreach (var item in l.Items)
                {
                    var li = new ListItem();
                    var p = new Paragraph { Margin = new Thickness(0, 0, 0, 3) };
                    for (var k = 0; k < item.Lines.Count; k++)
                    {
                        if (k > 0) p.Inlines.Add(new LineBreak());
                        p.Inlines.AddRange(Inlines(item.Lines[k]));
                    }
                    li.Blocks.Add(p);
                    if (item.Child != null) li.Blocks.Add(Render(item.Child, depth + 1));
                    list.ListItems.Add(li);
                }
                return list;
            }
            case Table t:
                return RenderTable(t);
        }
        return new Paragraph();
    }

    private static System.Windows.Documents.Block RenderTable(Table t)
    {
        var cols = Math.Max(t.Header.Length, t.Rows.Count == 0 ? 0 : t.Rows.Max(r => r.Length));
        var table = new System.Windows.Documents.Table { CellSpacing = 0, Margin = new Thickness(0, 2, 0, 12), BorderBrush = Th.Border, BorderThickness = new Thickness(1, 1, 0, 0) };
        for (var k = 0; k < cols; k++) table.Columns.Add(new TableColumn { Width = GridLength.Auto });
        var group = new TableRowGroup();
        TableRow Row(string[] cells, bool header)
        {
            var row = new TableRow();
            if (header) row.Background = Th.CodeBg;
            for (var k = 0; k < cols; k++)
            {
                var p = new Paragraph { Margin = new Thickness(0), TextAlignment = k < t.Align.Length ? t.Align[k] : TextAlignment.Left, LineHeight = 20 };
                if (header) p.FontWeight = FontWeights.SemiBold;
                p.Inlines.AddRange(Inlines(k < cells.Length ? cells[k] : ""));
                row.Cells.Add(new TableCell(p) { Padding = new Thickness(9, 5, 9, 5), BorderBrush = Th.Border, BorderThickness = new Thickness(0, 0, 1, 1) });
            }
            return row;
        }
        group.Rows.Add(Row(t.Header, true));
        foreach (var r in t.Rows) group.Rows.Add(Row(r, false));
        table.RowGroups.Add(group);
        return table;
    }

    /// <summary>代码块：顶栏是语言名与复制按钮，代码不换行、可横向滚动、可选中</summary>
    public static FrameworkElement CodeBlock(string lang, string code)
    {
        var copy = new Btn { Height = 22, Padding = new Thickness(6, 0, 7, 0), CornerRadius = new CornerRadius(5), HoverBrush = Th.Hover2, Focusable = false };
        void ShowCopy(bool done) => copy.Child = K.Row(4, K.Icon(done ? "check" : "copy", 11.5, Th.Secondary, 2), K.Text(L.T(done ? "common.copied" : "common.copy"), 11.5, Th.Secondary));
        ShowCopy(false);
        copy.UpdateLook();
        copy.Click += () =>
        {
            if (AppModel.Current?.SelfTest != true)
            {
                try { Clipboard.SetText(code); }
                catch (Exception e) { AppModel.Current?.ShowError(e); return; }
            }
            ShowCopy(true);
            Ui.After(1500, () => ShowCopy(false));
        };
        var head = new Border
        {
            Padding = new Thickness(12, 3, 4, 3), BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 0, 0, 1),
            Child = K.Row(6, K.Grow(K.Text(lang.Length == 0 ? "text" : lang, 11.5, Th.Tertiary, mono: true)), copy),
        };
        var box = K.SelectableText(code, 12.5, Th.Text, mono: true, wrap: false);
        box.HorizontalScrollBarVisibility = ScrollBarVisibility.Auto;
        box.VerticalScrollBarVisibility = ScrollBarVisibility.Disabled;
        box.Padding = new Thickness(12, 9, 12, 10);
        Wheel.PassToParent(box);
        var dock = new DockPanel();
        DockPanel.SetDock(head, Dock.Top);
        dock.Children.Add(head);
        dock.Children.Add(box);
        return new Border
        {
            Background = Th.CodeBg, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Child = dock, SnapsToDevicePixels = true,
        };
    }

    /// <summary>流式输出时末尾闪烁的光标</summary>
    private static Inline Caret()
    {
        var bar = new Border { Width = 7, Height = 15, Background = Th.Text, Margin = new Thickness(2, 0, 0, -2), CornerRadius = new CornerRadius(1) };
        if (AppModel.Current?.SelfTest != true)
            bar.BeginAnimation(UIElement.OpacityProperty, new DoubleAnimation(1, 0.1, TimeSpan.FromSeconds(0.55)) { AutoReverse = true, RepeatBehavior = RepeatBehavior.Forever });
        return new InlineUIContainer(bar) { BaselineAlignment = BaselineAlignment.Center };
    }

    /// <summary>显示一条 Markdown 的只读文本区（可跨段选中、链接可点）</summary>
    public static RichTextBox View()
    {
        var r = new RichTextBox
        {
            IsReadOnly = true, IsDocumentEnabled = true, BorderThickness = new Thickness(0), Background = Brushes.Transparent, Padding = new Thickness(0),
            Foreground = Th.Text, SelectionBrush = Th.Accent, CaretBrush = Brushes.Transparent, FocusVisualStyle = null, IsReadOnlyCaretVisible = false,
            VerticalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        };
        Wheel.PassToParent(r);
        return r;
    }
}

/// <summary>内层可滚动控件（文本框、代码块）不吃掉滚轮：交给外层的消息流滚动</summary>
internal static class Wheel
{
    public static void PassToParent(UIElement e)
    {
        e.PreviewMouseWheel += (s, args) =>
        {
            if (args.Handled) return;
            args.Handled = true;
            var parent = VisualTreeHelper.GetParent((DependencyObject)s) as UIElement;
            parent?.RaiseEvent(new MouseWheelEventArgs(args.MouseDevice, args.Timestamp, args.Delta) { RoutedEvent = UIElement.MouseWheelEvent, Source = s });
        };
    }
}
