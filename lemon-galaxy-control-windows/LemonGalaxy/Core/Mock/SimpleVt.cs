using System.Text;

namespace LemonGalaxy.Core.Mock;

/// <summary>
/// 简易 VT100 / xterm 屏幕模型，模拟内核下代替内核的 TerminalScreen（真内核用 Rust 的 vt100）。
/// 支持常用的光标移动、擦除、滚动区域、SGR 颜色（16 / 256 / 真彩色）、备用屏、应用光标键、括号粘贴、鼠标模式、标题、宽字符。
/// 输出的 TermScreenState 与内核同一结构，终端控件不区分真假内核。
/// </summary>
internal sealed class SimpleVt : ITermScreen
{
    private struct Cell
    {
        public string Ch;     // 空字符串表示空白
        public uint Fg, Bg;
        public byte Flags;
        public byte Width;    // 1 普通，2 宽字符首格，0 宽字符的后半格
    }

    private sealed class Line
    {
        public Cell[] Cells;
        public bool Wrapped;
        public Line(int cols) { Cells = NewCells(cols); }
    }

    private static Cell[] NewCells(int n)
    {
        var a = new Cell[n];
        for (var i = 0; i < n; i++) a[i] = new Cell { Ch = "", Width = 1 };
        return a;
    }

    private readonly object lk = new();
    private int rows, cols;
    private readonly int maxScrollback;
    private List<Line> screen;
    private List<Line>? savedMain;
    private readonly List<Line> scrollback = new();
    private int cr, cc;
    private bool pendingWrap;
    private int top, bottom;
    private uint fg, bg;
    private byte flags;
    private (int r, int c, uint fg, uint bg, byte flags) saved;
    private bool appCursor, appKeypad, bracketed, cursorVisible = true, alt, mouseSgr;
    private TermMouseMode mouse = TermMouseMode.None;
    private string title = "";
    private uint bells;
    private uint lastOffset;
    private readonly Decoder utf8 = new UTF8Encoding(false).GetDecoder();

    private enum St { Ground, Esc, Csi, Osc, OscEsc, Charset }
    private St st = St.Ground;
    private readonly StringBuilder csi = new();
    private readonly StringBuilder osc = new();

    public SimpleVt(ushort rows, ushort cols, uint scrollback)
    {
        this.rows = Math.Max(2, (int)rows);
        this.cols = Math.Max(10, (int)cols);
        maxScrollback = (int)Math.Min(scrollback, 100_000);
        screen = Enumerable.Range(0, this.rows).Select(_ => new Line(this.cols)).ToList();
        top = 0;
        bottom = this.rows - 1;
    }

    public void Feed(byte[] data)
    {
        lock (lk)
        {
            var chars = new char[utf8.GetCharCount(data, 0, data.Length)];
            utf8.GetChars(data, 0, data.Length, chars, 0);
            for (var i = 0; i < chars.Length; i++)
            {
                var ch = chars[i];
                if (char.IsHighSurrogate(ch) && i + 1 < chars.Length && char.IsLowSurrogate(chars[i + 1]))
                {
                    Process(new string(new[] { ch, chars[i + 1] }));
                    i++;
                }
                else Process(ch.ToString());
            }
        }
    }

    private void Process(string s)
    {
        var ch = s[0];
        switch (st)
        {
            case St.Ground:
                if (ch == '\x1b') { st = St.Esc; return; }
                if (s.Length == 1 && ch < 0x20 || ch == 0x7f) { Control(ch); return; }
                Print(s);
                return;
            case St.Esc:
                st = St.Ground;
                switch (ch)
                {
                    case '[': csi.Clear(); st = St.Csi; break;
                    case ']': osc.Clear(); st = St.Osc; break;
                    case '(': case ')': case '*': case '+': st = St.Charset; break;
                    case '7': saved = (cr, cc, fg, bg, flags); break;
                    case '8': (cr, cc, fg, bg, flags) = saved; pendingWrap = false; break;
                    case 'D': Index(); break;
                    case 'E': cc = 0; Index(); break;
                    case 'M': ReverseIndex(); break;
                    case 'c': Reset(); break;
                    case '=': appKeypad = true; break;
                    case '>': appKeypad = false; break;
                }
                return;
            case St.Charset:
                st = St.Ground;
                return;
            case St.Osc:
                if (ch == '\a') { FinishOsc(); st = St.Ground; }
                else if (ch == '\x1b') st = St.OscEsc;
                else if (osc.Length < 4096) osc.Append(s);
                return;
            case St.OscEsc:
                FinishOsc();
                st = St.Ground;
                if (ch != '\\') Process(s);
                return;
            case St.Csi:
                if (ch >= 0x40 && ch <= 0x7e) { Csi(ch, csi.ToString()); st = St.Ground; }
                else if (csi.Length < 256) csi.Append(ch);
                return;
        }
    }

    private void FinishOsc()
    {
        var s = osc.ToString();
        var semi = s.IndexOf(';');
        if (semi > 0 && (s.StartsWith("0;", StringComparison.Ordinal) || s.StartsWith("2;", StringComparison.Ordinal))) title = s.Substring(semi + 1);
    }

    private void Control(char ch)
    {
        switch (ch)
        {
            case '\r': cc = 0; pendingWrap = false; break;
            case '\n': case '\v': case '\f': Index(); break;
            case '\b': if (cc > 0) cc--; pendingWrap = false; break;
            case '\t': cc = Math.Min(cols - 1, (cc / 8 + 1) * 8); break;
            case '\a': bells++; break;
        }
    }

    private static bool IsWide(string s)
    {
        var cp = char.ConvertToUtf32(s, 0);
        return cp >= 0x1100 && cp <= 0x115F || cp >= 0x2E80 && cp <= 0xA4CF || cp >= 0xAC00 && cp <= 0xD7A3 || cp >= 0xF900 && cp <= 0xFAFF
               || cp >= 0xFE30 && cp <= 0xFE4F || cp >= 0xFF00 && cp <= 0xFF60 || cp >= 0xFFE0 && cp <= 0xFFE6 || cp >= 0x1F300 && cp <= 0x1FAFF || cp >= 0x20000 && cp <= 0x3FFFD;
    }

    private void Print(string s)
    {
        var w = IsWide(s) ? 2 : 1;
        if (pendingWrap || cc + w > cols)
        {
            if (pendingWrap || cc + w > cols)
            {
                screen[cr].Wrapped = true;
                cc = 0;
                Index();
                screen[cr].Wrapped = false;
            }
            pendingWrap = false;
        }
        var line = screen[cr].Cells;
        line[cc] = new Cell { Ch = s, Fg = fg, Bg = bg, Flags = flags, Width = (byte)w };
        if (w == 2 && cc + 1 < cols) line[cc + 1] = new Cell { Ch = "", Fg = fg, Bg = bg, Flags = flags, Width = 0 };
        cc += w;
        if (cc >= cols) { cc = cols - 1; pendingWrap = true; }
        // 被覆盖的半个宽字符清掉
        if (w == 2 && cc < cols && !pendingWrap && cc < line.Length && line[cc].Width == 0 && cc > 0 && line[cc - 1].Width != 2) line[cc] = Blank();
    }

    private Cell Blank() => new() { Ch = "", Fg = 0, Bg = bg, Flags = 0, Width = 1 };

    private void Index()
    {
        pendingWrap = false;
        if (cr == bottom) ScrollUp(1);
        else if (cr < rows - 1) cr++;
    }

    private void ReverseIndex()
    {
        if (cr == top) ScrollDown(1);
        else if (cr > 0) cr--;
    }

    private void ScrollUp(int n)
    {
        for (var i = 0; i < n; i++)
        {
            var removed = screen[top];
            screen.RemoveAt(top);
            screen.Insert(bottom, new Line(cols));
            if (top == 0 && !alt)
            {
                scrollback.Add(removed);
                if (scrollback.Count > maxScrollback) scrollback.RemoveAt(0);
            }
        }
    }

    private void ScrollDown(int n)
    {
        for (var i = 0; i < n; i++)
        {
            screen.RemoveAt(bottom);
            screen.Insert(top, new Line(cols));
        }
    }

    private static int[] Params(string p, out bool priv)
    {
        priv = p.StartsWith("?", StringComparison.Ordinal);
        if (priv || p.StartsWith(">", StringComparison.Ordinal)) p = p.Substring(1);
        if (p.Length == 0) return Array.Empty<int>();
        return p.Split(';').Select(x => int.TryParse(x.Split(':')[0], out var v) ? v : 0).ToArray();
    }

    private void Csi(char final, string raw)
    {
        var ps = Params(raw, out var priv);
        int P(int i, int def) => i < ps.Length && ps[i] > 0 ? ps[i] : def;
        pendingWrap = false;
        switch (final)
        {
            case 'A': cr = Math.Max(cr < top ? 0 : top, cr - P(0, 1)); break;
            case 'B': cr = Math.Min(cr > bottom ? rows - 1 : bottom, cr + P(0, 1)); break;
            case 'C': cc = Math.Min(cols - 1, cc + P(0, 1)); break;
            case 'D': cc = Math.Max(0, cc - P(0, 1)); break;
            case 'E': cc = 0; cr = Math.Min(rows - 1, cr + P(0, 1)); break;
            case 'F': cc = 0; cr = Math.Max(0, cr - P(0, 1)); break;
            case 'G': case '`': cc = Math.Min(cols - 1, P(0, 1) - 1); break;
            case 'd': cr = Math.Min(rows - 1, P(0, 1) - 1); break;
            case 'H': case 'f': cr = Math.Min(rows - 1, P(0, 1) - 1); cc = Math.Min(cols - 1, P(1, 1) - 1); break;
            case 'J': EraseDisplay(ps.Length > 0 ? ps[0] : 0); break;
            case 'K': EraseLine(ps.Length > 0 ? ps[0] : 0); break;
            case 'L': if (cr >= top && cr <= bottom) for (var i = 0; i < P(0, 1); i++) { screen.RemoveAt(bottom); screen.Insert(cr, new Line(cols)); } break;
            case 'M': if (cr >= top && cr <= bottom) for (var i = 0; i < P(0, 1); i++) { screen.RemoveAt(cr); screen.Insert(bottom, new Line(cols)); } break;
            case 'P': DeleteChars(P(0, 1)); break;
            case '@': InsertChars(P(0, 1)); break;
            case 'X': for (var i = 0; i < P(0, 1) && cc + i < cols; i++) screen[cr].Cells[cc + i] = Blank(); break;
            case 'S': ScrollUp(P(0, 1)); break;
            case 'T': ScrollDown(P(0, 1)); break;
            case 'm': Sgr(ps); break;
            case 'r':
                top = Math.Max(0, P(0, 1) - 1);
                bottom = Math.Min(rows - 1, P(1, rows) - 1);
                if (top >= bottom) { top = 0; bottom = rows - 1; }
                cr = 0; cc = 0;
                break;
            case 's': saved = (cr, cc, fg, bg, flags); break;
            case 'u': (cr, cc, fg, bg, flags) = saved; break;
            case 'h': case 'l':
                if (priv) foreach (var m in ps) Mode(m, final == 'h');
                break;
        }
    }

    private void Mode(int m, bool on)
    {
        switch (m)
        {
            case 1: appCursor = on; break;
            case 25: cursorVisible = on; break;
            case 47: case 1047: case 1049: SetAlt(on, m == 1049); break;
            case 2004: bracketed = on; break;
            case 1000: mouse = on ? TermMouseMode.PressRelease : TermMouseMode.None; break;
            case 1002: mouse = on ? TermMouseMode.ButtonMotion : TermMouseMode.None; break;
            case 1003: mouse = on ? TermMouseMode.AnyMotion : TermMouseMode.None; break;
            case 1006: mouseSgr = on; break;
        }
    }

    private void SetAlt(bool on, bool saveCursor)
    {
        if (on == alt) return;
        if (on)
        {
            if (saveCursor) saved = (cr, cc, fg, bg, flags);
            savedMain = screen;
            screen = Enumerable.Range(0, rows).Select(_ => new Line(cols)).ToList();
            alt = true;
        }
        else
        {
            screen = savedMain ?? screen;
            savedMain = null;
            alt = false;
            if (saveCursor) (cr, cc, fg, bg, flags) = saved;
        }
        top = 0;
        bottom = rows - 1;
    }

    private void EraseDisplay(int mode)
    {
        switch (mode)
        {
            case 0:
                EraseLine(0);
                for (var r = cr + 1; r < rows; r++) screen[r] = new Line(cols);
                break;
            case 1:
                EraseLine(1);
                for (var r = 0; r < cr; r++) screen[r] = new Line(cols);
                break;
            case 2:
                for (var r = 0; r < rows; r++) screen[r] = new Line(cols);
                break;
            case 3:
                scrollback.Clear();
                break;
        }
    }

    private void EraseLine(int mode)
    {
        var c = screen[cr].Cells;
        var (from, to) = mode switch { 0 => (cc, cols - 1), 1 => (0, cc), _ => (0, cols - 1) };
        for (var i = from; i <= to && i < cols; i++) c[i] = Blank();
    }

    private void DeleteChars(int n)
    {
        var c = screen[cr].Cells;
        for (var i = cc; i < cols; i++) c[i] = i + n < cols ? c[i + n] : Blank();
    }

    private void InsertChars(int n)
    {
        var c = screen[cr].Cells;
        for (var i = cols - 1; i >= cc; i--) c[i] = i - n >= cc ? c[i - n] : Blank();
    }

    private void Sgr(int[] ps)
    {
        if (ps.Length == 0) { fg = 0; bg = 0; flags = 0; return; }
        for (var i = 0; i < ps.Length; i++)
        {
            var p = ps[i];
            switch (p)
            {
                case 0: fg = 0; bg = 0; flags = 0; break;
                case 1: flags |= 1; break;
                case 3: flags |= 2; break;
                case 4: flags |= 4; break;
                case 7: flags |= 8; break;
                case 22: flags &= unchecked((byte)~1); break;
                case 23: flags &= unchecked((byte)~2); break;
                case 24: flags &= unchecked((byte)~4); break;
                case 27: flags &= unchecked((byte)~8); break;
                case >= 30 and <= 37: fg = 0x0100_0000u | (uint)(p - 30); break;
                case 39: fg = 0; break;
                case >= 40 and <= 47: bg = 0x0100_0000u | (uint)(p - 40); break;
                case 49: bg = 0; break;
                case >= 90 and <= 97: fg = 0x0100_0000u | (uint)(p - 90 + 8); break;
                case >= 100 and <= 107: bg = 0x0100_0000u | (uint)(p - 100 + 8); break;
                case 38: case 48:
                    uint color = 0;
                    if (i + 2 < ps.Length && ps[i + 1] == 5) { color = 0x0100_0000u | (uint)(ps[i + 2] & 0xFF); i += 2; }
                    else if (i + 4 < ps.Length && ps[i + 1] == 2) { color = 0x0200_0000u | (uint)((ps[i + 2] & 0xFF) << 16 | (ps[i + 3] & 0xFF) << 8 | (ps[i + 4] & 0xFF)); i += 4; }
                    if (p == 38) fg = color; else bg = color;
                    break;
            }
        }
    }

    private void Reset()
    {
        screen = Enumerable.Range(0, rows).Select(_ => new Line(cols)).ToList();
        scrollback.Clear();
        cr = cc = 0;
        fg = bg = 0;
        flags = 0;
        top = 0;
        bottom = rows - 1;
        appCursor = bracketed = alt = false;
        cursorVisible = true;
        mouse = TermMouseMode.None;
    }

    public void Resize(ushort newRows, ushort newCols)
    {
        lock (lk)
        {
            int nr = Math.Max(2, (int)newRows), nc = Math.Max(10, (int)newCols);
            if (nr == rows && nc == cols) return;
            void Fit(List<Line> lines)
            {
                foreach (var l in lines)
                {
                    var n = NewCells(nc);
                    Array.Copy(l.Cells, n, Math.Min(nc, l.Cells.Length));
                    l.Cells = n;
                }
            }
            Fit(screen);
            Fit(scrollback);
            if (savedMain != null) Fit(savedMain);
            // 行数变少时把顶部的行推进历史（光标保持在屏幕里）
            while (screen.Count > nr)
            {
                if (cr > 0 || screen.Count - 1 > cr)
                {
                    if (screen.Count - 1 > cr && screen[screen.Count - 1].Cells.All(x => x.Ch.Length == 0))
                        screen.RemoveAt(screen.Count - 1);
                    else
                    {
                        if (!alt) scrollback.Add(screen[0]);
                        screen.RemoveAt(0);
                        cr = Math.Max(0, cr - 1);
                    }
                }
                else screen.RemoveAt(screen.Count - 1);
            }
            while (screen.Count < nr) screen.Add(new Line(nc));
            rows = nr;
            cols = nc;
            top = 0;
            bottom = rows - 1;
            cr = Math.Min(cr, rows - 1);
            cc = Math.Min(cc, cols - 1);
            pendingWrap = false;
        }
    }

    private List<Line> Visible(uint offset)
    {
        var off = (int)Math.Min(offset, (uint)scrollback.Count);
        if (off == 0) return screen;
        var list = new List<Line>(rows);
        var start = scrollback.Count - off;
        for (var i = 0; i < rows; i++)
        {
            var idx = start + i;
            list.Add(idx < scrollback.Count ? scrollback[idx] : screen[idx - scrollback.Count]);
        }
        return list;
    }

    public TermScreenState State(uint scrollOffset)
    {
        lock (lk)
        {
            var off = Math.Min(scrollOffset, (uint)scrollback.Count);
            lastOffset = off;
            var lines = new List<TermLine>(rows);
            var vis = Visible(off);
            for (var r = 0; r < rows; r++)
            {
                var spans = new List<TermSpan>();
                var sb = new StringBuilder();
                uint sfg = 0, sbg = 0;
                byte sfl = 0;
                ushort cells = 0;
                var started = false;
                foreach (var c in vis[r].Cells)
                {
                    if (c.Width == 0) continue;
                    var text = c.Ch.Length == 0 ? " " : c.Ch;
                    if (started && (c.Fg != sfg || c.Bg != sbg || c.Flags != sfl))
                    {
                        spans.Add(new TermSpan(sb.ToString(), cells, sfg, sbg, sfl));
                        sb.Clear();
                        cells = 0;
                    }
                    started = true;
                    sfg = c.Fg; sbg = c.Bg; sfl = c.Flags;
                    sb.Append(text);
                    cells += c.Width;
                }
                if (started) spans.Add(new TermSpan(sb.ToString(), cells, sfg, sbg, sfl));
                // 行尾默认样式的空白去掉
                if (spans.Count > 0)
                {
                    var last = spans[spans.Count - 1];
                    if (last.Fg == 0 && last.Bg == 0 && last.Flags == 0)
                    {
                        var trimmed = last.Text.TrimEnd(' ');
                        var cut = last.Text.Length - trimmed.Length;
                        spans[spans.Count - 1] = last with { Text = trimmed, Cells = (ushort)(last.Cells - cut) };
                        if (trimmed.Length == 0) spans.RemoveAt(spans.Count - 1);
                    }
                }
                lines.Add(new TermLine(spans.ToArray(), r > 0 && vis[r - 1].Wrapped));
            }
            return new TermScreenState((ushort)rows, (ushort)cols, lines.ToArray(), (ushort)cr, (ushort)cc, cursorVisible && off == 0, title,
                alt, appCursor, appKeypad, bracketed, mouse, mouseSgr, off, (uint)scrollback.Count, bells);
        }
    }

    public string Text()
    {
        lock (lk)
        {
            return string.Join("\n", Visible(lastOffset).Select(l => string.Concat(l.Cells.Where(c => c.Width != 0).Select(c => c.Ch.Length == 0 ? " " : c.Ch)).TrimEnd()));
        }
    }

    public string TextBetween(ushort startRow, ushort startCol, ushort endRow, ushort endCol)
    {
        lock (lk)
        {
            var vis = Visible(lastOffset);
            var sb = new StringBuilder();
            for (int r = startRow; r <= endRow && r < rows; r++)
            {
                var cells = vis[r].Cells;
                int from = r == startRow ? startCol : 0, to = r == endRow ? Math.Min(endCol, (ushort)(cols - 1)) : cols - 1;
                var line = new StringBuilder();
                for (var c = from; c <= to && c < cells.Length; c++)
                    if (cells[c].Width != 0) line.Append(cells[c].Ch.Length == 0 ? " " : cells[c].Ch);
                var text = line.ToString();
                var wrappedNext = r + 1 < rows && vis[r].Wrapped;
                sb.Append(r == endRow ? text : text.TrimEnd());
                if (r != endRow && !wrappedNext) sb.Append('\n');
            }
            return sb.ToString();
        }
    }
}
