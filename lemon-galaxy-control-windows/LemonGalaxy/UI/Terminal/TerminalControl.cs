using System.Globalization;
using System.Text;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

/// <summary>
/// 自绘终端控件：屏幕内容来自 ITermScreen（真内核为 Rust 的 vt100，模拟内核为 SimpleVt），按单元格用 FormattedText 绘制；
/// 键盘转 VT 序列（含应用光标键、Alt 前缀、Ctrl 组合键、功能键），鼠标选中复制、Ctrl+Shift+C / V、滚轮翻看历史、
/// 程序开启鼠标上报时把鼠标事件发给程序，尺寸变化时调整屏幕并通知被控端。
/// </summary>
internal sealed class TerminalControl : FrameworkElement
{
    public readonly ITermScreen Screen;
    public event Action<byte[]>? Input;
    public event Action<ushort, ushort>? Resized;     // cols, rows
    public event Action<string>? TitleChanged;

    public TerminalTheme Theme = TerminalTheme.Fallback;
    public bool CursorBlink = true;
    public bool CopyOnSelect;
    public bool RightClickPaste;
    /// <summary>selftest：离屏渲染时按有焦点的样子画光标</summary>
    public bool ForceFocusedLook;

    private FontFamily family = TerminalFonts.Family("");
    private double fontSize = 14;
    private Typeface regular = null!, bold = null!, italic = null!, boldItalic = null!;
    private double cellW = 8, cellH = 17, baseline;
    private const double PadX = 10, PadY = 6;
    private ushort cols = 80, rows = 24;
    private uint scrollOffset;
    private TermScreenState? state;
    private string lastTitle = "";
    private bool cursorOn = true;
    private readonly DispatcherTimer blink;
    private DispatcherTimer? resizeTimer;
    private (int r, int c)? selStart, selEnd;
    private bool selecting;
    private int mouseButtonDown = -1;

    public TerminalControl(ITermScreen screen)
    {
        Screen = screen;
        Focusable = true;
        FocusVisualStyle = null;
        Cursor = Cursors.IBeam;
        ClipToBounds = true;
        SnapsToDevicePixels = true;
        InputMethod.SetIsInputMethodEnabled(this, true);
        SetFont(TerminalFonts.Family(""), 14);
        blink = new DispatcherTimer(DispatcherPriority.Background) { Interval = TimeSpan.FromMilliseconds(530) };
        blink.Tick += (_, _) =>
        {
            if (!CursorBlink || !IsKeyboardFocused) { if (!cursorOn) { cursorOn = true; InvalidateVisual(); } return; }
            cursorOn = !cursorOn;
            InvalidateVisual();
        };
        GotKeyboardFocus += (_, _) => { cursorOn = true; blink.Start(); InvalidateVisual(); };
        LostKeyboardFocus += (_, _) => { blink.Stop(); cursorOn = true; InvalidateVisual(); };
        ContextMenuOpening += OnContextMenuOpening;
        ContextMenu = new ContextMenu();
    }

    public ushort Cols => cols;
    public ushort Rows => rows;

    public void SetFont(FontFamily f, double size)
    {
        family = f;
        fontSize = size;
        regular = new Typeface(f, FontStyles.Normal, FontWeights.Normal, FontStretches.Normal);
        bold = new Typeface(f, FontStyles.Normal, FontWeights.Bold, FontStretches.Normal);
        italic = new Typeface(f, FontStyles.Italic, FontWeights.Normal, FontStretches.Normal);
        boldItalic = new Typeface(f, FontStyles.Italic, FontWeights.Bold, FontStretches.Normal);
        var probe = Make("MMMMMMMMMM", regular, Brushes.Black);
        cellW = Math.Max(4, probe.WidthIncludingTrailingWhitespace / 10);
        cellH = Math.Max(8, Math.Ceiling(probe.Height + 1));
        baseline = probe.Baseline;
        UpdateGrid(force: true);
        InvalidateVisual();
    }

    private double PixelsPerDip
    {
        get
        {
            try { return VisualTreeHelper.GetDpi(this).PixelsPerDip; }
            catch { return 1; }
        }
    }

    private FormattedText Make(string text, Typeface tf, Brush brush)
        => new(text, CultureInfo.InvariantCulture, FlowDirection.LeftToRight, tf, fontSize, brush, null, TextFormattingMode.Display, PixelsPerDip);

    /// <summary>有新输出时调用（任意线程）：合并到下一次界面刷新</summary>
    public void Changed() => Ui.Coalesce(this, Refresh, DispatcherPriority.Render);

    private void Refresh()
    {
        state = Screen.State(scrollOffset);
        if (state.Title != lastTitle)
        {
            lastTitle = state.Title;
            TitleChanged?.Invoke(lastTitle);
        }
        InvalidateVisual();
    }

    protected override Size MeasureOverride(Size availableSize)
        => new(double.IsInfinity(availableSize.Width) ? 640 : availableSize.Width, double.IsInfinity(availableSize.Height) ? 400 : availableSize.Height);

    protected override void OnRenderSizeChanged(SizeChangedInfo sizeInfo)
    {
        base.OnRenderSizeChanged(sizeInfo);
        UpdateGrid();
    }

    private void UpdateGrid(bool force = false)
    {
        if (ActualWidth <= 0 || ActualHeight <= 0) return;
        var c = (ushort)Math.Max(10, Math.Floor((ActualWidth - PadX * 2 - 8) / cellW));
        var r = (ushort)Math.Max(2, Math.Floor((ActualHeight - PadY * 2) / cellH));
        if (!force && c == cols && r == rows) return;
        if (c == cols && r == rows) { Refresh(); return; }
        cols = c;
        rows = r;
        Screen.Resize(rows, cols);
        selStart = selEnd = null;
        Refresh();
        // 通知被控端稍微延后，拖动窗口时不连发
        resizeTimer?.Stop();
        resizeTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(120) };
        resizeTimer.Tick += (_, _) => { resizeTimer?.Stop(); Resized?.Invoke(cols, rows); };
        resizeTimer.Start();
    }

    /// <summary>selftest：不经过界面布局，直接定好行列</summary>
    public void ForceSize(double width, double height)
    {
        var c = (ushort)Math.Max(10, Math.Floor((width - PadX * 2 - 8) / cellW));
        var r = (ushort)Math.Max(2, Math.Floor((height - PadY * 2) / cellH));
        cols = c;
        rows = r;
        Screen.Resize(rows, cols);
        Refresh();
    }

    // ---------- 绘制 ----------

    private Brush Brush(Color c) => Th.B(c);

    protected override void OnRender(DrawingContext dc)
    {
        var t = Theme;
        dc.DrawRectangle(Brush(t.Background), null, new Rect(0, 0, ActualWidth, ActualHeight));
        var st = state ??= Screen.State(scrollOffset);
        var focused = IsKeyboardFocused || ForceFocusedLook;
        var selBrush = Th.B(Palettes.A(t.SelectionColor, 0.9));
        var (s0, s1) = OrderedSelection();
        for (var r = 0; r < st.Lines.Length && r < rows; r++)
        {
            var y = PadY + r * cellH;
            var x0 = PadX;
            var col = 0;
            // 先画背景与选中，再画文字
            foreach (var span in st.Lines[r].Spans)
            {
                var (fg, bg) = Colors(span);
                if (bg != t.Background) dc.DrawRectangle(Brush(bg), null, new Rect(x0 + col * cellW, y, span.Cells * cellW + 0.5, cellH));
                col += span.Cells;
            }
            if (s0 != null && s1 != null && r >= s0.Value.r && r <= s1.Value.r)
            {
                var from = r == s0.Value.r ? s0.Value.c : 0;
                var to = r == s1.Value.r ? s1.Value.c : cols - 1;
                if (to >= from) dc.DrawRectangle(selBrush, null, new Rect(x0 + from * cellW, y, (to - from + 1) * cellW, cellH));
            }
            col = 0;
            foreach (var span in st.Lines[r].Spans)
            {
                var (fg, _) = Colors(span);
                DrawSpan(dc, span, x0 + col * cellW, y, Brush(fg));
                col += span.Cells;
            }
        }
        // 光标
        if (st.CursorVisible && st.CursorRow < rows && st.CursorCol < cols)
        {
            var rect = new Rect(PadX + st.CursorCol * cellW, PadY + st.CursorRow * cellH, cellW, cellH);
            if (focused && cursorOn)
            {
                dc.DrawRectangle(Brush(t.Cursor), null, rect);
                var ch = CharAt(st, st.CursorRow, st.CursorCol);
                if (ch.Length > 0 && ch != " ") dc.DrawText(Make(ch, regular, Brush(t.Background)), new Point(rect.X, rect.Y));
            }
            else if (!focused) dc.DrawRectangle(null, new Pen(Brush(t.Cursor), 1), new Rect(rect.X + 0.5, rect.Y + 0.5, rect.Width - 1, rect.Height - 1));
        }
        // 翻看历史时右侧显示位置
        if (st.ScrollbackLen > 0 && st.ScrollOffset > 0)
        {
            var total = st.ScrollbackLen + rows;
            var h = Math.Max(24, ActualHeight * rows / total);
            var pos = (double)(st.ScrollbackLen - st.ScrollOffset) / st.ScrollbackLen;
            var top = (ActualHeight - h) * pos;
            dc.DrawRoundedRectangle(Th.B(Palettes.A(t.Foreground, 0.35)), null, new Rect(ActualWidth - 7, top, 4, h), 2, 2);
        }
    }

    private (Color fg, Color bg) Colors(TermSpan s)
    {
        var bold = (s.Flags & 1) != 0;
        var fg = Theme.Resolve(s.Fg, true, bold);
        var bg = Theme.Resolve(s.Bg, false, false);
        if ((s.Flags & 8) != 0) (fg, bg) = (bg, fg);
        return (fg, bg);
    }

    private void DrawSpan(DrawingContext dc, TermSpan span, double x, double y, Brush fg)
    {
        var flags = span.Flags;
        var tf = (flags & 1) != 0 ? ((flags & 2) != 0 ? boldItalic : bold) : (flags & 2) != 0 ? italic : regular;
        var text = span.Text;
        var ascii = text.All(ch => ch < 0x80);
        if (ascii && text.Length == span.Cells)
        {
            if (text.Trim().Length > 0) dc.DrawText(Make(text, tf, fg), new Point(x, y));
        }
        else
        {
            // 非 ASCII（中文、制表符、方块字符）逐个按单元格放，避免回退字体宽度不同导致错位
            var cx = x;
            for (var i = 0; i < text.Length; i++)
            {
                string ch;
                if (char.IsHighSurrogate(text[i]) && i + 1 < text.Length) { ch = text.Substring(i, 2); i++; }
                else ch = text[i].ToString();
                var w = IsWide(ch) ? 2 : 1;
                if (ch != " ")
                {
                    if (IsBlock(ch)) DrawBlock(dc, ch, cx, y, fg);
                    else
                    {
                        var ft = Make(ch, tf, fg);
                        // 宽字符居中放在两格里
                        var off = Math.Max(0, (w * cellW - ft.WidthIncludingTrailingWhitespace) / 2);
                        dc.DrawText(ft, new Point(cx + off, y));
                    }
                }
                cx += w * cellW;
            }
        }
        if ((flags & 4) != 0)
            dc.DrawRectangle(fg, null, new Rect(x, y + baseline + 1.5, span.Cells * cellW, 1));
    }

    /// <summary>块字符按单元格画满，避免字体里的方块不满格</summary>
    private static bool IsBlock(string ch) => ch.Length == 1 && ch[0] >= 0x2580 && ch[0] <= 0x259F;

    private void DrawBlock(DrawingContext dc, string ch, double x, double y, Brush fg)
    {
        var c = ch[0];
        Rect r = c switch
        {
            '▀' => new Rect(x, y, cellW, cellH / 2),
            '▄' => new Rect(x, y + cellH / 2, cellW, cellH / 2),
            '▌' => new Rect(x, y, cellW / 2, cellH),
            '▐' => new Rect(x + cellW / 2, y, cellW / 2, cellH),
            _ => new Rect(x, y, cellW + 0.5, cellH),
        };
        var brush = c switch
        {
            '░' => Th.B(Palettes.A(((SolidColorBrush)fg).Color, 0.25)),
            '▒' => Th.B(Palettes.A(((SolidColorBrush)fg).Color, 0.5)),
            '▓' => Th.B(Palettes.A(((SolidColorBrush)fg).Color, 0.75)),
            _ => fg,
        };
        dc.DrawRectangle(brush, null, r);
    }

    public static bool IsWide(string s)
    {
        var cp = char.ConvertToUtf32(s, 0);
        return cp >= 0x1100 && cp <= 0x115F || cp >= 0x2E80 && cp <= 0xA4CF || cp >= 0xAC00 && cp <= 0xD7A3 || cp >= 0xF900 && cp <= 0xFAFF
               || cp >= 0xFE30 && cp <= 0xFE4F || cp >= 0xFF00 && cp <= 0xFF60 || cp >= 0xFFE0 && cp <= 0xFFE6 || cp >= 0x1F300 && cp <= 0x1FAFF || cp >= 0x20000 && cp <= 0x3FFFD;
    }

    private static string CharAt(TermScreenState st, int row, int col)
    {
        if (row >= st.Lines.Length) return "";
        var c = 0;
        foreach (var span in st.Lines[row].Spans)
        {
            if (col < c + span.Cells)
            {
                var idx = 0;
                var cc = c;
                for (var i = 0; i < span.Text.Length; i++)
                {
                    string ch;
                    if (char.IsHighSurrogate(span.Text[i]) && i + 1 < span.Text.Length) { ch = span.Text.Substring(i, 2); i++; }
                    else ch = span.Text[i].ToString();
                    var w = IsWide(ch) ? 2 : 1;
                    if (col < cc + w) return ch;
                    cc += w;
                    idx++;
                }
                return "";
            }
            c += span.Cells;
        }
        return "";
    }

    // ---------- 键盘 ----------

    private void Send(string s) => Send(Encoding.UTF8.GetBytes(s));

    private void Send(byte[] b)
    {
        if (b.Length == 0) return;
        if (scrollOffset != 0) { scrollOffset = 0; Refresh(); }
        if (selStart != null && !CopyOnSelect) { selStart = selEnd = null; InvalidateVisual(); }
        Input?.Invoke(b);
    }

    private static int ModCode(ModifierKeys m) => 1 + (m.HasFlag(ModifierKeys.Shift) ? 1 : 0) + (m.HasFlag(ModifierKeys.Alt) ? 2 : 0) + (m.HasFlag(ModifierKeys.Control) ? 4 : 0);

    protected override void OnPreviewKeyDown(KeyEventArgs e)
    {
        base.OnPreviewKeyDown(e);
        var key = e.Key == Key.System ? e.SystemKey : e.Key;
        if (key == Key.ImeProcessed || key == Key.DeadCharProcessed) return;
        var mods = Keyboard.Modifiers;
        var ctrl = mods.HasFlag(ModifierKeys.Control);
        var shift = mods.HasFlag(ModifierKeys.Shift);
        var alt = mods.HasFlag(ModifierKeys.Alt);
        var st = state;
        var appCursor = st?.ApplicationCursor ?? false;

        // 本地快捷键（Ctrl+Tab 留给窗口切换标签）
        if (ctrl && key == Key.Tab) return;
        if (ctrl && shift && key == Key.C) { CopySelection(); e.Handled = true; return; }
        if (ctrl && shift && key == Key.V || shift && key == Key.Insert) { Paste(); e.Handled = true; return; }
        if (ctrl && key == Key.Insert) { CopySelection(); e.Handled = true; return; }
        if (shift && key is Key.PageUp or Key.PageDown) { ScrollBy(key == Key.PageUp ? rows - 1 : -(rows - 1)); e.Handled = true; return; }
        if (shift && !ctrl && key is Key.Home or Key.End && st != null) { scrollOffset = key == Key.Home ? st.ScrollbackLen : 0; Refresh(); e.Handled = true; return; }

        string? seq = null;
        var m = ModCode(mods);
        string Csi(string final, string num = "1") => m > 1 ? $"\x1b[{num};{m}{final}" : num == "1" ? $"\x1b[{final}" : $"\x1b[{num}{final}";
        string Cursor(char c) => m > 1 ? $"\x1b[1;{m}{c}" : appCursor ? $"\x1bO{c}" : $"\x1b[{c}";
        string Tilde(int n) => m > 1 ? $"\x1b[{n};{m}~" : $"\x1b[{n}~";
        switch (key)
        {
            case Key.Enter: seq = alt ? "\x1b\r" : "\r"; break;
            case Key.Back: seq = ctrl ? "\x08" : alt ? "\x1b\x7f" : "\x7f"; break;
            case Key.Tab: seq = shift ? "\x1b[Z" : "\t"; break;
            case Key.Escape: seq = "\x1b"; break;
            case Key.Up: seq = Cursor('A'); break;
            case Key.Down: seq = Cursor('B'); break;
            case Key.Right: seq = Cursor('C'); break;
            case Key.Left: seq = Cursor('D'); break;
            case Key.Home: seq = Cursor('H'); break;
            case Key.End: seq = Cursor('F'); break;
            case Key.Insert: seq = Tilde(2); break;
            case Key.Delete: seq = Tilde(3); break;
            case Key.PageUp: seq = Tilde(5); break;
            case Key.PageDown: seq = Tilde(6); break;
            case Key.F1: seq = m > 1 ? Csi("P") : "\x1bOP"; break;
            case Key.F2: seq = m > 1 ? Csi("Q") : "\x1bOQ"; break;
            case Key.F3: seq = m > 1 ? Csi("R") : "\x1bOR"; break;
            case Key.F4: seq = m > 1 ? Csi("S") : "\x1bOS"; break;
            case Key.F5: seq = Tilde(15); break;
            case Key.F6: seq = Tilde(17); break;
            case Key.F7: seq = Tilde(18); break;
            case Key.F8: seq = Tilde(19); break;
            case Key.F9: seq = Tilde(20); break;
            case Key.F10: seq = Tilde(21); break;
            case Key.F11: seq = Tilde(23); break;
            case Key.F12: seq = Tilde(24); break;
        }
        if (seq == null && ctrl && !alt)
        {
            // Ctrl + 字母 / 符号 → 控制字符
            if (key >= Key.A && key <= Key.Z) seq = ((char)(key - Key.A + 1)).ToString();
            else seq = key switch
            {
                Key.Space or Key.D2 => "\0",
                Key.OemOpenBrackets => "\x1b",
                Key.Oem5 => "\x1c",
                Key.OemCloseBrackets => "\x1d",
                Key.D6 => "\x1e",
                Key.OemMinus => "\x1f",
                Key.OemQuestion => "\x1f",
                _ => null,
            };
        }
        else if (seq == null && alt && !ctrl)
        {
            // Alt + 字母 / 数字 → Esc 前缀（emacs、bash 快捷键）
            if (key >= Key.A && key <= Key.Z) seq = "\x1b" + (char)((shift ? 'A' : 'a') + (key - Key.A));
            else if (key >= Key.D0 && key <= Key.D9 && !shift) seq = "\x1b" + (char)('0' + (key - Key.D0));
            else if (key == Key.OemPeriod) seq = "\x1b.";
        }
        if (seq != null)
        {
            Send(seq);
            e.Handled = true;
        }
    }

    protected override void OnTextInput(TextCompositionEventArgs e)
    {
        base.OnTextInput(e);
        var text = e.Text;
        if (string.IsNullOrEmpty(text)) return;
        // 控制字符已在按键里处理
        if (text.Length == 1 && (text[0] < 0x20 || text[0] == 0x7f)) return;
        Send(text);
        e.Handled = true;
    }

    // ---------- 复制粘贴与选中 ----------

    private ((int r, int c)? a, (int r, int c)? b) OrderedSelection()
    {
        if (selStart == null || selEnd == null || selStart == selEnd) return (null, null);
        var a = selStart.Value;
        var b = selEnd.Value;
        if (a.r > b.r || a.r == b.r && a.c > b.c) (a, b) = (b, a);
        return (a, b);
    }

    public string SelectedText()
    {
        var (a, b) = OrderedSelection();
        if (a == null || b == null) return "";
        return Screen.TextBetween((ushort)a.Value.r, (ushort)a.Value.c, (ushort)b.Value.r, (ushort)b.Value.c);
    }

    public void CopySelection()
    {
        var t = SelectedText();
        if (t.Length > 0) SysClipboard.SetText(t);
    }

    public void Paste()
    {
        var t = SysClipboard.GetText();
        if (string.IsNullOrEmpty(t)) return;
        PasteText(t!);
    }

    public void PasteText(string t)
    {
        t = t.Replace("\r\n", "\r").Replace("\n", "\r");
        if (state?.BracketedPaste == true) t = "\x1b[200~" + t.Replace("\x1b[201~", "") + "\x1b[201~";
        Send(t);
    }

    public void SelectAll()
    {
        selStart = (0, 0);
        selEnd = (rows - 1, cols - 1);
        InvalidateVisual();
    }

    private (int r, int c) CellAt(Point p)
    {
        var c = (int)Math.Floor((p.X - PadX) / cellW);
        var r = (int)Math.Floor((p.Y - PadY) / cellH);
        return (Math.Max(0, Math.Min(rows - 1, r)), Math.Max(0, Math.Min(cols - 1, c)));
    }

    private bool MouseReporting(MouseEventArgs e) => state is { } st && st.MouseMode != TermMouseMode.None && !Keyboard.Modifiers.HasFlag(ModifierKeys.Shift);

    private void Report(int button, bool press, Point p, bool motion = false)
    {
        var st = state;
        if (st == null) return;
        var (r, c) = CellAt(p);
        var mods = Keyboard.Modifiers;
        var b = button + (motion ? 32 : 0) + (mods.HasFlag(ModifierKeys.Shift) ? 4 : 0) + (mods.HasFlag(ModifierKeys.Alt) ? 8 : 0) + (mods.HasFlag(ModifierKeys.Control) ? 16 : 0);
        string s;
        if (st.MouseSgr) s = $"\x1b[<{b};{c + 1};{r + 1}{(press ? 'M' : 'm')}";
        else
        {
            var code = press ? b : 3 + (b & ~3);
            if (c > 222 || r > 222) return;
            s = "\x1b[M" + (char)(32 + code) + (char)(33 + c) + (char)(33 + r);
        }
        Input?.Invoke(Encoding.UTF8.GetBytes(s));
    }

    protected override void OnMouseLeftButtonDown(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonDown(e);
        Focus();
        Keyboard.Focus(this);
        var p = e.GetPosition(this);
        if (MouseReporting(e))
        {
            Report(0, true, p);
            mouseButtonDown = 0;
            CaptureMouse();
            e.Handled = true;
            return;
        }
        var cell = CellAt(p);
        if (e.ClickCount == 2) { SelectWord(cell); e.Handled = true; return; }
        if (e.ClickCount >= 3) { selStart = (cell.r, 0); selEnd = (cell.r, cols - 1); InvalidateVisual(); AfterSelect(); e.Handled = true; return; }
        selStart = selEnd = cell;
        selecting = true;
        CaptureMouse();
        InvalidateVisual();
        e.Handled = true;
    }

    private void SelectWord((int r, int c) cell)
    {
        var st = state;
        if (st == null) return;
        bool Word(string ch) => ch.Length > 0 && (char.IsLetterOrDigit(ch, 0) || "_-./~:@".Contains(ch[0]) || ch[0] > 0x2E80);
        if (!Word(CharAt(st, cell.r, cell.c))) return;
        var a = cell.c;
        while (a > 0 && Word(CharAt(st, cell.r, a - 1))) a--;
        var b = cell.c;
        while (b < cols - 1 && Word(CharAt(st, cell.r, b + 1))) b++;
        selStart = (cell.r, a);
        selEnd = (cell.r, b);
        InvalidateVisual();
        AfterSelect();
    }

    private void AfterSelect()
    {
        if (CopyOnSelect) CopySelection();
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);
        var p = e.GetPosition(this);
        if (mouseButtonDown >= 0 && state is { MouseMode: TermMouseMode.ButtonMotion or TermMouseMode.AnyMotion })
        {
            Report(mouseButtonDown, true, p, motion: true);
            return;
        }
        if (!selecting) return;
        var cell = CellAt(p);
        if (selEnd != cell)
        {
            selEnd = cell;
            InvalidateVisual();
        }
    }

    protected override void OnMouseLeftButtonUp(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonUp(e);
        if (mouseButtonDown == 0)
        {
            mouseButtonDown = -1;
            ReleaseMouseCapture();
            if (state is { MouseMode: not TermMouseMode.Press }) Report(0, false, e.GetPosition(this));
            return;
        }
        if (!selecting) return;
        selecting = false;
        ReleaseMouseCapture();
        if (selStart == selEnd) { selStart = selEnd = null; InvalidateVisual(); }
        else AfterSelect();
    }

    protected override void OnMouseRightButtonDown(MouseButtonEventArgs e)
    {
        if (MouseReporting(e))
        {
            Report(2, true, e.GetPosition(this));
            mouseButtonDown = 2;
            e.Handled = true;
            return;
        }
        if (RightClickPaste)
        {
            if (selStart != null && selStart != selEnd) { CopySelection(); selStart = selEnd = null; InvalidateVisual(); }
            else Paste();
            e.Handled = true;
            return;
        }
        base.OnMouseRightButtonDown(e);
    }

    protected override void OnMouseRightButtonUp(MouseButtonEventArgs e)
    {
        if (mouseButtonDown == 2)
        {
            mouseButtonDown = -1;
            if (state is { MouseMode: not TermMouseMode.Press }) Report(2, false, e.GetPosition(this));
            e.Handled = true;
            return;
        }
        if (RightClickPaste) { e.Handled = true; return; }
        base.OnMouseRightButtonUp(e);
    }

    private void OnContextMenuOpening(object sender, ContextMenuEventArgs e)
    {
        var hasSel = SelectedText().Length > 0;
        ContextMenu = K.Menu(new List<MenuEntry>
        {
            new(L.T("terminal.copy") + "\tCtrl+Shift+C", CopySelection, hasSel),
            new(L.T("terminal.paste") + "\tCtrl+Shift+V", Paste),
            new(L.T("menu.selectAll"), SelectAll),
        });
    }

    protected override void OnMouseWheel(MouseWheelEventArgs e)
    {
        base.OnMouseWheel(e);
        var st = state;
        var lines = Math.Max(1, Math.Abs(e.Delta) / 40);
        if (st != null && MouseReporting(e))
        {
            for (var i = 0; i < Math.Max(1, lines / 3); i++) Report(e.Delta > 0 ? 64 : 65, true, e.GetPosition(this));
        }
        else if (st is { AlternateScreen: true })
        {
            // 全屏程序（less、vim）里滚轮发方向键
            var seq = st.ApplicationCursor ? (e.Delta > 0 ? "\x1bOA" : "\x1bOB") : (e.Delta > 0 ? "\x1b[A" : "\x1b[B");
            Input?.Invoke(Encoding.ASCII.GetBytes(string.Concat(Enumerable.Repeat(seq, lines))));
        }
        else ScrollBy(e.Delta > 0 ? lines : -lines);
        e.Handled = true;
    }

    public void ScrollBy(int lines)
    {
        var max = state?.ScrollbackLen ?? 0;
        var next = (uint)Math.Max(0, Math.Min((long)max, (long)scrollOffset + lines));
        if (next == scrollOffset) return;
        scrollOffset = next;
        selStart = selEnd = null;
        Refresh();
    }
}
