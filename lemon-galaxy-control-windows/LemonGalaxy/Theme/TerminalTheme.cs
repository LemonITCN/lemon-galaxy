using System.IO;
using System.Web.Script.Serialization;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>
/// 终端配色主题：Resources\TerminalThemes\*.json，一个主题一个文件，加新主题只加文件（与 Mac 端同一格式）。
/// 字段：id、name、dark、order（排序）、foreground、background、cursor、selection、ansi（16 个颜色，0–7 普通，8–15 高亮）。
/// </summary>
internal sealed class TerminalTheme
{
    public string Id = "galaxy-dark";
    public string Name = "Galaxy Dark";
    public bool Dark = true;
    public int Order;
    public Color Foreground, Background, Cursor, SelectionColor;
    public Color[] Ansi = new Color[16];

    public static readonly TerminalTheme Fallback = Make("galaxy-dark", "Galaxy Dark", true, 0, "#E3E6EB", "#171A21", "#4C82F7", "#2C3A55",
        new[] { "#1F232B", "#F2555A", "#34C77B", "#F0B43A", "#4C82F7", "#B07CF2", "#2EC4D6", "#D5D9E0",
                "#5C6575", "#FF7A7E", "#5BE09A", "#FFD066", "#79A2FF", "#CDA0FF", "#5EE0EE", "#FFFFFF" });

    private static TerminalTheme Make(string id, string name, bool dark, int order, string fg, string bg, string cursor, string sel, string[] ansi) => new()
    {
        Id = id, Name = name, Dark = dark, Order = order,
        Foreground = Palettes.Hex(fg), Background = Palettes.Hex(bg), Cursor = Palettes.Hex(cursor), SelectionColor = Palettes.Hex(sel),
        Ansi = ansi.Select(Palettes.Hex).ToArray(),
    };

    private static List<TerminalTheme>? all;

    public static List<TerminalTheme> All
    {
        get
        {
            if (all != null) return all;
            var list = new List<TerminalTheme>();
            foreach (var f in AppInfo.ResourceFiles("TerminalThemes", "json"))
            {
                try
                {
                    if (new JavaScriptSerializer().DeserializeObject(File.ReadAllText(f)) is not Dictionary<string, object> o) continue;
                    if (o.GetValueOrDefault("id") is not string id || o.GetValueOrDefault("name") is not string name) continue;
                    if (o.GetValueOrDefault("foreground") is not string fg || o.GetValueOrDefault("background") is not string bg) continue;
                    if (o.GetValueOrDefault("ansi") is not object[] ansi || ansi.Length != 16) continue;
                    list.Add(Make(id, name, o.GetValueOrDefault("dark") as bool? ?? true, o.GetValueOrDefault("order") is int ord ? ord : 99,
                        fg, bg, o.GetValueOrDefault("cursor") as string ?? fg, o.GetValueOrDefault("selection") as string ?? "#44475A",
                        ansi.Select(a => a as string ?? "#808080").ToArray()));
                }
                catch (Exception e) { Log.Error("终端主题 " + f + " 读取失败：" + e.Message); }
            }
            all = list.Count == 0 ? new List<TerminalTheme> { Fallback }
                : list.OrderBy(t => t.Order).ThenBy(t => t.Name, StringComparer.OrdinalIgnoreCase).ToList();
            return all;
        }
    }

    public static TerminalTheme Named(string? id) =>
        All.FirstOrDefault(t => t.Id == id) ?? All.FirstOrDefault(t => t.Id == "galaxy-dark") ?? All[0];

    /// <summary>xterm 256 色调色板：0–15 取主题，16–231 为 6×6×6 立方，232–255 为灰阶</summary>
    public Color PaletteColor(int i)
    {
        if (i < 16) return Ansi[Math.Max(0, i)];
        if (i < 232)
        {
            var n = i - 16;
            byte C(int v) => (byte)(v == 0 ? 0 : 55 + v * 40);
            return Color.FromRgb(C(n / 36), C(n / 6 % 6), C(n % 6));
        }
        var g = (byte)Math.Min(255, 8 + (i - 232) * 10);
        return Color.FromRgb(g, g, g);
    }

    /// <summary>TermSpan 的颜色编码：0 默认；0x0100_00nn 调色板；0x02RRGGBB 真彩色</summary>
    public Color Resolve(uint code, bool foreground, bool bold)
    {
        if (code == 0) return foreground ? Foreground : Background;
        if ((code & 0xFF00_0000) == 0x0100_0000)
        {
            var idx = (int)(code & 0xFF);
            if (foreground && bold && idx < 8) idx += 8;   // 粗体用高亮色，和多数终端一致
            return PaletteColor(idx);
        }
        return Color.FromRgb((byte)((code >> 16) & 0xFF), (byte)((code >> 8) & 0xFF), (byte)(code & 0xFF));
    }
}

/// <summary>终端字体：本机已安装的等宽字体，优先 Cascadia Mono、Consolas。</summary>
internal static class TerminalFonts
{
    private static readonly string[] Preferred = { "Cascadia Mono", "Cascadia Code", "Consolas", "JetBrains Mono", "Fira Code", "Source Code Pro", "Lucida Console", "Courier New" };
    private static List<string>? families;

    public static List<string> Families
    {
        get
        {
            if (families != null) return families;
            var installed = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            try
            {
                foreach (var f in Fonts.SystemFontFamilies)
                    installed.Add(f.Source);
            }
            catch { }
            var list = Preferred.Where(installed.Contains).ToList();
            foreach (var f in installed.OrderBy(x => x, StringComparer.OrdinalIgnoreCase))
            {
                if (list.Contains(f)) continue;
                var low = f.ToLowerInvariant();
                if (low.Contains("mono") || low.Contains("code") || low.Contains("consol") || low.Contains("courier")) list.Add(f);
            }
            if (list.Count == 0) list.Add("Consolas");
            families = list;
            return list;
        }
    }

    public static string Default => Families.FirstOrDefault() ?? "Consolas";

    /// <summary>设置里的字体 + 中文回退</summary>
    public static FontFamily Family(string setting)
    {
        var name = string.IsNullOrWhiteSpace(setting) ? Default : setting;
        return new FontFamily(name + ", Consolas, Microsoft YaHei UI, Microsoft YaHei, SimSun");
    }
}
