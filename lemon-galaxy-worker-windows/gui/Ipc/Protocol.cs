using System.Collections;
using System.Web.Script.Serialization;

namespace LemonGalaxy.Ipc;

/// <summary>
/// 本机控制接口的消息（与 galaxy-worker/src/control.rs 一一对应，一行一个 JSON）：
///   → {"id":1,"req":{"op":"hello","lang":"zh-CN"}}
///   ← {"id":1,"resp":{"type":"hello","protocol":1,"version":"0.1.0"}}
///   ← {"event":{"type":"status","status":{…}}}
/// 字段名、枚举值按 control.rs 的 serde 写法（snake_case）。只加字段时两边互相兼容，读不到的字段取默认值。
/// </summary>
internal sealed class J
{
    public readonly Dictionary<string, object?> D;

    public J(Dictionary<string, object?> d) { D = d; }

    public static J? Parse(string line)
    {
        try { return new JavaScriptSerializer { MaxJsonLength = int.MaxValue }.DeserializeObject(line) is Dictionary<string, object?> d ? new J(d) : null; }
        catch { return null; }
    }

    public static string Encode(object o) => new JavaScriptSerializer { MaxJsonLength = int.MaxValue }.Serialize(o);

    public bool Has(string k) => D.ContainsKey(k) && D[k] != null;
    public string S(string k, string def = "") => D.TryGetValue(k, out var v) && v is string s ? s : def;
    public string? SOpt(string k) => D.TryGetValue(k, out var v) ? v as string : null;
    public bool B(string k, bool def = false) => D.TryGetValue(k, out var v) && v is bool b ? b : def;

    public long L(string k) => D.TryGetValue(k, out var v) ? v switch
    {
        int i => i,
        long l => l,
        decimal m => (long)m,
        double d => (long)d,
        _ => 0,
    } : 0;

    public J? O(string k) => D.TryGetValue(k, out var v) && v is Dictionary<string, object?> d ? new J(d) : null;

    private IEnumerable<object?> Items(string k) => D.TryGetValue(k, out var v) ? v switch
    {
        object?[] a => a,
        ArrayList al => al.Cast<object?>(),
        _ => Array.Empty<object?>(),
    } : Array.Empty<object?>();

    public List<J> A(string k) => Items(k).OfType<Dictionary<string, object?>>().Select(x => new J(x)).ToList();
    public List<string> AS(string k) => Items(k).OfType<string>().ToList();
}

// ---------------- 数据 ----------------

internal static class ManageRight
{
    /// <summary>直接能改（SYSTEM、已提升的管理员、刚经 UAC 授权过的连接）</summary>
    public const string Direct = "direct";
    /// <summary>要经 UAC 授权（界面弹「用户账户控制」，几分钟内不再问）</summary>
    public const string Elevate = "elevate";
    public const string Polkit = "polkit";
    public const string Denied = "denied";
}

internal sealed record Rights(bool Operate, bool ViewConfig, string Manage)
{
    public bool CanManage => Manage != ManageRight.Denied;
    /// <summary>改配置前要先经 UAC（按钮上画盾牌）</summary>
    public bool NeedsElevation => Manage == ManageRight.Elevate || Manage == ManageRight.Polkit;

    public static Rights From(J? j) => j == null ? new(false, false, ManageRight.Denied) : new(j.B("operate"), j.B("view_config"), j.S("manage", ManageRight.Denied));
}

internal sealed record Session(string Id, string FromUser, string FromDevice, string Mode, string Path, long StartedAt, bool AiActive, bool Assist, bool Notify)
{
    public bool Relayed => Path is "relay_udp" or "relay_tcp";

    public static Session From(J j) => new(j.S("id"), j.S("from_user"), j.S("from_device"), j.S("mode"), j.S("path"), j.L("started_at"), j.B("ai_active"), j.B("assist"), j.B("notify", true));
}

internal sealed record AssistInfo(bool Mine, string DeviceCode, string Code)
{
    public static AssistInfo From(J j) => new(j.B("mine"), j.S("device_code"), j.S("code"));
}

internal sealed record ServiceInfo(string Mode, string User, long Pid)
{
    public static ServiceInfo From(J? j) => j == null ? new("system", "", 0) : new(j.S("mode", "system"), j.S("user"), j.L("pid"));
}

internal sealed class Status
{
    public string Version = "", DeviceId = "", Hostname = "", DataDir = "", Server = "", PublicIp = "", You = "";
    public ServiceInfo Service = new("system", "", 0);
    public int AccountCount, NeedsRelogin;
    public bool Online, RemoteEnabled;
    public List<Session> Sessions = new();
    public AssistInfo? Assist;
    public Rights Rights = new(false, false, ManageRight.Denied);
    public long ConfigRev;

    public bool Controlled => Sessions.Count > 0;
    public bool AiActive => Sessions.Any(s => s.AiActive);

    public static Status From(J j) => new()
    {
        Version = j.S("version"), DeviceId = j.S("device_id"), Hostname = j.S("hostname"), DataDir = j.S("data_dir"),
        Service = ServiceInfo.From(j.O("service")), AccountCount = (int)j.L("account_count"), NeedsRelogin = (int)j.L("needs_relogin"),
        Online = j.B("online"), Server = j.S("server"), PublicIp = j.S("public_ip"), RemoteEnabled = j.B("remote_enabled"),
        Sessions = j.A("sessions").Select(Session.From).ToList(), Assist = j.O("assist") is { } a ? AssistInfo.From(a) : null,
        Rights = Rights.From(j.O("rights")), You = j.S("you"), ConfigRev = j.L("config_rev"),
    };
}

/// <summary>账号在本机的权限（control.rs 的 Policy）</summary>
internal sealed record Policy(string SystemUser, bool View, bool Control, bool Files, bool Terminal, bool Clipboard, bool Audio, string AiCap, bool Notify)
{
    public static Policy From(J? j) => j == null
        ? new("", true, true, true, true, true, true, "confirm_all", true)
        : new(j.S("system_user"), j.B("view"), j.B("control"), j.B("files"), j.B("terminal"), j.B("clipboard"), j.B("audio"), j.S("ai_cap", "confirm_all"), j.B("notify", true));

    public Dictionary<string, object?> ToJson() => new()
    {
        ["system_user"] = SystemUser, ["view"] = View, ["control"] = Control, ["files"] = Files, ["terminal"] = Terminal,
        ["clipboard"] = Clipboard, ["audio"] = Audio, ["ai_cap"] = AiCap, ["notify"] = Notify,
    };
}

internal sealed record Account(long UserId, string Username, string DisplayName, bool Active, long AddedAt, Policy Policy)
{
    public string Title => DisplayName.Length == 0 || DisplayName == Username ? Username : $"{DisplayName}（{Username}）";

    public static Account From(J j) => new(j.L("user_id"), j.S("username"), j.S("display_name"), j.B("active"), j.L("added_at"), Policy.From(j.O("policy")));
}

internal sealed record Feature(bool Available, bool Enabled, string Detail)
{
    public static Feature From(J? j) => j == null ? new(false, false, "") : new(j.B("available"), j.B("enabled"), j.S("detail"));
}

internal sealed record DisplayMode(int Width, int Height, int Refresh)
{
    public string Id => Refresh == 60 ? $"{Width}x{Height}" : $"{Width}x{Height}@{Refresh}";
}

internal sealed class WorkerSettings
{
    public bool AutoUpdate, LockOnEnd, UnlockOnConnect;
    public DisplayMode? HeadlessDisplay;
    public string? AssistCode;
    public Feature Autostart = new(false, false, "");
    public Feature LoginScreenX11 = new(false, false, "");

    public static WorkerSettings From(J j) => new()
    {
        AutoUpdate = j.B("auto_update"), LockOnEnd = j.B("lock_on_end"), UnlockOnConnect = j.B("unlock_on_connect"),
        HeadlessDisplay = j.O("headless_display") is { } h ? new DisplayMode((int)h.L("width"), (int)h.L("height"), (int)h.L("refresh")) : null,
        AssistCode = j.SOpt("assist_code"), Autostart = Feature.From(j.O("autostart")), LoginScreenX11 = Feature.From(j.O("login_screen_x11")),
    };
}

internal sealed record LogEntry(long At, string Session, string User, string Kind, string Detail, bool Ai)
{
    public bool Warning => Kind is "reject" or "auth_failed" or "ai_denied" or "denied";

    public static LogEntry From(J j) => new(j.L("at"), j.S("session"), j.S("user"), j.S("kind"), j.S("detail"), j.B("ai"));
}

/// <summary>服务的回复。type 为 error 时 Code、Message 有值。</summary>
internal sealed class Response
{
    public readonly J J;
    public string Type => J.S("type");
    public bool Ok => Type != "error";
    public string Code => J.S("code");
    public string Message => J.S("message");

    public Response(J j) { J = j; }

    public static Response Error(string code, string message) => new(new J(new Dictionary<string, object?> { ["type"] = "error", ["code"] = code, ["message"] = message }));
}

internal static class ErrorCode
{
    public const string Denied = "denied", AuthFailed = "auth_failed", Invalid = "invalid", NotFound = "not_found", NotApplicable = "not_applicable",
        Busy = "busy", ConfirmRoot = "confirm_root", InvalidTotp = "invalid_totp", InvalidCredentials = "invalid_credentials", Failed = "failed",
        BadRequest = "bad_request", ElevationRequired = "elevation_required";
}

// ---------------- 请求 ----------------

/// <summary>请求（op + 字段）。键的顺序就是写出去的顺序，与 control.rs 的测试字面量一致。</summary>
internal static class Req
{
    private static Dictionary<string, object?> Op(string op, params (string, object?)[] fields)
    {
        var d = new Dictionary<string, object?> { ["op"] = op };
        foreach (var (k, v) in fields) d[k] = v;
        return d;
    }

    public static Dictionary<string, object?> Hello(string lang) => Op("hello", ("lang", lang));
    public static Dictionary<string, object?> Status() => Op("status");
    public static Dictionary<string, object?> Accounts() => Op("accounts");
    public static Dictionary<string, object?> SystemUsers() => Op("system_users");

    public static Dictionary<string, object?> Login(string server, string username, string password, string? totp, bool replaceServer)
    {
        var d = Op("login", ("server", server), ("username", username), ("password", password));
        if (!string.IsNullOrEmpty(totp)) d["totp"] = totp;
        d["replace_server"] = replaceServer;
        return d;
    }

    public static Dictionary<string, object?> LoginSetPassword(string pw) => Op("login_set_password", ("new_password", pw));
    public static Dictionary<string, object?> LoginFinish(string systemUser, bool confirmRoot) => Op("login_finish", ("system_user", systemUser), ("confirm_root", confirmRoot));
    public static Dictionary<string, object?> LoginCancel() => Op("login_cancel");
    public static Dictionary<string, object?> Logout(long userId) => Op("logout", ("user_id", userId));
    public static Dictionary<string, object?> SetPolicy(long userId, Policy p, bool confirmRoot) => Op("set_policy", ("user_id", userId), ("policy", p.ToJson()), ("confirm_root", confirmRoot));
    public static Dictionary<string, object?> SetRemoteEnabled(bool on) => Op("set_remote_enabled", ("enabled", on));
    public static Dictionary<string, object?> Disconnect(string sessionId) => Op("disconnect", ("session_id", sessionId));
    public static Dictionary<string, object?> DisconnectAll() => Op("disconnect_all");
    public static Dictionary<string, object?> StopAi() => Op("stop_ai");

    public static Dictionary<string, object?> AssistStart(string? server)
        => string.IsNullOrWhiteSpace(server) ? Op("assist_start") : Op("assist_start", ("server", server!.Trim()));

    public static Dictionary<string, object?> AssistStop() => Op("assist_stop");
    public static Dictionary<string, object?> AssistAnswer(long requestId, bool allow) => Op("assist_answer", ("request_id", requestId), ("allow", allow));
    public static Dictionary<string, object?> Settings() => Op("settings");
    public static Dictionary<string, object?> SetAssistCode(string? code) => Op("set_assist_code", ("code", code));
    public static Dictionary<string, object?> SetAutoUpdate(bool on) => Op("set_auto_update", ("enabled", on));
    public static Dictionary<string, object?> SetLockOnEnd(bool on) => Op("set_lock_on_end", ("enabled", on));
    public static Dictionary<string, object?> SetHeadlessDisplay(string value) => Op("set_headless_display", ("value", value));
    public static Dictionary<string, object?> SetAutostart(bool on) => Op("set_autostart", ("enabled", on));
    public static Dictionary<string, object?> Events(int limit) => Op("events", ("limit", limit));
    public static Dictionary<string, object?> CheckUpdate() => Op("check_update");
    public static Dictionary<string, object?> ApplyUpdate() => Op("apply_update");
    public static Dictionary<string, object?> ElevationKey() => Op("elevation_key");
    public static Dictionary<string, object?> Elevate(string key) => Op("elevate", ("key", key));

    /// <summary>一行请求：{"id":n,"req":{…}}</summary>
    public static string Line(long id, Dictionary<string, object?> req) => J.Encode(new Dictionary<string, object?> { ["id"] = id, ["req"] = req }) + "\n";
}
