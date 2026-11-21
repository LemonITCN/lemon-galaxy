using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Shapes;

namespace LemonGalaxy.UI;

/// <summary>可点击的块：悬停、按下底色，Enter / 空格触发，禁用时半透明。界面里所有按钮都基于它（不用系统按钮外观）。</summary>
internal class Btn : Border
{
    public event Action? Click;
    public event Action? DoubleClick;
    public Brush? Normal;
    public Brush? HoverBrush;
    public Brush? PressBrush;
    public Brush? NormalBorder;
    public Brush? HoverBorder;
    private bool hover, down;

    public Btn()
    {
        Cursor = Cursors.Hand;
        Focusable = true;
        FocusVisualStyle = Styles.FocusVisual;
        Background = Brushes.Transparent;
        SnapsToDevicePixels = true;
        // 自绘标题栏区域里的按钮也要能点
        System.Windows.Shell.WindowChrome.SetIsHitTestVisibleInChrome(this, true);
        IsEnabledChanged += (_, _) =>
        {
            Opacity = IsEnabled ? 1 : 0.45;
            Cursor = IsEnabled ? Cursors.Hand : Cursors.Arrow;
            UpdateLook();
        };
    }

    public bool Enabled { get => IsEnabled; set => IsEnabled = value; }

    public void UpdateLook()
    {
        var bg = down && hover ? PressBrush ?? HoverBrush : hover && IsEnabled ? HoverBrush : null;
        Background = bg ?? Normal ?? Brushes.Transparent;
        var bb = hover && IsEnabled ? HoverBorder ?? NormalBorder : NormalBorder;
        if (bb != null) BorderBrush = bb;
    }

    public void PerformClick() => Click?.Invoke();

    /// <summary>开始拖放时取消按下状态（不再触发点击）</summary>
    public void CancelPress()
    {
        down = false;
        if (IsMouseCaptured) ReleaseMouseCapture();
        UpdateLook();
    }

    public bool HasClick => Click != null;

    protected override void OnMouseEnter(MouseEventArgs e) { hover = true; UpdateLook(); base.OnMouseEnter(e); }
    protected override void OnMouseLeave(MouseEventArgs e) { hover = false; UpdateLook(); base.OnMouseLeave(e); }

    protected override void OnMouseLeftButtonDown(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonDown(e);
        if (e.ClickCount == 2 && DoubleClick != null)
        {
            e.Handled = true;
            DoubleClick();
            return;
        }
        down = true;
        CaptureMouse();
        UpdateLook();
        e.Handled = true;
    }

    protected override void OnMouseLeftButtonUp(MouseButtonEventArgs e)
    {
        base.OnMouseLeftButtonUp(e);
        if (!down) return;
        down = false;
        ReleaseMouseCapture();
        UpdateLook();
        var p = e.GetPosition(this);
        if (p.X >= 0 && p.Y >= 0 && p.X <= ActualWidth && p.Y <= ActualHeight) Click?.Invoke();
        e.Handled = true;
    }

    protected override void OnKeyDown(KeyEventArgs e)
    {
        base.OnKeyDown(e);
        if (e.Key is Key.Enter or Key.Space && Click != null)
        {
            Click();
            e.Handled = true;
        }
    }
}

/// <summary>输入框：圆角底框 + 无边框的系统输入控件 + 占位文字。单行、密码、多行三种。</summary>
internal sealed class Field : Border
{
    public readonly TextBox? Box;
    public readonly PasswordBox? Pwd;
    private readonly TextBlock placeholder;
    private bool invalid;
    public event Action<string>? Changed;
    public event Action? Submitted;
    public event Action? Escaped;
    /// <summary>多行时 Ctrl+Enter 触发 Submitted（AI 对话框）</summary>
    public bool CtrlEnterSubmits;

    public Field(string placeholderText = "", string text = "", bool password = false, bool mono = false, bool multiline = false, double fontSize = 13)
    {
        CornerRadius = new CornerRadius(7);
        Background = Th.Elevated;
        BorderBrush = Th.Border;
        BorderThickness = new Thickness(1);
        Padding = multiline ? new Thickness(8, 6, 8, 6) : new Thickness(9, 0, 9, 0);
        MinHeight = 30;
        SnapsToDevicePixels = true;
        var grid = new Grid();
        placeholder = new TextBlock
        {
            Text = placeholderText, Foreground = Th.Tertiary, FontSize = fontSize, IsHitTestVisible = false,
            VerticalAlignment = multiline ? VerticalAlignment.Top : VerticalAlignment.Center,
            Margin = new Thickness(2, multiline ? 1 : 0, 0, 0), TextTrimming = TextTrimming.CharacterEllipsis,
            FontFamily = mono ? Th.MonoFont : Th.UiFont,
        };
        Control input;
        if (password)
        {
            Pwd = new PasswordBox { Password = text };
            Pwd.PasswordChanged += (_, _) => { UpdatePlaceholder(); Changed?.Invoke(Pwd.Password); };
            input = Pwd;
        }
        else
        {
            Box = new TextBox { Text = text, AcceptsReturn = multiline, TextWrapping = multiline ? TextWrapping.Wrap : TextWrapping.NoWrap };
            if (multiline) Box.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
            Box.TextChanged += (_, _) => { UpdatePlaceholder(); Changed?.Invoke(Box.Text); };
            input = Box;
        }
        input.BorderThickness = new Thickness(0);
        input.Background = Brushes.Transparent;
        input.Foreground = Th.Text;
        input.FontSize = fontSize;
        input.FontFamily = mono ? Th.MonoFont : Th.UiFont;
        input.Padding = new Thickness(0);
        input.VerticalContentAlignment = multiline ? VerticalAlignment.Top : VerticalAlignment.Center;
        input.VerticalAlignment = multiline ? VerticalAlignment.Stretch : VerticalAlignment.Center;
        input.FocusVisualStyle = null;
        if (input is TextBox tb) { tb.CaretBrush = Th.Text; tb.SelectionBrush = Th.Accent; }
        if (input is PasswordBox pb) { pb.CaretBrush = Th.Text; pb.SelectionBrush = Th.Accent; }
        input.PreviewKeyDown += (_, e) =>
        {
            if (e.Key == Key.Enter && (!multiline || CtrlEnterSubmits && Keyboard.Modifiers.HasFlag(ModifierKeys.Control)))
            {
                if (Submitted != null) { Submitted(); e.Handled = true; }
            }
            else if (e.Key == Key.Escape && Escaped != null) { Escaped(); e.Handled = true; }
        };
        input.GotKeyboardFocus += (_, _) => UpdateBorder(true);
        input.LostKeyboardFocus += (_, _) => UpdateBorder(false);
        grid.Children.Add(placeholder);
        grid.Children.Add(input);
        Child = grid;
        UpdatePlaceholder();
    }

    public string Text
    {
        get => Box?.Text ?? Pwd?.Password ?? "";
        set
        {
            if (Box != null) Box.Text = value;
            else if (Pwd != null) Pwd.Password = value;
        }
    }

    public bool Invalid
    {
        get => invalid;
        set { invalid = value; UpdateBorder(IsKeyboardFocusWithin); }
    }

    public Control Input => (Control?)Box ?? Pwd!;

    public void FocusInput()
    {
        Input.Focus();
        Keyboard.Focus(Input);
        Box?.SelectAll();
    }

    private void UpdatePlaceholder() => placeholder.Visibility = Text.Length == 0 ? Visibility.Visible : Visibility.Collapsed;

    private void UpdateBorder(bool focused)
    {
        BorderBrush = invalid ? Th.Danger : focused ? Th.B(Th.P.Accent, 0.8) : Th.Border;
    }
}

/// <summary>开关（用界面配色的强调色画）</summary>
internal sealed class Toggle : Btn
{
    private bool on;
    private readonly Border track;
    private readonly Ellipse thumb;
    public event Action<bool>? Changed;

    public Toggle(bool isOn, bool small = true)
    {
        on = isOn;
        double w = small ? 32 : 38, h = small ? 18 : 22;
        Width = w;
        Height = h;
        CornerRadius = new CornerRadius(h / 2);
        track = new Border { CornerRadius = new CornerRadius(h / 2) };
        thumb = new Ellipse { Width = h - 4, Height = h - 4, Fill = Brushes.White, Margin = new Thickness(2) };
        thumb.Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 2, ShadowDepth = 0.5, Opacity = 0.25, Direction = 270 };
        var g = new Grid();
        g.Children.Add(track);
        g.Children.Add(thumb);
        Child = g;
        Click += () => { IsOn = !on; Changed?.Invoke(on); };
        Render();
    }

    public bool IsOn { get => on; set { on = value; Render(); } }

    private void Render()
    {
        track.Background = on ? Th.Accent : Th.B(Th.P.IsDark ? Palettes.A(Colors.White, 0.16) : Palettes.A(Colors.Black, 0.14));
        thumb.HorizontalAlignment = on ? HorizontalAlignment.Right : HorizontalAlignment.Left;
    }
}

internal enum DotState { Online, Lan, Offline, Busy }

internal sealed record MenuEntry(string Title, Action? Action = null, bool Enabled = true, bool Checked = false, bool Destructive = false,
    List<MenuEntry>? Sub = null, bool IsSeparator = false)
{
    public static MenuEntry Sep => new("", IsSeparator: true);
}

/// <summary>界面小部件工厂</summary>
internal static class K
{
    public static TextBlock Text(string s, double size = 13, Brush? fg = null, FontWeight? weight = null, bool wrap = false, bool mono = false, bool select = false)
    {
        var t = new TextBlock
        {
            Text = s, FontSize = size, Foreground = fg ?? Th.Text, FontWeight = weight ?? FontWeights.Normal,
            TextWrapping = wrap ? TextWrapping.Wrap : TextWrapping.NoWrap,
            TextTrimming = wrap ? TextTrimming.None : TextTrimming.CharacterEllipsis,
            FontFamily = mono ? Th.MonoFont : Th.UiFont,
        };
        _ = select;
        return t;
    }

    /// <summary>可选中复制的文字（只读、无边框的输入框）</summary>
    public static TextBox SelectableText(string s, double size = 13, Brush? fg = null, bool mono = false, bool wrap = true)
    {
        return new TextBox
        {
            Text = s, IsReadOnly = true, BorderThickness = new Thickness(0), Background = Brushes.Transparent, Padding = new Thickness(0),
            Foreground = fg ?? Th.Text, FontSize = size, FontFamily = mono ? Th.MonoFont : Th.UiFont,
            TextWrapping = wrap ? TextWrapping.Wrap : TextWrapping.NoWrap, SelectionBrush = Th.Accent, CaretBrush = Brushes.Transparent,
            FocusVisualStyle = null, Cursor = Cursors.IBeam,
        };
    }

    public static TextBlock Semi(string s, double size = 13, Brush? fg = null) => Text(s, size, fg, FontWeights.SemiBold);

    public static FrameworkElement Icon(string name, double size = 14, Brush? brush = null, double stroke = 2) => Icons.Make(name, size, brush ?? Th.Secondary, stroke);

    public static Flex Row(double spacing, params UIElement?[] children)
    {
        var f = new Flex { Horizontal = true, Spacing = spacing };
        foreach (var c in children) if (c != null) { Region.Detach(c); f.Children.Add(c); }
        return f;
    }

    public static Flex Col(double spacing, params UIElement?[] children)
    {
        var f = new Flex { Horizontal = false, Spacing = spacing };
        foreach (var c in children) if (c != null) { Region.Detach(c); f.Children.Add(c); }
        return f;
    }

    public static Flex Col(double spacing, IEnumerable<UIElement?> children) => Col(spacing, children.ToArray());
    public static Flex Row(double spacing, IEnumerable<UIElement?> children) => Row(spacing, children.ToArray());

    public static T Grow<T>(T e, double g = 1) where T : UIElement { Flex.SetGrow(e, g); return e; }

    public static FrameworkElement Spacer(double min = 0) => Grow(new Border { MinWidth = min, MinHeight = min });

    public static FrameworkElement Gap(double size) => new Border { Width = size, Height = size };

    public static T Margin<T>(T e, double l, double t, double r, double b) where T : FrameworkElement { e.Margin = new Thickness(l, t, r, b); return e; }
    public static T Margin<T>(T e, double all) where T : FrameworkElement { e.Margin = new Thickness(all); return e; }
    public static T Width<T>(T e, double w) where T : FrameworkElement { e.Width = w; return e; }
    public static T Height<T>(T e, double h) where T : FrameworkElement { e.Height = h; return e; }
    public static T Tip<T>(T e, string? tip) where T : FrameworkElement { if (!string.IsNullOrEmpty(tip)) e.ToolTip = tip; return e; }
    public static T Left<T>(T e) where T : FrameworkElement { e.HorizontalAlignment = HorizontalAlignment.Left; return e; }
    public static T Top<T>(T e) where T : FrameworkElement { e.VerticalAlignment = VerticalAlignment.Top; return e; }

    public static Border Pad(UIElement child, double l, double t, double r, double b) => new() { Padding = new Thickness(l, t, r, b), Child = Detached(child) };
    public static Border Pad(UIElement child, double all) => new() { Padding = new Thickness(all), Child = Detached(child) };

    public static Border Box(UIElement? child, Brush? bg = null, double radius = 0, Brush? border = null, double borderWidth = 1, Thickness? padding = null)
        => new()
        {
            Child = Detached(child), Background = bg, CornerRadius = new CornerRadius(radius), BorderBrush = border,
            BorderThickness = border == null ? new Thickness(0) : new Thickness(borderWidth), Padding = padding ?? new Thickness(0), SnapsToDevicePixels = true,
        };

    private static UIElement? Detached(UIElement? e) { Region.Detach(e); return e; }

    public static Rectangle HLine(Brush? b = null) => new() { Height = 1, Fill = b ?? Th.Separator, SnapsToDevicePixels = true };

    public static Rectangle VLine(Brush? b = null, double height = double.NaN)
    {
        var r = new Rectangle { Width = 1, Fill = b ?? Th.Separator, SnapsToDevicePixels = true };
        if (!double.IsNaN(height)) r.Height = height;
        else Flex.SetStretch(r, true);
        return r;
    }

    // ---------- 按钮 ----------

    public static Btn PrimaryButton(string title, Action onClick, bool destructive = false, string? icon = null, bool enabled = true)
    {
        var fg = destructive ? Brushes.White : Th.AccentText;
        var bgc = destructive ? Th.P.Danger : Th.P.Accent;
        var b = new Btn
        {
            CornerRadius = new CornerRadius(7), Height = 30, Padding = new Thickness(14, 0, 14, 0),
            Normal = Th.B(bgc), HoverBrush = Th.B(Mix(bgc, Th.P.IsDark ? Colors.White : Colors.Black, 0.08)), PressBrush = Th.B(Mix(bgc, Colors.Black, 0.16)),
            Child = ButtonContent(title, icon, fg, FontWeights.SemiBold),
        };
        b.UpdateLook();
        b.Click += onClick;
        b.IsEnabled = enabled;
        return b;
    }

    public static Btn SecondaryButton(string title, Action onClick, bool destructive = false, string? icon = null, bool enabled = true)
    {
        var b = new Btn
        {
            CornerRadius = new CornerRadius(7), Height = 30, Padding = new Thickness(12, 0, 12, 0), BorderThickness = new Thickness(1),
            Normal = Th.Elevated, HoverBrush = Th.B(Mix(Th.P.Elevated, Th.P.Text, 0.06)), PressBrush = Th.B(Mix(Th.P.Elevated, Th.P.Text, 0.12)),
            NormalBorder = Th.Border,
            Child = ButtonContent(title, icon, destructive ? Th.Danger : Th.Text, FontWeights.Medium),
        };
        b.UpdateLook();
        b.Click += onClick;
        b.IsEnabled = enabled;
        return b;
    }

    /// <summary>浅色底的强调按钮（例如「断开」「停止」）</summary>
    public static Btn TintButton(string title, Action onClick, Color color, string? icon = null, double height = 26)
    {
        var b = new Btn
        {
            CornerRadius = new CornerRadius(6), Height = height, Padding = new Thickness(10, 0, 10, 0),
            Normal = Th.B(color, 0.12), HoverBrush = Th.B(color, 0.2), PressBrush = Th.B(color, 0.28),
            Child = ButtonContent(title, icon, Th.B(color), FontWeights.Medium, 12.5),
        };
        b.UpdateLook();
        b.Click += onClick;
        return b;
    }

    private static UIElement ButtonContent(string title, string? icon, Brush fg, FontWeight weight, double size = 13)
    {
        var t = Text(title, size, fg, weight);
        t.VerticalAlignment = VerticalAlignment.Center;
        if (icon == null) return t;
        return Row(6, Icon(icon, size, fg, 2.2), t);
    }

    public static Color Mix(Color a, Color b, double t) => Color.FromArgb(
        (byte)(a.A + (b.A - a.A) * t), (byte)(a.R + (b.R - a.R) * t), (byte)(a.G + (b.G - a.G) * t), (byte)(a.B + (b.B - a.B) * t));

    /// <summary>无边框图标按钮（工具栏用）</summary>
    public static Btn IconButton(string icon, string? tip, Action onClick, bool active = false, double size = 14, Brush? color = null, double w = 28, double h = 26)
    {
        var b = new Btn
        {
            Width = w, Height = h, CornerRadius = new CornerRadius(6),
            Normal = active ? Th.B(Th.P.Accent, 0.14) : Brushes.Transparent,
            HoverBrush = active ? Th.B(Th.P.Accent, 0.2) : Th.Hover2,
            Child = Icon(icon, size, active ? Th.Accent : color ?? Th.Secondary),
        };
        b.UpdateLook();
        Tip(b, tip);
        b.Click += onClick;
        return b;
    }

    /// <summary>纯文字的链接式按钮</summary>
    public static Btn LinkButton(string title, Action onClick, Brush? fg = null, double size = 12)
    {
        var b = new Btn { Padding = new Thickness(4, 2, 4, 2), CornerRadius = new CornerRadius(4), HoverBrush = Th.Hover2, Child = Text(title, size, fg ?? Th.Secondary) };
        b.UpdateLook();
        b.Click += onClick;
        return b;
    }

    // ---------- 输入 ----------

    public static Field TextField(string placeholder = "", string text = "", Action<string>? changed = null, bool mono = false, double width = double.NaN, double fontSize = 13)
    {
        var f = new Field(placeholder, text, mono: mono, fontSize: fontSize);
        if (!double.IsNaN(width)) f.Width = width;
        if (changed != null) f.Changed += changed;
        return f;
    }

    public static Field PasswordField(string placeholder = "", string text = "", Action<string>? changed = null, double width = double.NaN)
    {
        var f = new Field(placeholder, text, password: true);
        if (!double.IsNaN(width)) f.Width = width;
        if (changed != null) f.Changed += changed;
        return f;
    }

    public static Field TextArea(string placeholder, string text, double height, Action<string>? changed = null, bool mono = false, double fontSize = 13)
    {
        var f = new Field(placeholder, text, multiline: true, mono: mono, fontSize: fontSize) { Height = height };
        if (changed != null) f.Changed += changed;
        return f;
    }

    /// <summary>带放大镜图标的搜索框</summary>
    public static FrameworkElement SearchField(string placeholder, string text, Action<string> changed, out Field field)
    {
        var f = new Field(placeholder, text) { Height = 28, MinHeight = 28, Padding = new Thickness(28, 0, 8, 0) };
        f.Changed += changed;
        f.Escaped += () => f.Text = "";
        field = f;
        var icon = Icon("search", 13, Th.Tertiary);
        icon.HorizontalAlignment = HorizontalAlignment.Left;
        icon.VerticalAlignment = VerticalAlignment.Center;
        icon.Margin = new Thickness(10, 0, 0, 0);
        return new Grid { Children = { f, icon } };
    }

    public static Toggle Switch(bool on, Action<bool> changed, bool small = true)
    {
        var t = new Toggle(on, small);
        t.Changed += changed;
        return t;
    }

    public static Btn CheckBox(string label, bool on, Action<bool> changed, double size = 13)
    {
        var state = on;
        var box = new Border { Width = 16, Height = 16, CornerRadius = new CornerRadius(4), BorderThickness = new Thickness(1) };
        void Render()
        {
            box.Background = state ? Th.Accent : Th.Elevated;
            box.BorderBrush = state ? Th.Accent : Th.B(Th.P.SecondaryText, 0.6);
            box.Child = state ? Icon("check", 12, Th.AccentText, 3) : null;
        }
        Render();
        var b = new Btn { Child = label.Length == 0 ? box : Row(7, box, Text(label, size)), Padding = new Thickness(0, 2, 2, 2) };
        b.Click += () => { state = !state; Render(); changed(state); };
        return b;
    }

    /// <summary>下拉选择：按钮显示当前值，点开是主题化的菜单</summary>
    public static Btn Dropdown<T>(IEnumerable<(T value, string label)> items, T current, Action<T> onPick, double minWidth = 0, string? icon = null)
    {
        var list = items.ToList();
        var label = Text(list.FirstOrDefault(i => Equals(i.value, current)).label ?? "", 13);
        var b = new Btn
        {
            CornerRadius = new CornerRadius(7), Height = 30, Padding = new Thickness(10, 0, 8, 0), BorderThickness = new Thickness(1),
            Normal = Th.Elevated, HoverBrush = Th.B(Mix(Th.P.Elevated, Th.P.Text, 0.05)), NormalBorder = Th.Border, MinWidth = minWidth,
            Child = Row(8, icon == null ? null : Icon(icon, 13), Grow(label), Icon("chevronDown", 12, Th.Tertiary, 2.4)),
        };
        b.UpdateLook();
        var cur = current;
        b.Click += () => ShowMenu(b, list.Select(i => new MenuEntry(i.label, () =>
        {
            cur = i.value;
            label.Text = i.label;
            onPick(i.value);
        }, Checked: Equals(i.value, cur))).ToList());
        return b;
    }

    /// <summary>无边框的菜单触发按钮（工具栏上的「…」「片段」等）</summary>
    public static Btn MenuButton(UIElement content, Func<List<MenuEntry>> entries, string? tip = null, double height = 26)
    {
        var b = new Btn { Height = height, CornerRadius = new CornerRadius(6), Padding = new Thickness(6, 0, 6, 0), HoverBrush = Th.Hover2, Child = content };
        b.UpdateLook();
        Tip(b, tip);
        b.Click += () => ShowMenu(b, entries());
        return b;
    }

    public static ContextMenu Menu(IEnumerable<MenuEntry> entries)
    {
        var m = new ContextMenu();
        foreach (var e in entries) m.Items.Add(MenuItemFor(e));
        return m;
    }

    private static object MenuItemFor(MenuEntry e)
    {
        if (e.IsSeparator) return new Separator();
        var mi = new MenuItem { Header = e.Title, IsEnabled = e.Enabled, IsChecked = e.Checked };
        if (e.Destructive) mi.Foreground = Th.Danger;
        if (e.Sub != null) foreach (var s in e.Sub) mi.Items.Add(MenuItemFor(s));
        else if (e.Action != null)
        {
            var act = e.Action;
            mi.Click += (_, _) => act();
        }
        return mi;
    }

    public static void ShowMenu(FrameworkElement anchor, List<MenuEntry> entries)
    {
        if (entries.Count == 0) return;
        var m = Menu(entries);
        m.PlacementTarget = anchor;
        m.Placement = PlacementMode.Bottom;
        m.VerticalOffset = 2;
        m.IsOpen = true;
    }

    /// <summary>右键菜单：每次打开时按当前状态生成</summary>
    public static void ContextMenuFor(FrameworkElement e, Func<List<MenuEntry>> entries)
    {
        e.ContextMenu = new ContextMenu();
        e.ContextMenuOpening += (_, args) =>
        {
            var list = entries();
            if (list.Count == 0) { args.Handled = true; return; }
            var m = Menu(list);
            e.ContextMenu = m;
        };
    }

    /// <summary>分段选择</summary>
    public static Border Segmented<T>(IEnumerable<(T value, string label)> items, T current, Action<T> onPick, double itemMinWidth = 0, double height = 28)
    {
        var row = new Flex { Horizontal = true, Spacing = 2 };
        foreach (var (value, label) in items)
        {
            var sel = Equals(value, current);
            var b = new Btn
            {
                Height = height - 6, CornerRadius = new CornerRadius(5), Padding = new Thickness(10, 0, 10, 0), MinWidth = itemMinWidth,
                Normal = sel ? Th.Card : Brushes.Transparent, HoverBrush = sel ? Th.Card : Th.Hover2,
                Child = Center(Text(label, 12.5, sel ? Th.Text : Th.Secondary, sel ? FontWeights.SemiBold : FontWeights.Normal)),
            };
            if (sel) b.Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 3, ShadowDepth = 0.5, Opacity = 0.12, Direction = 270 };
            b.UpdateLook();
            var v = value;
            b.Click += () => onPick(v);
            row.Children.Add(b);
        }
        return new Border
        {
            Background = Th.B(Th.P.IsDark ? Palettes.A(Colors.White, 0.06) : Palettes.A(Colors.Black, 0.05)), CornerRadius = new CornerRadius(7),
            Padding = new Thickness(3), Child = row, HorizontalAlignment = HorizontalAlignment.Left,
        };
    }

    public static FrameworkElement Center(UIElement e) { Region.Detach(e); return new Grid { Children = { e } }.Also(g => { if (e is FrameworkElement fe) { fe.HorizontalAlignment = HorizontalAlignment.Center; fe.VerticalAlignment = VerticalAlignment.Center; } }); }

    /// <summary>小 i 图标：说明文字只放在弹出层里，界面上不铺描述小字。</summary>
    public static FrameworkElement Info(string text)
    {
        var b = new Btn { Width = 18, Height = 18, CornerRadius = new CornerRadius(9), HoverBrush = Th.Hover2, Child = Icon("info", 13, Th.Tertiary) };
        b.UpdateLook();
        var popup = new Popup
        {
            PlacementTarget = b, Placement = PlacementMode.Bottom, StaysOpen = false, AllowsTransparency = true, PopupAnimation = PopupAnimation.Fade,
            Child = new Border
            {
                Margin = new Thickness(0, 4, 10, 10), Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(9),
                Padding = new Thickness(14, 12, 14, 12), Width = 300,
                Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 14, ShadowDepth = 3, Opacity = 0.2, Direction = 270 },
                Child = Text(text, 12.5, Th.Text, wrap: true),
            },
        };
        b.Click += () => popup.IsOpen = !popup.IsOpen;
        return b;
    }

    public static Border TagChip(string text, bool selected = false, Action? onClick = null)
    {
        var t = Text(text, 11, selected ? Th.AccentText : Th.Secondary, FontWeights.Medium);
        var bg = selected ? Th.Accent : Th.B(Th.P.IsDark ? Palettes.A(Colors.White, 0.08) : Palettes.A(Colors.Black, 0.05));
        if (onClick == null)
            return new Border { Child = t, Background = bg, CornerRadius = new CornerRadius(4), Padding = new Thickness(6, 1, 6, 2), BorderBrush = selected ? null : Th.Border, BorderThickness = new Thickness(selected ? 0 : 1) };
        var b = new Btn { Child = t, Normal = bg, HoverBrush = selected ? bg : Th.Hover2, CornerRadius = new CornerRadius(4), Padding = new Thickness(6, 1, 6, 2), BorderThickness = new Thickness(selected ? 0 : 1), NormalBorder = Th.Border };
        b.UpdateLook();
        b.Click += onClick;
        return b;
    }

    public static Brush DotColor(DotState s) => s switch
    {
        DotState.Online => Th.Success,
        DotState.Lan => Th.Info,
        DotState.Busy => Th.Warning,
        _ => Th.B(Th.P.TertiaryText, 0.7),
    };

    public static DotState DotOf(DeviceInfo d) => d.Online ? DotState.Online : d.LanSeen ? DotState.Lan : DotState.Offline;

    public static FrameworkElement StatusDot(DotState s, double size = 8, Brush? ring = null)
        => new Ellipse { Width = size + 3, Height = size + 3, Fill = DotColor(s), Stroke = ring ?? Th.List, StrokeThickness = 1.5 };

    public static string PlatformIcon(Platform p) => p switch
    {
        Platform.Macos => "apple",
        Platform.Ios => "phone",
        Platform.Windows => "windows",
        Platform.Android => "android",
        Platform.Harmonyos => "hexagon",
        _ => "linux",
    };

    public static FrameworkElement PlatformGlyph(Platform p, double size = 14, Brush? brush = null) => Icon(PlatformIcon(p), size, brush ?? Th.Secondary, 1.8);

    /// <summary>协议徽标（SSH / RDP 方块）</summary>
    public static Border ProtocolBadge(string text, Color color, double size = 26, double font = 8.5)
        => new()
        {
            Width = size, Height = size, CornerRadius = new CornerRadius(size * 0.23), Background = Th.B(color, 0.13),
            Child = Center(new TextBlock { Text = text, FontSize = font, FontWeight = FontWeights.Bold, Foreground = Th.B(color), FontFamily = Th.UiFont }),
        };

    /// <summary>应用标志：深色圆角方块 + 黄绿渐变的星轨弧线（与 Mac 端同样的画法）</summary>
    public static FrameworkElement AppMark(double size)
    {
        var g = new Grid { Width = size, Height = size };
        g.Children.Add(new Border
        {
            CornerRadius = new CornerRadius(size * 0.24),
            Background = new LinearGradientBrush(Palettes.Hex("#1B1F2A"), Palettes.Hex("#0E1016"), 90),
        });
        var pad = size * 0.2;
        var inner = size - pad * 2;
        var c = new Point(pad + inner / 2, pad + inner / 2);
        var geo = new PathGeometry();
        void Arc(double r, double from, double to)
        {
            Point P(double deg) => new(c.X + r * Math.Cos(deg * Math.PI / 180), c.Y + r * Math.Sin(deg * Math.PI / 180));
            var span = ((to - from) % 360 + 360) % 360;
            var f = new PathFigure { StartPoint = P(from), IsClosed = false };
            f.Segments.Add(new ArcSegment(P(to), new Size(r, r), 0, span > 180, SweepDirection.Clockwise, true));
            geo.Figures.Add(f);
        }
        Arc(inner * 0.5, 200, 340);
        Arc(inner * 0.5, 20, 160);
        Arc(inner * 0.3, 290, 70);
        g.Children.Add(new Path
        {
            Data = geo, StrokeThickness = size * 0.055, StrokeStartLineCap = PenLineCap.Round, StrokeEndLineCap = PenLineCap.Round,
            Stroke = new LinearGradientBrush(Palettes.Hex("#F5D90A"), Palettes.Hex("#7BD35A"), new Point(0, 0), new Point(1, 1)),
        });
        var dot = size * 0.13;
        g.Children.Add(new Ellipse { Width = dot, Height = dot, Fill = Th.Hex("#F5D90A"), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center });
        return g;
    }

    public static Border Card(UIElement child, double padding = 16, Brush? border = null)
        => new()
        {
            Child = Detached(child), Padding = new Thickness(padding), Background = Th.Card, BorderBrush = border ?? Th.Border,
            BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10), SnapsToDevicePixels = true,
        };

    public static TextBlock SectionLabel(string text)
    {
        var t = Text(text.ToUpperInvariant(), 11, Th.Tertiary, FontWeights.SemiBold);
        return t;
    }

    public static ScrollViewer Scroll(UIElement child, bool horizontal = false)
    {
        Region.Detach(child);
        return new ScrollViewer
        {
            Content = child, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Focusable = false,
            HorizontalScrollBarVisibility = horizontal ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled,
            PanningMode = PanningMode.VerticalFirst,
        };
    }

    /// <summary>页面滚动容器：内容最宽 maxWidth，居中，四周留白 24</summary>
    public static ScrollViewer PageScroll(UIElement content, double maxWidth = 860)
    {
        Region.Detach(content);
        var b = new Border { Child = content, MaxWidth = maxWidth + 48, Padding = new Thickness(24) };
        return Scroll(b);
    }

    public static FrameworkElement Spinner(double size = 16, Brush? brush = null)
    {
        var r = size / 2 - 1.5;
        var geo = new PathGeometry();
        var f = new PathFigure { StartPoint = new Point(size / 2, 1.5) };
        f.Segments.Add(new ArcSegment(new Point(size / 2 + r, size / 2), new Size(r, r), 0, false, SweepDirection.Clockwise, true));
        geo.Figures.Add(f);
        var track = new Ellipse { Width = size - 3, Height = size - 3, Stroke = Th.B((brush as SolidColorBrush)?.Color ?? Th.P.Accent, 0.2), StrokeThickness = 2, Margin = new Thickness(1.5) };
        var arc = new Path { Data = geo, Stroke = brush ?? Th.Accent, StrokeThickness = 2, StrokeStartLineCap = PenLineCap.Round, StrokeEndLineCap = PenLineCap.Round };
        var rot = new RotateTransform(0, size / 2, size / 2);
        arc.RenderTransform = rot;
        rot.BeginAnimation(RotateTransform.AngleProperty, new DoubleAnimation(0, 360, TimeSpan.FromSeconds(0.9)) { RepeatBehavior = RepeatBehavior.Forever });
        return new Grid { Width = size, Height = size, Children = { track, arc }, IsHitTestVisible = false };
    }

    /// <summary>空状态：图标 + 一行标题（可带一个主按钮），不铺说明文字</summary>
    public static FrameworkElement EmptyState(string icon, string title, (string, Action)? action = null)
    {
        var col = Col(12, Center(Icon(icon, 34, Th.Tertiary, 1.5)), Center(Text(title, 13.5, Th.Secondary)),
            action is { } a ? Center(PrimaryButton(a.Item1, a.Item2)) : null);
        col.HorizontalAlignment = HorizontalAlignment.Center;
        col.VerticalAlignment = VerticalAlignment.Center;
        return new Grid { Children = { col } };
    }

    public static Border Badge(string text, Color color, string? icon = null, double size = 11)
    {
        var t = Text(text, size, Th.B(color), FontWeights.SemiBold);
        return new Border
        {
            Background = Th.B(color, 0.13), CornerRadius = new CornerRadius(10), Padding = new Thickness(7, 2, 7, 2),
            Child = icon == null ? t : Row(4, Icon(icon, size - 0.5, Th.B(color), 2.2), t),
        };
    }

    /// <summary>进度条（胶囊）</summary>
    public static FrameworkElement Progress(double fraction, Brush color, double height = 4)
    {
        fraction = Math.Max(0, Math.Min(1, fraction));
        var g = new Grid { Height = height };
        g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(Math.Max(fraction, 0.0001), GridUnitType.Star) });
        g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(Math.Max(1 - fraction, 0.0001), GridUnitType.Star) });
        var bg = new Border { CornerRadius = new CornerRadius(height / 2), Background = Th.B(Th.P.Border, 0.8) };
        Grid.SetColumnSpan(bg, 2);
        g.Children.Add(bg);
        if (fraction > 0) g.Children.Add(new Border { CornerRadius = new CornerRadius(height / 2), Background = color });
        return g;
    }

    /// <summary>头像圆：显示名的第一个字</summary>
    public static FrameworkElement Avatar(string name, double size)
    {
        var letter = string.IsNullOrEmpty(name) ? "?" : char.ToUpperInvariant(name[0]).ToString();
        return new Border
        {
            Width = size, Height = size, CornerRadius = new CornerRadius(size / 2), Background = Th.B(Th.P.Accent, 0.2),
            Child = Center(Text(letter, size * 0.45, Th.Accent, FontWeights.SemiBold)),
        };
    }

    public static T Also<T>(this T t, Action<T> a) { a(t); return t; }
}
