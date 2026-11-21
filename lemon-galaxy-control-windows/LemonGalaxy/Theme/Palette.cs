using System.Windows;
using System.Windows.Media;
using Microsoft.Win32;

namespace LemonGalaxy.UI;

/// <summary>界面配色（与 Mac 端同一套）。每套配色有浅色、深色两个变体；界面主题（跟随系统 / 浅色 / 深色）决定用哪一个。</summary>
internal sealed class Palette
{
    public string Id = "galaxy";
    public bool IsDark;
    public Color Accent, AccentText, Sidebar, SidebarText, SidebarSelection, List, Content, Card, Elevated, Border, Separator,
        Text, SecondaryText, TertiaryText, Hover, Selection, Success, Warning, Danger, Info, TabBar, TabActive;
}

internal sealed record PaletteFamily(string Id, string NameKey, string AccentLight, string AccentDark, string Swatch);

internal static class Palettes
{
    public static readonly PaletteFamily[] Families =
    {
        new("galaxy", "palette.galaxy", "#2F6BEE", "#4C82F7", "#3B76F2"),
        new("teal", "palette.teal", "#0E9F8E", "#22C3AE", "#14B8A6"),
        new("violet", "palette.violet", "#6D4AE8", "#8B70FF", "#7C5CFF"),
        new("lemon", "palette.lemon", "#B58300", "#F2C230", "#E8B400"),
        new("rose", "palette.rose", "#D63C55", "#F2607A", "#E5484D"),
        new("graphite", "palette.graphite", "#4B5563", "#A1A8B3", "#6B7280"),
    };

    public static Color Hex(string hex)
    {
        var s = hex.TrimStart('#');
        uint v = Convert.ToUInt32(s, 16);
        if (s.Length == 8) return Color.FromArgb((byte)(v & 0xff), (byte)(v >> 24), (byte)((v >> 16) & 0xff), (byte)((v >> 8) & 0xff));
        return Color.FromRgb((byte)(v >> 16), (byte)((v >> 8) & 0xff), (byte)(v & 0xff));
    }

    public static Color A(Color c, double opacity) => Color.FromArgb((byte)Math.Round(255 * Math.Max(0, Math.Min(1, opacity)) * c.A / 255.0), c.R, c.G, c.B);

    public static Palette Make(string id, bool dark)
    {
        var f = Families.FirstOrDefault(x => x.Id == id) ?? Families[0];
        var accent = Hex(dark ? f.AccentDark : f.AccentLight);
        // 柠檬、石墨在深色下强调色很亮，上面的字用深色
        var accentText = dark && (f.Id == "lemon" || f.Id == "graphite") ? Hex("#15171C") : Colors.White;
        if (dark)
            return new Palette
            {
                Id = f.Id, IsDark = true, Accent = accent, AccentText = accentText,
                Sidebar = Hex("#14161B"), SidebarText = Hex("#C9CED8"), SidebarSelection = A(accent, 0.22),
                List = Hex("#191C22"), Content = Hex("#1E2128"), Card = Hex("#252932"), Elevated = Hex("#2B3039"),
                Border = Hex("#343A45"), Separator = Hex("#2A2F38"),
                Text = Hex("#E8EAEE"), SecondaryText = Hex("#A3ABB8"), TertiaryText = Hex("#737C8A"),
                Hover = A(Colors.White, 0.05), Selection = A(accent, 0.24),
                Success = Hex("#34C77B"), Warning = Hex("#F0A43A"), Danger = Hex("#F2555A"), Info = Hex("#58A6FF"),
                TabBar = Hex("#14161B"), TabActive = Hex("#1E2128"),
            };
        return new Palette
        {
            Id = f.Id, IsDark = false, Accent = accent, AccentText = accentText,
            Sidebar = Hex("#EEF0F4"), SidebarText = Hex("#3A4250"), SidebarSelection = A(accent, 0.14),
            List = Hex("#F6F7F9"), Content = Hex("#FFFFFF"), Card = Hex("#FFFFFF"), Elevated = Hex("#FFFFFF"),
            Border = Hex("#DDE1E7"), Separator = Hex("#E6E9EE"),
            Text = Hex("#1A1E26"), SecondaryText = Hex("#5B6576"), TertiaryText = Hex("#8A93A2"),
            Hover = A(Colors.Black, 0.04), Selection = A(accent, 0.13),
            Success = Hex("#1E9E5A"), Warning = Hex("#C77A0A"), Danger = Hex("#D93036"), Info = Hex("#1F6FEB"),
            TabBar = Hex("#E7EAEF"), TabActive = Hex("#FFFFFF"),
        };
    }

    /// <summary>系统是不是深色模式（Windows 10 1809 起的「应用模式」；Win7 / 8 视为浅色）</summary>
    public static bool SystemIsDark()
    {
        try
        {
            using var k = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            if (k?.GetValue("AppsUseLightTheme") is int v) return v == 0;
        }
        catch { }
        return false;
    }
}

/// <summary>当前配色的画刷。界面构建时读取；外观变化时整个界面重建。</summary>
internal static class Th
{
    public static Palette P = Palettes.Make("galaxy", false);
    private static readonly Dictionary<Color, SolidColorBrush> Cache = new();

    public static SolidColorBrush B(Color c)
    {
        if (Cache.TryGetValue(c, out var b)) return b;
        b = new SolidColorBrush(c);
        b.Freeze();
        Cache[c] = b;
        return b;
    }

    public static SolidColorBrush B(Color c, double opacity) => B(Palettes.A(c, opacity));
    public static SolidColorBrush Hex(string hex) => B(Palettes.Hex(hex));

    public static Brush Accent => B(P.Accent);
    public static Brush AccentText => B(P.AccentText);
    public static Brush Sidebar => B(P.Sidebar);
    public static Brush SidebarText => B(P.SidebarText);
    public static Brush SidebarSelection => B(P.SidebarSelection);
    public static Brush List => B(P.List);
    public static Brush Content => B(P.Content);
    public static Brush Card => B(P.Card);
    public static Brush Elevated => B(P.Elevated);
    public static Brush Border => B(P.Border);
    public static Brush Separator => B(P.Separator);
    public static Brush Text => B(P.Text);
    public static Brush Secondary => B(P.SecondaryText);
    public static Brush Tertiary => B(P.TertiaryText);
    public static Brush Hover => B(P.Hover);
    public static Brush Hover2 => B(P.IsDark ? Palettes.A(Colors.White, 0.09) : Palettes.A(Colors.Black, 0.07));
    public static Brush Selection => B(P.Selection);
    public static Brush Success => B(P.Success);
    public static Brush Warning => B(P.Warning);
    public static Brush Danger => B(P.Danger);
    public static Brush Info => B(P.Info);
    public static Brush TabBar => B(P.TabBar);
    public static Brush TabActive => B(P.TabActive);
    public static Brush CodeBg => B(P.IsDark ? Palettes.A(Colors.Black, 0.25) : Palettes.A(Colors.Black, 0.04));
    public static Brush Scrim => B(P.IsDark ? Palettes.A(Colors.Black, 0.45) : Palettes.A(Colors.Black, 0.25));

    public static readonly FontFamily UiFont = new("Segoe UI, Microsoft YaHei UI, Microsoft YaHei, SimSun");
    public static readonly FontFamily MonoFont = new("Cascadia Mono, Consolas, Microsoft YaHei UI, Microsoft YaHei, SimSun");

    /// <summary>根据设置算出当前是浅色还是深色，更新画刷与资源字典里给模板用的颜色</summary>
    public static void Apply(AppSettings s)
    {
        var dark = s.Appearance switch { "dark" => true, "light" => false, _ => Palettes.SystemIsDark() };
        P = Palettes.Make(s.Palette, dark);
        Styles.ApplyColors();
    }
}
