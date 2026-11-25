using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using LemonGalaxy.Ipc;

namespace LemonGalaxy.UI;

/// <summary>被控端界面的版式小件（建在共用的 K 上）：页面标题、分组卡片、设置行、带 UAC 盾牌的按钮、异步开关。</summary>
internal static class W
{
    /// <summary>页面标题：大标题 + i 说明 + 右侧按钮</summary>
    public static FrameworkElement Title(string text, string? info = null, params UIElement?[] trailing)
        => K.Row(8, new UIElement?[] { K.Semi(text, 20), info == null ? null : K.Info(info), K.Spacer() }.Concat(trailing).ToArray());

    /// <summary>分组标题（小号大写）+ 可选 i</summary>
    public static FrameworkElement Section(string text, string? info = null, params UIElement?[] trailing)
        => K.Margin(K.Row(6, new UIElement?[] { K.SectionLabel(text), info == null ? null : K.Info(info), K.Spacer() }.Concat(trailing).ToArray()), 2, 6, 0, 0);

    /// <summary>一张卡片里若干行，行间分隔线</summary>
    public static Border Rows(IEnumerable<UIElement?> rows)
    {
        var col = new Flex { Horizontal = false };
        var first = true;
        foreach (var r in rows)
        {
            if (r == null) continue;
            if (!first) col.Children.Add(K.Margin(K.HLine(), 14, 0, 0, 0));
            first = false;
            Region.Detach(r);
            col.Children.Add(r);
        }
        return K.Card(col, padding: 0);
    }

    public static Border Rows(params UIElement?[] rows) => Rows((IEnumerable<UIElement?>)rows);

    /// <summary>设置行：标题（可带副标题、i 说明）在左，控件在右</summary>
    public static FrameworkElement Row(string title, UIElement? right, string? info = null, string? subtitle = null, UIElement? icon = null, Brush? subtitleColor = null)
    {
        var text = K.Col(2, K.Row(6, K.Text(title, 13), info == null ? null : K.Info(info)), string.IsNullOrEmpty(subtitle) ? null : K.Text(subtitle!, 12, subtitleColor ?? Th.Secondary, wrap: true));
        return K.Pad(K.Row(12, icon, K.Grow(text), right), 14, 11, 14, 11);
    }

    /// <summary>右侧的只读值（可选中复制）</summary>
    public static FrameworkElement Value(string text, bool mono = false)
    {
        var t = K.SelectableText(text, 12.5, Th.Secondary, mono: mono, wrap: false);
        t.HorizontalContentAlignment = HorizontalAlignment.Right;
        t.MaxWidth = 380;
        return t;
    }

    public static Btn CopyButton(Func<string> text)
        => K.IconButton("copy", L.T("common.copy"), () =>
        {
            SysClipboard.SetText(text());
            AppModel.Current?.ShowToast(L.T("common.copied"));
        }, size: 13, w: 26, h: 24);

    /// <summary>主按钮；要经 UAC 授权时左边画盾牌（Windows 的惯例）</summary>
    public static Btn Primary(string title, Action onClick, bool shield = false, bool destructive = false, bool enabled = true)
        => K.PrimaryButton(title, onClick, destructive, shield ? "shield" : null, enabled);

    public static Btn Secondary(string title, Action onClick, bool shield = false, bool destructive = false, bool enabled = true)
        => K.SecondaryButton(title, onClick, destructive, shield ? "shield" : null, enabled);

    /// <summary>
    /// 开关：点了先变过去并禁用，动作做完（返回成功）才算数，失败就拨回去。
    /// shield 为真时开关旁边画一个小盾牌（改它要管理员授权）。
    /// </summary>
    public static FrameworkElement Switch(bool on, Func<bool, Task<bool>> act, bool enabled = true, bool shield = false)
    {
        var t = new Toggle(on) { IsEnabled = enabled };
        t.Changed += v => Ui.Run(async () =>
        {
            t.IsEnabled = false;
            var ok = false;
            try { ok = await act(v); }
            finally
            {
                if (!ok) t.IsOn = !v;
                t.IsEnabled = enabled;
            }
        });
        if (!shield || !enabled) return t;
        return K.Row(6, K.Tip(K.Icon("shield", 12, Th.Tertiary), L.T("uac.needed")), t);
    }

    /// <summary>发请求的开关：成功返回 true</summary>
    public static FrameworkElement ReqSwitch(AppModel app, bool on, Func<bool, Dictionary<string, object?>> req, bool enabled = true)
        => Switch(on, async v => await app.Do(req(v)) != null, enabled, app.Rights.NeedsElevation);

    /// <summary>空状态 / 没有权限时的整页提示：图标 + 标题 + i</summary>
    public static FrameworkElement Placeholder(string icon, string title, string? info = null, UIElement? action = null)
    {
        var col = K.Col(12, K.Center(K.Icon(icon, 36, Th.Tertiary, 1.5)), K.Center(K.Row(6, K.Text(title, 14, Th.Secondary), info == null ? null : K.Info(info))), action == null ? null : K.Center(action));
        col.HorizontalAlignment = HorizontalAlignment.Center;
        col.VerticalAlignment = VerticalAlignment.Center;
        col.Margin = new Thickness(0, 60, 0, 0);
        return col;
    }

    /// <summary>页面内的提示条：图标 + 文字 + 按钮</summary>
    public static Border Banner(string icon, Color color, string text, UIElement? action = null)
        => new()
        {
            Background = Th.B(color, 0.1), BorderBrush = Th.B(color, 0.35), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(9),
            Padding = new Thickness(12, 10, 12, 10),
            Child = K.Row(10, K.Icon(icon, 15, Th.B(color)), K.Grow(K.Text(text, 12.5, Th.Text, wrap: true)), action),
        };

    public static FrameworkElement Busy(string text) => K.Row(8, K.Spinner(14), K.Text(text, 12.5, Th.Secondary));
}

/// <summary>窗口内弹层卡片</summary>
internal static class Dialogs
{
    public static FrameworkElement Header(string title, string? subtitle = null, string? icon = null, string? info = null, Action? close = null)
    {
        var text = K.Col(2, K.Row(6, K.Semi(title, 15), info == null ? null : K.Info(info)), subtitle == null ? null : K.Text(subtitle, 12, Th.Secondary));
        return K.Row(10, icon == null ? null : K.Icon(icon, 18, Th.Accent), K.Grow(text), close == null ? null : K.IconButton("close", L.T("common.close"), close, size: 12));
    }

    public static FrameworkElement Buttons(params UIElement?[] buttons) => K.Row(8, new UIElement?[] { K.Spacer() }.Concat(buttons).ToArray());

    public static FrameworkElement ConfirmCard(string title, string body, string ok, bool destructive, Action yes, Action no)
        => Card(K.Col(14,
            Header(title, icon: destructive ? "warning" : "info"),
            K.Text(body, 12.5, Th.Secondary, wrap: true),
            Buttons(K.SecondaryButton(L.T("common.cancel"), no), K.PrimaryButton(ok, yes, destructive))), 420);

    /// <summary>弹层卡片的外框：圆角、阴影、固定宽度</summary>
    public static Border Card(UIElement content, double width)
        => new()
        {
            Width = width, Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(12),
            Padding = new Thickness(20), Child = content, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
            Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 24, ShadowDepth = 6, Opacity = Th.P.IsDark ? 0.45 : 0.18, Direction = 270 },
        };
}

internal static class Fmt
{
    /// <summary>毫秒时间戳 → 本地时间（今天只写时分）</summary>
    public static string Time(long ms, bool seconds = false)
    {
        if (ms <= 0) return "—";
        var t = DateTimeOffset.FromUnixTimeMilliseconds(ms).LocalDateTime;
        var f = seconds ? "HH:mm:ss" : "HH:mm";
        return t.Date == DateTime.Today ? t.ToString(f, CultureInfo.InvariantCulture) : t.ToString("yyyy-MM-dd " + f, CultureInfo.InvariantCulture);
    }

    /// <summary>设备 ID 每 4 位一组</summary>
    public static string GroupId(string id)
    {
        var parts = new List<string>();
        for (var i = 0; i < id.Length; i += 4) parts.Add(id.Substring(i, Math.Min(4, id.Length - i)));
        return string.Join(" ", parts);
    }

    public static string ShortDevice(string d) => d.Length > 8 ? d.Substring(0, 8) : d;

    public static string Who(string user) => string.IsNullOrEmpty(user) ? L.T("session.anonymous") : user;

    public static string PathText(string path) => path switch
    {
        "lan" => L.T("path.lan"),
        "direct" => L.T("path.direct"),
        "relay_udp" or "relay_tcp" => L.T("path.relay"),
        "connecting" => L.T("path.connecting"),
        _ => path,
    };

    public static string AiMode(string m) => L.T("ai." + m);

    public static readonly string[] AiModes = { "off", "read_only", "confirm_all", "confirm_dangerous", "full_trust" };

    public static string Kind(string k)
    {
        var key = "kind." + k;
        var t = L.T(key);
        return t == key ? k : t;
    }

    public static string Size(string id)
    {
        var at = id.IndexOf('@');
        var wh = (at >= 0 ? id.Substring(0, at) : id).Replace("x", " × ");
        return at >= 0 ? $"{wh} · {id.Substring(at + 1)} Hz" : wh;
    }
}
