using System.IO;
using System.Web.Script.Serialization;
using LemonGalaxy.UI;

namespace LemonGalaxy;

/// <summary>
/// 本机界面偏好，存在 %APPDATA%\LemonGalaxy\settings.json。selftest 下不读不写（Persistent = false）。
/// 外观、语言变化由主窗口整体重建界面；其他设置即时生效（通过 Changed 通知）。
/// </summary>
internal sealed class AppSettings : Observable
{
    public static readonly AppSettings Shared = new();
    public bool Persistent = true;
    private readonly Dictionary<string, object> values = new();
    private bool loading;

    /// <summary>外观或语言变了（需要整体重建界面）</summary>
    public readonly Observable LookChanged = new();

    private string S(string k, string d) => values.TryGetValue(k, out var v) && v is string s ? s : d;
    private bool B(string k, bool d) => values.TryGetValue(k, out var v) && v is bool b ? b : d;
    private int I(string k, int d) => values.TryGetValue(k, out var v) ? v switch { int i => i, long l => (int)l, decimal m => (int)m, double x => (int)x, _ => d } : d;

    private void Set(string k, object v, bool look = false)
    {
        if (values.TryGetValue(k, out var old) && Equals(old, v)) return;
        values[k] = v;
        if (loading) return;
        Save();
        Notify();
        if (look) LookChanged.Notify();
    }

    // 外观
    public string Appearance { get => S("appearance", "system"); set => Set("appearance", value, true); }          // system | light | dark
    public string Palette { get => S("palette", "galaxy"); set => Set("palette", value, true); }
    public string Language { get => S("language", "system"); set => Set("language", value, true); }
    public bool SidebarCompact { get => B("sidebarCompact", false); set => Set("sidebarCompact", value); }
    public bool ListHidden { get => B("listHidden", false); set => Set("listHidden", value); }
    /// <summary>切到终端、远程桌面等会话标签时自动收起中间列表，回到首页再出现</summary>
    public bool AutoHideList { get => B("autoHideList", true); set => Set("autoHideList", value); }

    // 终端
    public string TerminalTheme { get => S("terminalTheme", "galaxy-dark"); set => Set("terminalTheme", value); }
    public string TerminalFont { get => S("terminalFont", ""); set => Set("terminalFont", value); }
    public int TerminalFontSize { get => I("terminalFontSize", 14); set => Set("terminalFontSize", Math.Max(9, Math.Min(28, value))); }
    public bool TerminalCursorBlink { get => B("terminalCursorBlink", true); set => Set("terminalCursorBlink", value); }
    public bool TerminalCopyOnSelect { get => B("terminalCopyOnSelect", false); set => Set("terminalCopyOnSelect", value); }
    public bool TerminalRightClickPaste { get => B("terminalRightClickPaste", false); set => Set("terminalRightClickPaste", value); }

    // 连接
    public string DisplayQuality { get => S("displayQuality", "balanced"); set => Set("displayQuality", value); }     // smooth | balanced | sharp | custom
    public int DisplayFps { get => I("displayFps", 60); set => Set("displayFps", value); }
    public int DisplayBitrateKbps { get => I("displayBitrateKbps", 12000); set => Set("displayBitrateKbps", value); }
    public bool ClipboardSync { get => B("clipboardSync", true); set => Set("clipboardSync", value); }
    /// <summary>远程桌面播放被控端声音（工具栏开关，记住上次的选择）</summary>
    public bool RemoteAudio { get => B("remoteAudio", true); set => Set("remoteAudio", value); }
    public bool ReattachTerminals { get => B("reattachTerminals", true); set => Set("reattachTerminals", value); }

    // 本机被控（Windows 被控服务）
    /// <summary>远程结束后自动锁屏：最后一次在本机设置成功的值（服务的数据目录只有管理员能读，主控读不到实际值）</summary>
    public bool WorkerLockOnEnd { get => B("workerLockOnEnd", false); set => Set("workerLockOnEnd", value); }

    // AI
    public string AiDefaultMode { get => S("aiDefaultMode", "confirmDangerous"); set => Set("aiDefaultMode", value); }
    public bool AiPlanFirst { get => B("aiPlanFirst", false); set => Set("aiPlanFirst", value); }
    /// <summary>上次选的模型："provider_id:model_id"</summary>
    public string AiLastModel { get => S("aiLastModel", ""); set => Set("aiLastModel", value); }
    /// <summary>完全信任的时限（分钟），0 表示「本对话」（不限时）</summary>
    public int AiTrustMinutes { get => I("aiTrustMinutes", 0); set => Set("aiTrustMinutes", value); }

    // 登录与窗口
    public string LastServer { get => S("lastServer", ""); set => Set("lastServer", value); }
    public string LastUsername { get => S("lastUsername", ""); set => Set("lastUsername", value); }
    public int WindowWidth { get => I("windowWidth", 1280); set => Set("windowWidth", value); }
    public int WindowHeight { get => I("windowHeight", 800); set => Set("windowHeight", value); }
    public bool WindowMaximized { get => B("windowMaximized", false); set => Set("windowMaximized", value); }

    public AiMode AiMode
    {
        get => AiModes.From(AiDefaultMode);
        set => AiDefaultMode = value.Key();
    }

    /// <summary>画质预设 → 帧率与码率（与 Mac 一致）</summary>
    public static (int fps, int kbps) Quality(string q) => q switch
    {
        "smooth" => (60, 6000),
        "sharp" => (60, 30000),
        _ => (60, 12000),
    };

    private static string FilePath => IOPath.Combine(AppInfo.ConfigDir, "settings.json");

    public void Load()
    {
        if (!Persistent) return;
        try
        {
            if (!File.Exists(FilePath)) return;
            var obj = new JavaScriptSerializer().DeserializeObject(File.ReadAllText(FilePath)) as Dictionary<string, object>;
            if (obj == null) return;
            loading = true;
            foreach (var kv in obj) values[kv.Key] = kv.Value;
        }
        catch (Exception e) { Log.Error("读取设置失败：" + e.Message); }
        finally { loading = false; }
    }

    /// <summary>selftest：恢复默认值、不落盘</summary>
    public void ResetForSelfTest()
    {
        Persistent = false;
        values.Clear();
    }

    private void Save()
    {
        if (!Persistent) return;
        try
        {
            Directory.CreateDirectory(AppInfo.ConfigDir);
            var tmp = FilePath + ".tmp";
            File.WriteAllText(tmp, new JavaScriptSerializer().Serialize(values));
            if (File.Exists(FilePath)) File.Delete(FilePath);
            File.Move(tmp, FilePath);
        }
        catch (Exception e) { Log.Error("保存设置失败：" + e.Message); }
    }
}

internal static class AiModes
{
    public static readonly AiMode[] Selectable = { AiMode.ReadOnly, AiMode.ConfirmAll, AiMode.ConfirmDangerous, AiMode.FullTrust };

    public static string Key(this AiMode m) => m switch
    {
        AiMode.Off => "off",
        AiMode.ReadOnly => "readOnly",
        AiMode.ConfirmAll => "confirmAll",
        AiMode.FullTrust => "fullTrust",
        _ => "confirmDangerous",
    };

    public static AiMode From(string s) => s switch
    {
        "off" => AiMode.Off,
        "readOnly" => AiMode.ReadOnly,
        "confirmAll" => AiMode.ConfirmAll,
        "fullTrust" => AiMode.FullTrust,
        _ => AiMode.ConfirmDangerous,
    };

    /// <summary>off &lt; read_only &lt; confirm_all &lt; confirm_dangerous &lt; full_trust（ai-agent.md 第 5 节）</summary>
    public static int Rank(this AiMode m) => m switch
    {
        AiMode.Off => 0,
        AiMode.ReadOnly => 1,
        AiMode.ConfirmAll => 2,
        AiMode.ConfirmDangerous => 3,
        _ => 4,
    };

    public static string LabelKey(this AiMode m) => "ai.mode." + m.Key();
}
