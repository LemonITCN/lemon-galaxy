using System.Globalization;
using System.IO;
using System.Web.Script.Serialization;

namespace LemonGalaxy;

/// <summary>
/// 多语言底座：Resources\Locales\&lt;代码&gt;.json，一种语言一个文件，新增语言只加文件（与 Mac 端同一格式，可直接复用）。
/// 格式：{ "_meta": { "code": "zh-Hans", "name": "简体中文", "coreLocale": "zh-CN" }, "strings": { "键": "文字" } }
/// 文字里的 {0} {1} … 按顺序替换为参数。缺的键回落到英文，再回落到键本身（selftest 会列出缺失的键）。
/// </summary>
internal sealed class LanguagePack
{
    public string Code = "en";
    public string Name = "English";
    public string CoreLocale = "en";
    public Dictionary<string, string> Strings = new();

    public static LanguagePack? Load(string path)
    {
        try
        {
            var obj = new JavaScriptSerializer { MaxJsonLength = int.MaxValue }.DeserializeObject(File.ReadAllText(path)) as Dictionary<string, object>;
            if (obj == null || obj.GetValueOrDefault("_meta") is not Dictionary<string, object> meta) return null;
            if (meta.GetValueOrDefault("code") is not string code) return null;
            var p = new LanguagePack
            {
                Code = code,
                Name = meta.GetValueOrDefault("name") as string ?? code,
                CoreLocale = meta.GetValueOrDefault("coreLocale") as string ?? code,
            };
            if (obj.GetValueOrDefault("strings") is Dictionary<string, object> strings)
                foreach (var kv in strings)
                    if (kv.Value is string s) p.Strings[kv.Key] = s;
            return p;
        }
        catch { return null; }
    }
}

internal static class L
{
    public static List<LanguagePack> Packs = new();
    public static LanguagePack Pack = new();
    private static LanguagePack? fallback;
    private static readonly HashSet<string> Missing = new();

    public static void LoadPacks()
    {
        Packs = AppInfo.ResourceFiles("Locales", "json").Select(LanguagePack.Load).Where(p => p != null).Select(p => p!).ToList();
        fallback = Packs.FirstOrDefault(p => p.Code == "en");
        Pack = Packs.FirstOrDefault() ?? new LanguagePack();
    }

    public static bool IsChinese => Pack.Code.StartsWith("zh", StringComparison.Ordinal);

    /// <summary>"system" 跟随系统：系统界面语言以 zh 开头用简体中文，否则看有没有同名语言包，没有用英文。</summary>
    public static string Resolve(string setting)
    {
        if (setting != "system" && Packs.Any(p => p.Code == setting)) return setting;
        var ui = CultureInfo.CurrentUICulture.Name;
        if (ui.StartsWith("zh", StringComparison.OrdinalIgnoreCase)) return "zh-Hans";
        var hit = Packs.FirstOrDefault(p => ui.StartsWith(p.Code, StringComparison.OrdinalIgnoreCase));
        return hit?.Code ?? "en";
    }

    public static void Apply(string setting)
    {
        var code = Resolve(setting);
        Pack = Packs.FirstOrDefault(p => p.Code == code) ?? fallback ?? Pack;
    }

    public static string T(string key, params object[] args)
    {
        if (!Pack.Strings.TryGetValue(key, out var s))
        {
            if (fallback == null || !fallback.Strings.TryGetValue(key, out s))
            {
                lock (Missing) Missing.Add(key);
                s = key;
            }
        }
        for (var i = 0; i < args.Length; i++) s = s.Replace("{" + i + "}", Convert.ToString(args[i], CultureInfo.InvariantCulture));
        return s;
    }

    public static string[] MissingKeys { get { lock (Missing) return Missing.OrderBy(k => k, StringComparer.Ordinal).ToArray(); } }
}

internal static class DictExt
{
    public static TV? GetValueOrDefault<TK, TV>(this Dictionary<TK, TV> d, TK key) where TK : notnull
        => d.TryGetValue(key, out var v) ? v : default;
}
