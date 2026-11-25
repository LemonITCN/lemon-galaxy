using System.Globalization;
using System.IO;
using System.Text;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using LemonGalaxy.Ipc;
using LemonGalaxy.Setup;
using LemonGalaxy.UI;

namespace LemonGalaxy;

/// <summary>
/// 自测（CI 与本机）：
///   1. 协议：请求写出来的 JSON 与 galaxy-worker/src/control.rs 测试里的字面量逐字一致；样例状态、设置、登录步骤能读对。
///   2. 界面：用假数据把各页面离屏渲染成 PNG（浅色 / 深色 × 中文 / 英文），不显示窗口、不连服务、不读写用户设置。
///   3. 语言包：渲染过程中用到但缺的键。
///   4. --live：连真的被控服务（环境变量 LEMON_GALAXY_CONTROL_SOCKET 指定的管道），握手、读状态与设置、改一项设置再读回、拿授权钥匙；
///      再用界面平时用的后台连接（await）发一次请求。
///   5. await：回到界面线程、失败时抛出里面的那个异常（老系统版按 .NET 4.0 编，这些是 App/Net40.cs 里补的）。
/// 退出码：0 全部通过；1 有问题（见输出目录 report.txt）。
/// </summary>
internal static class SelfTest
{
    private static readonly Size WindowSize = new(1080, 720);
    private static string dir = "selftest";
    private static int count;
    private static readonly List<string> Problems = new();
    private static readonly StringBuilder Report = new();

    public static int Run(string outputDir, string[] args)
    {
        try
        {
            dir = IOPath.GetFullPath(outputDir);
            Directory.CreateDirectory(dir);
            Log.FilePath = IOPath.Combine(dir, "ui.log");
            var settings = AppSettings.Shared;
            L.LoadPacks();
            if (L.Packs.Count < 2) Problems.Add("语言包少于两个：" + string.Join(", ", L.Packs.Select(p => p.Code)));
            Protocol();
            var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
            // 每张图都是新建的一棵界面；前面几张留下的区域不能再跟着状态重建（否则会把弹层卡片挪到旧的那棵里去）
            Region.RequireAttached = true;
            Styles.Install(app);
            if (Styles.LoadError != null) Problems.Add("控件模板解析失败：" + Styles.LoadError);
            Report.AppendLine("运行时 " + Environment.Version + "，界面按 " + AppInfo.Framework + " 编");
            AsyncCheck();
            foreach (var lang in new[] { "zh-Hans", "en" })
                foreach (var theme in new[] { "light", "dark" })
                    RenderAll(settings, lang, theme);
            CheckLocales();
            if (Array.IndexOf(args, "--live") >= 0) Live();
        }
        catch (Exception e) { Problems.Add("自测异常：" + e); }
        Report.AppendLine($"截图 {count} 张");
        var missing = L.MissingKeys;
        if (missing.Length > 0) Problems.Add("语言包缺键：" + string.Join(", ", missing));
        foreach (var p in Problems) Report.AppendLine("问题：" + p);
        Report.AppendLine(Problems.Count == 0 ? "全部通过" : $"共 {Problems.Count} 个问题");
        try { File.WriteAllText(IOPath.Combine(dir, "report.txt"), Report.ToString(), new UTF8Encoding(false)); } catch { }
        Console.WriteLine(Report.ToString());
        return Problems.Count == 0 ? 0 : 1;
    }

    private static void Check(bool ok, string what)
    {
        if (ok) Report.AppendLine("通过：" + what);
        else Problems.Add(what);
    }

    // ---------------- 协议 ----------------

    private static void Protocol()
    {
        // 与 control.rs 的 responses_and_events_round_trip / requests_round_trip 里的字面量一致
        Check(Req.Line(1, Req.Status()) == "{\"id\":1,\"req\":{\"op\":\"status\"}}\n", "请求格式：status");
        Check(Req.Line(6, Req.Elevate("k")) == "{\"id\":6,\"req\":{\"op\":\"elevate\",\"key\":\"k\"}}\n", "请求格式：elevate");
        Check(Req.Line(7, Req.ElevationKey()) == "{\"id\":7,\"req\":{\"op\":\"elevation_key\"}}\n", "请求格式：elevation_key");
        Check(Req.Line(2, Req.AssistStart(null)) == "{\"id\":2,\"req\":{\"op\":\"assist_start\"}}\n", "请求格式：assist_start 不带服务器");
        var login = Req.Line(3, Req.Login("h:1#sha256-x", "u", "p", null, false));
        Check(login == "{\"id\":3,\"req\":{\"op\":\"login\",\"server\":\"h:1#sha256-x\",\"username\":\"u\",\"password\":\"p\",\"replace_server\":false}}\n", "请求格式：login（没有两步验证码时不写 totp）" + login);
        var pol = Req.Line(4, Req.SetPolicy(7, new Policy("PC\\alice", true, false, true, true, true, false, "confirm_dangerous", true), false));
        Check(pol.Contains("\"policy\":{\"system_user\":\"PC\\\\alice\",\"view\":true,\"control\":false") && pol.Contains("\"ai_cap\":\"confirm_dangerous\""), "请求格式：set_policy " + pol);
        Check(Req.Line(5, Req.SetAssistCode(null)) == "{\"id\":5,\"req\":{\"op\":\"set_assist_code\",\"code\":null}}\n", "请求格式：清除固定验证码");

        var line = "{\"event\":{\"type\":\"status\",\"status\":" + SampleStatusJson + "}}";
        var j = J.Parse(line);
        var st = j?.O("event")?.O("status") is { } sj ? Status.From(sj) : null;
        Check(st != null && st.DeviceId == new string('a', 32) && st.Sessions.Count == 1 && st.Sessions[0].Relayed && st.Sessions[0].AiActive
              && st.Rights.Manage == ManageRight.Elevate && st.Rights.NeedsElevation && st.Assist?.Code == "ABCDEFGH" && st.ConfigRev == 1700000000123, "读状态");
        var resp = J.Parse("{\"id\":9,\"resp\":{\"type\":\"error\",\"code\":\"elevation_required\",\"message\":\"x\"}}");
        var r = resp?.O("resp") is { } rj ? new Response(rj) : null;
        Check(r != null && !r.Ok && r.Code == ErrorCode.ElevationRequired && resp!.L("id") == 9, "读错误回复");
        var set = J.Parse("{\"auto_update\":true,\"lock_on_end\":false,\"unlock_on_connect\":false,\"headless_display\":{\"width\":2560,\"height\":1440,\"refresh\":60},\"autostart\":{\"available\":true,\"enabled\":true},\"login_screen_x11\":{\"available\":false,\"enabled\":false,\"detail\":\"Linux only.\"}}");
        var ws = set == null ? null : WorkerSettings.From(set);
        Check(ws != null && ws.AutoUpdate && ws.HeadlessDisplay?.Id == "2560x1440" && ws.Autostart.Available && ws.AssistCode == null, "读设置");
        var step = J.Parse("{\"kind\":\"choose_system_user\",\"username\":\"u\",\"display_name\":\"U\",\"default_user\":\"PC\\\\alice\",\"users\":[\"PC\\\\alice\",\"SYSTEM\"],\"can_choose\":true,\"superuser\":\"SYSTEM\"}");
        Check(step != null && step.AS("users").Count == 2 && step.S("default_user") == "PC\\alice" && step.B("can_choose"), "读登录步骤");
    }

    private const string SampleStatusJson = "{\"version\":\"0.1.0.99\",\"device_id\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"hostname\":\"DESKTOP-LEMON\",\"data_dir\":\"C:\\\\ProgramData\\\\LemonGalaxy\\\\worker\","
        + "\"service\":{\"mode\":\"system\",\"user\":\"SYSTEM\",\"pid\":4242},\"account_count\":2,\"needs_relogin\":1,\"online\":true,\"server\":\"galaxy.example.com:7443#sha256-AbCdEf\",\"public_ip\":\"203.0.113.7\","
        + "\"remote_enabled\":true,\"sessions\":[{\"id\":\"s1\",\"from_user\":\"Alice（alice）\",\"from_device\":\"cd34ef56aa\",\"mode\":\"control\",\"path\":\"relay_udp\",\"started_at\":1700000000000,\"ai_active\":true,\"assist\":false,\"notify\":true}],"
        + "\"assist\":{\"mine\":true,\"device_code\":\"123 456 789\",\"code\":\"ABCDEFGH\"},\"rights\":{\"operate\":true,\"view_config\":true,\"manage\":\"elevate\"},\"you\":\"DESKTOP-LEMON\\\\alice\",\"config_rev\":1700000000123}";

    // ---------------- 界面 ----------------

    private static AppModel Fake(AppSettings settings)
    {
        var app = new AppModel(settings, selfTest: true) { ConnectGraceOver = true };
        var j = J.Parse(SampleStatusJson)!;
        app.Status = Status.From(j);
        app.Status.Sessions.Add(new Session("s2", "Bob（bob）", "a1b2c3d4e5", "assist", "direct", Clock.NowMs - 600000, false, true, true));
        app.EverConnected = true;
        app.Service = ServiceState.Running;
        app.Accounts = new()
        {
            new Account(1, "alice", "Alice", true, 0, new Policy("DESKTOP-LEMON\\alice", true, true, true, true, true, true, "confirm_all", true)),
            new Account(2, "ops", "", false, 0, new Policy("SYSTEM", true, true, false, true, false, false, "read_only", false)),
        };
        app.Worker = new WorkerSettings
        {
            AutoUpdate = true, LockOnEnd = true, HeadlessDisplay = new DisplayMode(1920, 1080, 60), AssistCode = "LEMON2026",
            Autostart = new Feature(true, true, ""), LoginScreenX11 = new Feature(false, false, ""),
        };
        app.Events = new()
        {
            new LogEntry(Clock.NowMs - 3600000, "s0", "alice", "connect", "relay_udp · DESKTOP-OFFICE", false),
            new LogEntry(Clock.NowMs - 3000000, "s0", "alice", "file", "upload C:\\Users\\alice\\Desktop\\report.xlsx (2.1 MB)", false),
            new LogEntry(Clock.NowMs - 2400000, "s0", "alice", "exec", "powershell Get-Service LemonGalaxyWorker", true),
            new LogEntry(Clock.NowMs - 1800000, "", "mallory", "auth_failed", "bad signature", false),
            new LogEntry(Clock.NowMs - 600000, "s1", "alice", "ai_denied", "format C: (blocked by AI limit)", true),
        };
        return app;
    }

    private static void RenderAll(AppSettings settings, string lang, string theme)
    {
        settings.Language = lang;
        settings.Appearance = theme;
        L.Apply(lang);
        Th.Apply(settings);
        var zh = lang.StartsWith("zh", StringComparison.Ordinal);
        var p = $"{(zh ? "zh" : "en")}-{theme}-";
        var app = Fake(settings);
        foreach (var page in new[] { Page.Overview, Page.Accounts, Page.Assist, Page.Settings, Page.Events, Page.About })
        {
            app.Page = page;
            Capture(p + page.ToString().ToLowerInvariant(), Shell.Root(app));
        }
        // 弹层：权限、登录、确认
        app.Page = Page.Accounts;
        PolicyDialog.Open(app, app.Accounts![0]);
        Capture(p + "policy", Shell.Root(app));
        app.Modals.Clear();
        Login.Open(app);
        Capture(p + "login", Shell.Root(app));
        app.Modals.Clear();
        _ = app.Confirm(L.T("accounts.signOut.title", "alice"), L.T("accounts.signOut.body"), L.T("accounts.signOut"), true);
        Capture(p + "confirm", Shell.Root(app));
        app.Modals.Clear();
        // 协助：没开、别的窗口开的
        app.Page = Page.Assist;
        app.Status!.Assist = null;
        app.Status.Server = "";
        Capture(p + "assist-idle", Shell.Root(app));
        // 普通用户：看不到配置
        app.Status.Rights = new Rights(false, false, ManageRight.Elevate);
        app.Page = Page.Settings;
        Capture(p + "settings-denied", Shell.Root(app));
        app.Page = Page.Accounts;
        Capture(p + "accounts-denied", Shell.Root(app));
        // 没连上服务：安装、启动、连接中
        var down = new AppModel(settings, selfTest: true) { ConnectGraceOver = true, Service = ServiceState.NotInstalled };
        Capture(p + "setup", Shell.Root(down));
        down.Service = ServiceState.Stopped;
        Capture(p + "service-stopped", Shell.Root(down));
        down.Busy = "install";
        Capture(p + "installing", Shell.Root(down));
        down.Busy = null;
        down.Service = ServiceState.Unknown;
        Capture(p + "connecting", Shell.Root(down));
        // 提示条
        var t = Fake(settings);
        t.ShowToast(L.T("policy.saved"));
        Capture(p + "toast", Shell.Root(t));
        // 托盘图标
        foreach (var o in new[] { Overall.Online, Overall.Controlled, Overall.Disabled, Overall.NotRunning, Overall.NoAccount })
        {
            try
            {
                foreach (var size in new[] { 16, 24, 32 })
                    File.WriteAllBytes(IOPath.Combine(dir, $"{p}tray-{o.ToString().ToLowerInvariant()}-{size}.png"), Tray.Render(o, size));
                count++;
            }
            catch (Exception e) { Problems.Add("托盘图标渲染失败：" + e.Message); }
        }
    }

    private static void Settle(FrameworkElement root, Size size)
    {
        for (var i = 0; i < 4; i++)
        {
            Ui.DoEvents();
            root.Measure(size);
            root.Arrange(new Rect(size));
            root.UpdateLayout();
        }
        Ui.Pump(30);
        root.Measure(size);
        root.Arrange(new Rect(size));
        root.UpdateLayout();
    }

    private static void Capture(string name, FrameworkElement root)
    {
        try
        {
            Settle(root, WindowSize);
            var rtb = new RenderTargetBitmap((int)WindowSize.Width, (int)WindowSize.Height, 96, 96, PixelFormats.Pbgra32);
            var bg = new DrawingVisual();
            using (var dc = bg.RenderOpen()) dc.DrawRectangle(Th.Content, null, new Rect(WindowSize));
            rtb.Render(bg);
            rtb.Render(root);
            var enc = new PngBitmapEncoder();
            enc.Frames.Add(BitmapFrame.Create(rtb));
            using var fs = File.Create(IOPath.Combine(dir, name + ".png"));
            enc.Save(fs);
            count++;
        }
        catch (Exception e) { Problems.Add($"渲染 {name} 失败：{e.Message}"); }
    }

    /// <summary>两种语言的键要一样多</summary>
    private static void CheckLocales()
    {
        var zh = L.Packs.FirstOrDefault(p => p.Code == "zh-Hans");
        var en = L.Packs.FirstOrDefault(p => p.Code == "en");
        if (zh == null || en == null)
        {
            Problems.Add("缺中文或英文语言包");
            return;
        }
        var onlyZh = zh.Strings.Keys.Except(en.Strings.Keys).ToList();
        var onlyEn = en.Strings.Keys.Except(zh.Strings.Keys).ToList();
        Check(onlyZh.Count == 0, "英文语言包缺：" + string.Join(", ", onlyZh));
        Check(onlyEn.Count == 0, "中文语言包缺：" + string.Join(", ", onlyEn));
    }

    // ---------------- 连真服务 ----------------

    private static void Live()
    {
        var env = Environment.GetEnvironmentVariable("LEMON_GALAXY_CONTROL_SOCKET");
        var (name, system) = env is { Length: > 0 } ? (ControlClient.StripPrefix(env), false) : (ControlClient.SystemPipe, true);
        try
        {
            var rs = ControlClient.Once(name, system, new[]
            {
                Req.Hello("zh-CN"), Req.Status(), Req.Settings(), Req.SetLockOnEnd(true), Req.Settings(), Req.SetLockOnEnd(false),
                Req.Events(10), Req.ElevationKey(), Req.SystemUsers(), Req.Accounts(),
            });
            Check(rs[0].Type == "hello" && rs[0].J.L("protocol") >= 1, "连服务：握手 " + rs[0].J.S("version"));
            var st = rs[1].J.O("status") is { } sj ? Status.From(sj) : null;
            Check(st != null && st.DeviceId.Length == 32, "连服务：状态 " + st?.You + " " + st?.Rights.Manage);
            Check(rs[2].Type == "settings", "连服务：设置");
            Check(rs[3].Ok, "连服务：改设置 " + rs[3].Message);
            Check(rs[4].J.O("settings")?.B("lock_on_end") == true, "连服务：改过的设置读得回来");
            Check(rs[6].Type == "events", "连服务：事件日志");
            Check(rs[7].Type == "elevation_key" && rs[7].J.S("key").Length == 32, "连服务：授权钥匙");
            Check(rs[8].Type == "system_users" && rs[8].J.AS("users").Count > 0, "连服务：系统用户 " + string.Join(", ", rs[8].J.AS("users")));
            Check(rs[9].Type == "accounts", "连服务：账号");
            Report.AppendLine("服务版本 " + rs[0].J.S("version") + "，管道 " + name);
        }
        catch (Exception e) { Problems.Add("连服务失败：" + e.Message); }
        LiveAsync();
    }

    /// <summary>界面平时的连法：后台线程连接与读，回复经 await 回到界面线程</summary>
    private static void LiveAsync()
    {
        var ui = Thread.CurrentThread.ManagedThreadId;
        using var client = new ControlClient(a => Ui.Post(a));
        var connected = false;
        client.Connected += () => connected = true;
        client.Start();
        if (!Ui.WaitFor(() => connected, 10000))
        {
            Problems.Add("连服务（后台连接）：10 秒没连上");
            return;
        }
        string? result = null;
        Ui.Run(async () =>
        {
            try
            {
                var r = await client.Call(Req.Status());
                var st = r.J.O("status") is { } sj ? Status.From(sj) : null;
                result = (Thread.CurrentThread.ManagedThreadId == ui) + "|" + (st?.DeviceId.Length == 32);
            }
            catch (Exception e) { result = "异常 " + e.Message; }
        });
        Ui.WaitFor(() => result != null, 10000);
        Check(result == "True|True", "连服务（后台连接）：await 拿到状态并回到界面线程 " + result);
    }

    /// <summary>await 的行为与 .NET 4.5 起一致（老系统版用的是自己补的实现）</summary>
    private static void AsyncCheck()
    {
        var ui = Thread.CurrentThread.ManagedThreadId;
        string? result = null;
        async Task<string> Steps()
        {
            var bg = await Bg.Run(() => Thread.CurrentThread.ManagedThreadId);
            var back = Thread.CurrentThread.ManagedThreadId;
            var err = "";
            try { await Bg.Run<int>(() => throw new InvalidOperationException("boom")); }
            catch (InvalidOperationException e) { err = e.Message; }
            var done = await Bg.Done(5);
            await Bg.Run(() => Thread.Sleep(20));
            return $"{bg != ui}|{back == ui}|{err}|{done}|{Thread.CurrentThread.ManagedThreadId == ui}";
        }
        Ui.Run(async () =>
        {
            try { result = await Steps(); }
            catch (Exception e) { result = "异常 " + e; }
        });
        Ui.WaitFor(() => result != null, 10000);
        Check(result == "True|True|boom|5|True", "await：后台跑完回到界面线程、失败时抛出里面的异常 " + result);
    }
}
