using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using LemonGalaxy.Ipc;
using LemonGalaxy.Setup;

namespace LemonGalaxy.UI;

internal static partial class Pages
{
    public static FrameworkElement Build(AppModel app) => app.Page switch
    {
        Page.Accounts => AccountsPage(app),
        Page.Assist => AssistPage(app),
        Page.Settings => SettingsPage(app),
        Page.Events => EventsPage(app),
        Page.About => AboutPage(app),
        _ => Overview(app),
    };

    /// <summary>页面外框：滚动 + 居中最宽 760</summary>
    private static ScrollViewer Frame(params UIElement?[] content) => K.PageScroll(K.Col(14, content), 760);

    // ---------------- 服务没运行 / 安装 ----------------

    /// <summary>
    /// 没连上被控服务时整页显示：正在连接 / 没装（从解压目录运行：完整安装；已在安装目录：只装服务）/ 没运行（启动服务）/
    /// 运行着但连不上（老版本服务没有本机控制接口：安装新版本）。
    /// </summary>
    public static FrameworkElement ServiceDown(AppModel app)
    {
        if (app.Busy is "install" or "start")
            return Centered(Shell.AppMark(64), K.Center(W.Busy(L.T(app.Busy == "install" ? "setup.installing" : "setup.starting"))));
        var installed = AppInfo.RunningInstalled;
        // 已装的那份与这份是同一版本：只差服务没起来
        var sameVersion = installed || AppInfo.InstalledVersion == AppInfo.Version;
        FrameworkElement Connecting(bool noPipe) => Centered(Shell.AppMark(64), K.Center(W.Busy(L.T("state.connecting"))),
            noPipe ? K.Center(K.Row(6, K.Text(L.T("setup.noPipe"), 12.5, Th.Secondary), K.Info(L.T("setup.noPipe.info")))) : null);
        switch (app.Service)
        {
            case ServiceState.Unknown:
                return Connecting(false);
            case ServiceState.Running or ServiceState.Starting:
                // 服务在跑却一直连不上：多半是没有本机控制接口的老版本，装这份新版本即可
                if (!app.ConnectGraceOver || app.EverConnected) return Connecting(false);
                return sameVersion ? Connecting(true) : SetupCard(app);
            case ServiceState.Stopped when sameVersion:
                return Centered(Shell.AppMark(64),
                    K.Center(K.Row(6, K.Semi(L.T("setup.stopped.title"), 17), K.Info(L.T("setup.stopped.info")))),
                    K.Center(W.Primary(L.T("setup.start"), () => Ui.Run(async () =>
                    {
                        if (await app.RunElevated("start", "--start-service")) app.ShowToast(L.T("setup.started"));
                    }), shield: true)));
            case ServiceState.NotInstalled when installed:
                return Centered(Shell.AppMark(64),
                    K.Center(K.Row(6, K.Semi(L.T("setup.noService.title"), 17), K.Info(L.T("setup.stopped.info")))),
                    K.Center(W.Primary(L.T("setup.installService"), () => Ui.Run(async () =>
                    {
                        if (await app.RunElevated("install", "--install")) app.ShowToast(L.T("setup.serviceInstalled"));
                    }), shield: true)));
            default:
                return SetupCard(app);
        }
    }

    private static FrameworkElement Centered(params UIElement?[] items)
    {
        var col = K.Col(16, items);
        col.HorizontalAlignment = HorizontalAlignment.Center;
        col.VerticalAlignment = VerticalAlignment.Center;
        // 安装卡片 540 宽，这里要放得下
        col.MaxWidth = 580;
        return new Grid { Background = Th.Content, Children = { col } };
    }

    /// <summary>完整安装（从解压目录运行时）：装到 Program Files、装服务、开始菜单、托盘自启</summary>
    private static FrameworkElement SetupCard(AppModel app)
    {
        var autostart = true;
        var tray = true;
        var upgrade = AppInfo.InstalledVersion;
        var title = upgrade == null ? L.T("setup.title") : L.T("setup.upgradeTitle");
        var sub = upgrade == null ? L.T("setup.version", AppInfo.Version) : L.T("setup.upgradeVersion", upgrade, AppInfo.Version);
        var card = K.Card(K.Col(16,
            K.Row(14, Shell.AppMark(52), K.Grow(K.Col(3, K.Row(6, K.Semi(title, 17), K.Info(L.T("setup.info"))), K.Text(sub, 12.5, Th.Secondary)))),
            K.HLine(),
            W.Row(L.T("setup.location"), W.Value(AppInfo.InstallDir)),
            K.Margin(K.CheckBox(L.T("setup.autostart"), autostart, v => autostart = v), 14, 0, 0, 0),
            K.Margin(K.CheckBox(L.T("setup.tray"), tray, v => tray = v), 14, 0, 0, 0),
            K.Row(10, K.Spacer(), W.Primary(upgrade == null ? L.T("setup.install") : L.T("setup.upgrade"), () => Ui.Run(async () =>
            {
                var args = new List<string> { "--install" };
                if (!autostart) args.Add("--no-autostart");
                if (!tray) args.Add("--no-tray");
                if (!await app.RunElevated("install", args.ToArray())) return;
                Program.LaunchInstalledAndExit();
            }), shield: true))), 22);
        card.Width = 540;
        return Centered(card);
    }

    // ---------------- 概览 ----------------

    private static FrameworkElement Overview(AppModel app)
    {
        var hero = new Region(() => Hero(app)).On(app.StatusChanged);
        var banners = new Region(() => Banners(app)).On(app.StatusChanged);
        var remote = new Region(() => RemoteSection(app)).On(app.StatusChanged);
        var sessions = new Region(() => SessionsSection(app)).On(app.StatusChanged);
        return Frame(W.Title(L.T("page.overview")), hero, banners, remote, sessions);
    }

    private static FrameworkElement Hero(AppModel app)
    {
        var s = app.Status!;
        var o = States.Of(app);
        var color = States.Color(o);
        var icon = o switch
        {
            Overall.Online => "check",
            Overall.Controlled => "display",
            Overall.Ai => "sparkles",
            Overall.Disabled => "ban",
            Overall.NoAccount => "user",
            _ => "hourglass",
        };
        var badge = new Grid { Width = 40, Height = 40, Children = { new System.Windows.Shapes.Ellipse { Fill = Th.B(color, 0.16) }, K.Center(K.Icon(icon, 18, Th.B(color))) } };
        return K.Card(K.Col(0,
            K.Pad(K.Row(12, badge, K.Grow(K.Col(2, K.Semi(States.Text(o), 15), K.Text(s.Hostname + " · " + AppInfo.OsName, 12, Th.Secondary)))), 16, 14, 16, 14),
            K.HLine(),
            W.Row(L.T("overview.deviceId"), K.Row(4, W.Value(Fmt.GroupId(s.DeviceId), mono: true), W.CopyButton(() => s.DeviceId)), L.T("overview.deviceId.info"))), padding: 0);
    }

    private static FrameworkElement Banners(AppModel app)
    {
        var s = app.Status!;
        var list = new List<UIElement?>();
        if (s.AccountCount == 0 && s.Rights.CanManage)
            list.Add(W.Banner("info", Th.P.Info, L.T("overview.noAccount"), W.Secondary(L.T("accounts.signIn"), () => Login.Open(app), shield: s.Rights.NeedsElevation)));
        if (s.NeedsRelogin > 0)
            list.Add(W.Banner("alertCircle", Th.P.Warning, L.T("overview.relogin"), K.SecondaryButton(L.T("overview.viewAccounts"), () => app.Go(Page.Accounts))));
        if (!AppInfo.RunningInstalled)
        {
            var inst = AppInfo.InstalledVersion;
            if (inst != AppInfo.Version)
                list.Add(W.Banner("download", Th.P.Accent, inst == null ? L.T("overview.notInstalled") : L.T("overview.newerHere", AppInfo.Version, inst),
                    W.Secondary(inst == null ? L.T("setup.install") : L.T("setup.upgrade"), () => Ui.Run(async () =>
                    {
                        if (await app.RunElevated("install", "--install")) Program.LaunchInstalledAndExit();
                    }), shield: true)));
        }
        if (s.Service.Mode == "foreground")
            list.Add(W.Banner("info", Th.P.Info, L.T("overview.foreground", s.Service.User)));
        return list.Count == 0 ? new Border() : K.Col(10, list);
    }

    private static FrameworkElement RemoteSection(AppModel app)
    {
        var s = app.Status!;
        var r = s.Rights;
        var allow = W.Switch(s.RemoteEnabled, async v => await app.Do(Req.SetRemoteEnabled(v)) != null, r.CanManage || (r.Operate && s.RemoteEnabled), shield: !s.RemoteEnabled && r.NeedsElevation);
        return K.Col(8,
            W.Section(L.T("overview.remote")),
            W.Rows(
                W.Row(L.T("overview.allowRemote"), allow, L.T("overview.allowRemote.info")),
                W.Row(L.T("overview.server"), W.Value(s.Server.Length == 0 ? L.T("common.notSet") : ServerHost(s.Server)), L.T("overview.server.info")),
                W.Row(L.T("overview.publicIp"), W.Value(s.PublicIp.Length == 0 ? "—" : s.PublicIp, mono: true)),
                W.Row(L.T("overview.accounts"), W.Value(s.AccountCount.ToString())),
                W.Row(L.T("overview.version"), W.Value(s.Version))));
    }

    /// <summary>服务器地址只显示「主机:端口」，指纹太长（复制、详情在账号页）</summary>
    public static string ServerHost(string server)
    {
        var i = server.IndexOf('#');
        return i > 0 ? server.Substring(0, i) : server;
    }

    private static FrameworkElement SessionsSection(AppModel app)
    {
        var s = app.Status!;
        var r = s.Rights;
        var stopAi = s.AiActive ? K.TintButton(L.T("overview.stopAi"), () => Ui.Run(async () =>
        {
            if (await app.Do(Req.StopAi()) != null) app.ShowToast(L.T("overview.aiStopped"));
        }), Th.P.Warning, "hand") : null;
        var all = s.Sessions.Count > 1 ? K.TintButton(L.T("overview.disconnectAll"), () => Ui.Run(async () => await app.Do(Req.DisconnectAll())), Th.P.Danger) : null;
        if (stopAi != null) stopAi.IsEnabled = r.Operate;
        if (all != null) all.IsEnabled = r.Operate;
        var rows = s.Sessions.Count == 0
            ? new List<UIElement?> { K.Pad(K.Text(L.T("overview.noSessions"), 12.5, Th.Tertiary), 14, 14, 14, 14) }
            : s.Sessions.Select(x => (UIElement?)SessionRow(app, x, r.Operate)).ToList();
        return K.Col(8, W.Section(L.T("overview.sessions"), null, stopAi, all), W.Rows(rows));
    }

    private static FrameworkElement SessionRow(AppModel app, Session x, bool operate)
    {
        var icon = new Grid { Width = 30, Height = 30, Children = { new System.Windows.Shapes.Ellipse { Fill = Th.B(x.AiActive ? Th.P.Warning : Th.P.Accent, 0.15) }, K.Center(K.Icon(x.AiActive ? "sparkles" : "user", 15, Th.B(x.AiActive ? Th.P.Warning : Th.P.Accent))) } };
        var mode = x.Assist ? L.T("session.assist") : L.T("session.control");
        var sub = $"{mode} · {L.T("session.device")} {Fmt.ShortDevice(x.FromDevice)} · {Fmt.PathText(x.Path)} · {L.T("session.since", Fmt.Time(x.StartedAt))}";
        var btn = K.TintButton(L.T("session.disconnect"), () => Ui.Run(async () => await app.Do(Req.Disconnect(x.Id))), Th.P.Danger);
        btn.IsEnabled = operate;
        return W.Row(Fmt.Who(x.FromUser), K.Row(8, x.AiActive ? K.Badge("AI", Th.P.Warning) : null, btn), subtitle: sub, icon: icon);
    }
}
