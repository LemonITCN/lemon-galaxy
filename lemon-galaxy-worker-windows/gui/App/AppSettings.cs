using System.IO;
using System.Text;
using System.Web.Script.Serialization;
using LemonGalaxy.UI;

namespace LemonGalaxy;

/// <summary>界面自己的设置（每个 Windows 用户一份，%LOCALAPPDATA%\LemonGalaxy\worker-ui\settings.json）。被控服务的设置在服务那边。</summary>
internal sealed class AppSettings : Observable
{
    public static readonly AppSettings Shared = new();

    /// <summary>外观：system / light / dark</summary>
    public string Appearance = "system";
    /// <summary>配色（与主控端同一套）</summary>
    public string Palette = "teal";
    /// <summary>语言：system 或语言包代码</summary>
    public string Language = "system";
    /// <summary>登录后在托盘显示图标（安装时写了「所有用户登录后运行」；这里关掉就只对自己不显示）</summary>
    public bool TrayAtLogin = true;
    /// <summary>有人连入时发系统通知</summary>
    public bool Notifications = true;
    /// <summary>第一次关窗口时提示过「还在托盘里」</summary>
    public bool ToldAboutTray;
    public double WindowWidth = 980, WindowHeight = 680;

    /// <summary>外观或语言变了：整个界面重建</summary>
    public readonly Observable LookChanged = new();

    private string? path;

    public void Load(string? file = null)
    {
        path = file ?? IOPath.Combine(AppInfo.DataDir, "settings.json");
        try
        {
            if (!File.Exists(path)) return;
            if (new JavaScriptSerializer().DeserializeObject(File.ReadAllText(path, Encoding.UTF8)) is not Dictionary<string, object> d) return;
            string S(string k, string def) => d.TryGetValue(k, out var v) && v is string s ? s : def;
            bool B(string k, bool def) => d.TryGetValue(k, out var v) && v is bool b ? b : def;
            double N(string k, double def) => d.TryGetValue(k, out var v) ? v switch { int i => i, decimal m => (double)m, double x => x, _ => def } : def;
            Appearance = S("appearance", Appearance);
            Palette = S("palette", Palette);
            Language = S("language", Language);
            TrayAtLogin = B("trayAtLogin", TrayAtLogin);
            Notifications = B("notifications", Notifications);
            ToldAboutTray = B("toldAboutTray", ToldAboutTray);
            WindowWidth = N("windowWidth", WindowWidth);
            WindowHeight = N("windowHeight", WindowHeight);
        }
        catch (Exception e) { Log.Error("读设置失败：" + e.Message); }
    }

    public void Save()
    {
        if (path == null) return;
        try
        {
            Directory.CreateDirectory(IOPath.GetDirectoryName(path)!);
            var d = new Dictionary<string, object>
            {
                ["appearance"] = Appearance, ["palette"] = Palette, ["language"] = Language, ["trayAtLogin"] = TrayAtLogin,
                ["notifications"] = Notifications, ["toldAboutTray"] = ToldAboutTray, ["windowWidth"] = WindowWidth, ["windowHeight"] = WindowHeight,
            };
            File.WriteAllText(path, new JavaScriptSerializer().Serialize(d), new UTF8Encoding(false));
        }
        catch (Exception e) { Log.Error("保存设置失败：" + e.Message); }
    }

    public void SetLook(string? appearance = null, string? palette = null, string? language = null)
    {
        if (appearance != null) Appearance = appearance;
        if (palette != null) Palette = palette;
        if (language != null) Language = language;
        Save();
        LookChanged.Notify();
    }
}
