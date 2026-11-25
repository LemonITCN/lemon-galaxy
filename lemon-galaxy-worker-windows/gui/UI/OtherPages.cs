using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using LemonGalaxy.Ipc;
using LemonGalaxy.Setup;

namespace LemonGalaxy.UI;

internal static partial class Pages
{
    // ---------------- 设置 ----------------

    private static readonly string[] Sizes = { "1280x720", "1600x900", "1920x1080", "2560x1440", "3840x2160" };

    private static FrameworkElement SettingsPage(AppModel app)
    {
        var service = new Region(() => ServiceSettings(app)).On(app.StatusChanged, app.WorkerChanged).Keyed(() => (app.Worker, app.Rights));
        var update = new Region(() => UpdateSection(app)).On(app.StatusChanged).Keyed(() => (app.Rights, app.Status?.Version));
        return Frame(W.Title(L.T("page.settings")), service, update, PersonalSettings(app));
    }

    private static FrameworkElement ServiceSettings(AppModel app)
    {
        var r = app.Rights;
        var tray = W.Switch(app.Settings.TrayAtLogin, v => { Installer.SetUserTray(v); return Task.FromResult(true); });
        var trayRow = W.Row(L.T("settings.tray"), tray, L.T("settings.tray.info"));
        if (!r.ViewConfig)
            return K.Col(8, W.Section(L.T("settings.startup")), W.Rows(trayRow), W.Banner("lock", Th.P.TertiaryText, L.T("denied.settings")));
        var w = app.Worker;
        if (w == null) return K.Col(8, W.Section(L.T("settings.startup")), W.Rows(trayRow), W.Busy(L.T("common.loading")));
        var m = r.CanManage;
        var boot = W.ReqSwitch(app, w.Autostart.Enabled, Req.SetAutostart, m && w.Autostart.Available);
        var lockEnd = W.ReqSwitch(app, w.LockOnEnd, Req.SetLockOnEnd, m);
        var au = W.ReqSwitch(app, w.AutoUpdate, Req.SetAutoUpdate, m);

        // 无头兜底屏：开关 + 分辨率
        var cur = w.HeadlessDisplay?.Id ?? "1920x1080";
        var sizes = Sizes.Contains(cur) ? Sizes.ToList() : Sizes.Concat(new[] { cur }).ToList();
        var picked = cur;
        var dd = K.Dropdown(sizes.Select(x => (x, Fmt.Size(x))), cur, v =>
        {
            picked = v;
            if (w.HeadlessDisplay != null) Ui.Run(async () => await app.Do(Req.SetHeadlessDisplay(v)));
        }, minWidth: 150);
        dd.IsEnabled = m && w.HeadlessDisplay != null;
        var vd = W.Switch(w.HeadlessDisplay != null, async on =>
        {
            var ok = await app.Do(Req.SetHeadlessDisplay(on ? picked : "off")) != null;
            if (ok) dd.IsEnabled = on;
            return ok;
        }, m, r.NeedsElevation);

        return K.Col(8,
            W.Section(L.T("settings.startup")),
            W.Rows(
                W.Row(L.T("settings.boot"), boot, L.T("settings.boot.info"), w.Autostart.Available ? null : w.Autostart.Detail),
                trayRow),
            K.Gap(6),
            W.Section(L.T("settings.security")),
            W.Rows(W.Row(L.T("settings.lockOnEnd"), lockEnd, L.T("settings.lockOnEnd.info"))),
            K.Gap(6),
            W.Section(L.T("settings.display")),
            W.Rows(W.Row(L.T("settings.headless"), K.Row(10, dd, vd), L.T("settings.headless.info"))),
            K.Gap(6),
            W.Section(L.T("settings.updates")),
            W.Rows(W.Row(L.T("settings.autoUpdate"), au, L.T("settings.autoUpdate.info"))));
    }

    /// <summary>版本 + 检查更新 / 立即更新（服务按发布签名下载新版被控服务，装好后自己重启）</summary>
    private static FrameworkElement UpdateSection(AppModel app)
    {
        var r = app.Rights;
        if (!r.ViewConfig) return new Border();
        var sub = K.Text(L.T("settings.versions", app.Status?.Version ?? "", AppInfo.Version), 12, Th.Secondary, wrap: true);
        var right = new Border();
        Btn? check = null;
        void Idle(string? text, string? latest)
        {
            if (text != null) sub.Text = text;
            check = K.SecondaryButton(L.T("settings.checkUpdate"), Check);
            var now = latest == null ? null : W.Primary(L.T("settings.updateNow"), () => Apply(latest), shield: r.NeedsElevation, enabled: r.CanManage);
            right.Child = K.Row(8, check, now);
        }
        void Check() => Ui.Run(async () =>
        {
            right.Child = W.Busy(L.T("settings.checking"));
            var x = await app.Call(Req.CheckUpdate());
            if (x.Type == "update_check")
            {
                var latest = x.J.O("latest");
                if (latest == null) Idle(L.T("settings.upToDate", x.J.S("current")), null);
                else Idle(L.T("settings.available", latest.S("version"), x.J.S("current")), latest.S("version"));
            }
            else Idle(L.T("settings.checkFailed", x.Message), null);
        });
        void Apply(string version) => Ui.Run(async () =>
        {
            right.Child = W.Busy(L.T("settings.downloading"));
            var x = await app.Call(Req.ApplyUpdate());
            if (x.Type == "updated")
            {
                var v = x.J.SOpt("version");
                Idle(v == null ? L.T("settings.upToDate", app.Status?.Version ?? "") : x.J.B("restarting") ? L.T("settings.updatedRestarting", v) : L.T("settings.updated", v), null);
            }
            else Idle(L.T("settings.updateFailed", x.Message), version);
        });
        Idle(null, null);
        var row = K.Pad(K.Row(12, K.Grow(K.Col(2, K.Row(6, K.Text(L.T("settings.version"), 13), K.Info(L.T("settings.version.info"))), sub)), right), 14, 11, 14, 11);
        return W.Rows(row);
    }

    /// <summary>只影响自己的设置：通知、语言、外观</summary>
    private static FrameworkElement PersonalSettings(AppModel app)
    {
        var s = app.Settings;
        var notify = W.Switch(s.Notifications, v =>
        {
            s.Notifications = v;
            s.Save();
            return Task.FromResult(true);
        });
        var langs = new List<(string, string)> { ("system", L.T("settings.language.system")) };
        langs.AddRange(L.Packs.Select(p => (p.Code, p.Name)));
        var lang = K.Dropdown(langs, s.Language, v => s.SetLook(language: v), minWidth: 150);
        var look = K.Segmented(new[] { ("system", L.T("settings.appearance.system")), ("light", L.T("settings.appearance.light")), ("dark", L.T("settings.appearance.dark")) }, s.Appearance, v => s.SetLook(appearance: v));
        return K.Col(8,
            K.Gap(6),
            W.Section(L.T("settings.personal"), L.T("settings.personal.info")),
            W.Rows(
                W.Row(L.T("settings.notifications"), notify, L.T("settings.notifications.info")),
                W.Row(L.T("settings.language"), lang),
                W.Row(L.T("settings.appearance"), look)));
    }

    // ---------------- 事件日志 ----------------

    private static bool warningsOnly;

    private static FrameworkElement EventsPage(AppModel app)
        => new Region(() =>
        {
            if (!app.Rights.ViewConfig) return W.Placeholder("lock", L.T("denied.title"), L.T("denied.events"));
            var only = K.CheckBox(L.T("events.warnings"), warningsOnly, v =>
            {
                warningsOnly = v;
                app.EventsChanged.Notify();
            });
            var refresh = K.IconButton("refresh", L.T("common.refresh"), app.ReloadEvents);
            var list = app.Events;
            List<UIElement?> rows;
            if (list == null) rows = new() { K.Pad(W.Busy(L.T("common.loading")), 14) };
            else
            {
                var shown = list.AsEnumerable().Reverse().Where(e => !warningsOnly || e.Warning).ToList();
                rows = shown.Count == 0 ? new() { K.Pad(K.Text(L.T("events.empty"), 12.5, Th.Tertiary), 14, 14, 14, 14) } : shown.Select(e => (UIElement?)EventRow(e)).ToList();
            }
            return Frame(W.Title(L.T("page.events"), L.T("events.info"), only, refresh), W.Rows(rows));
        }).On(app.StatusChanged, app.EventsChanged).Keyed(() => (app.Events, app.Rights.ViewConfig, warningsOnly));

    private static FrameworkElement EventRow(LogEntry e)
    {
        var kind = K.Badge(Fmt.Kind(e.Kind), e.Warning ? Th.P.Danger : Th.P.SecondaryText);
        var time = K.Text(Fmt.Time(e.At, true), 11.5, Th.Tertiary, mono: true);
        time.Width = 130;
        time.VerticalAlignment = VerticalAlignment.Top;
        var head = K.Row(6, kind, e.Ai ? K.Badge("AI", Th.P.Warning) : null, e.User.Length == 0 ? null : K.Text(e.User, 12.5));
        var detail = e.Detail.Length == 0 ? null : K.Text(e.Detail, 12, Th.Secondary, wrap: true);
        return K.Pad(K.Row(12, time, K.Grow(K.Col(3, head, detail))), 14, 9, 14, 9);
    }

    // ---------------- 关于 ----------------

    private static FrameworkElement AboutPage(AppModel app)
        => new Region(() =>
        {
            var s = app.Status;
            string mode = s?.Service.Mode switch
            {
                "system" => L.T("about.mode.system"),
                "user" => L.T("about.mode.user"),
                "foreground" => L.T("about.mode.foreground"),
                _ => "—",
            };
            var head = K.Col(6, K.Center(Shell.AppMark(72)), K.Center(K.Semi(L.T("app.name"), 18)), K.Center(K.Text(L.T("about.tagline"), 12.5, Th.Secondary)));
            var rows = W.Rows(
                W.Row(L.T("about.appVersion"), W.Value(AppInfo.Version)),
                W.Row(L.T("about.serviceVersion"), W.Value(s?.Version ?? "—")),
                W.Row(L.T("about.runsAs"), W.Value(s == null ? "—" : $"{mode}（{s.Service.User}）")),
                W.Row(L.T("about.dataDir"), W.Value(s?.DataDir ?? "—")),
                W.Row(L.T("about.pipe"), W.Value(app.Client?.PipeName is { } p ? @"\\.\pipe\" + p : "—", mono: true)),
                W.Row(L.T("about.installDir"), W.Value(AppInfo.BaseDir.TrimEnd('\\'))),
                W.Row(L.T("about.system"), W.Value(AppInfo.OsName)));
            var links = K.Row(16, K.Spacer(),
                K.LinkButton(L.T("about.source"), () => OpenUrl(AppInfo.RepoUrl), Th.Accent, 12.5),
                K.LinkButton(L.T("about.license"), () => OpenUrl(AppInfo.LicenseUrl), Th.Accent, 12.5), K.Spacer());
            var uninstall = AppInfo.RunningInstalled || AppInfo.InstalledVersion != null
                ? K.Col(8, W.Section(L.T("about.uninstall")), W.Rows(W.Row(L.T("about.uninstall.row"), W.Secondary(L.T("about.uninstall.button"), () => Ui.Run(async () =>
                {
                    if (!await app.Confirm(L.T("about.uninstall.title"), L.T("about.uninstall.body", AppInfo.ServiceDataDir), L.T("about.uninstall.button"), true)) return;
                    if (!await app.RunElevated("uninstall", "--uninstall", "--confirmed", "--keep", Process.GetCurrentProcess().Id.ToString())) return;
                    MessageBox.Show(L.T("about.uninstall.done"), L.T("app.name"), MessageBoxButton.OK, MessageBoxImage.Information);
                    Program.Exit();
                }), shield: true, destructive: true), L.T("about.uninstall.info"))))
                : null;
            return Frame(head, rows, links, uninstall);
        }).On(app.StatusChanged).Keyed(() => (app.Status?.Version, app.Status?.Service, app.Client?.PipeName));

    private static void OpenUrl(string url)
    {
        try { Process.Start(new ProcessStartInfo(url) { UseShellExecute = true }); }
        catch (Exception e) { AppModel.Current?.ShowError(e); }
    }
}
