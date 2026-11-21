using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.ServiceProcess;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace LemonGalaxy.UI;

internal enum WorkerState { Unknown, NotInstalled, Stopped, Starting, Running }

/// <summary>
/// 本机被控：Windows 上由同目录附带的 lemon-galaxy-worker.exe 作为系统服务（LemonGalaxyWorker）负责（spec 3.0），
/// 不用内核自带的被控服务。这里只查服务状态，并以管理员身份打开控制台执行 login / install-service / uninstall-service。
/// </summary>
internal sealed class WorkerServiceModel
{
    public const string ServiceName = "LemonGalaxyWorker";
    private readonly AppModel app;
    public readonly Observable Changed = new();
    public WorkerState State = WorkerState.Unknown;
    public string? Error;
    public DateTime CheckedAt;
    private System.Windows.Threading.DispatcherTimer? timer;

    public WorkerServiceModel(AppModel app)
    {
        this.app = app;
        if (app.SelfTest) return;
        Refresh();
        timer = Ui.Every(3000, Refresh);
    }

    public bool WorkerPresent => File.Exists(AppInfo.WorkerExe);

    /// <summary>selftest 用：指定显示的状态</summary>
    public void Fake(WorkerState s)
    {
        State = s;
        CheckedAt = Fmt.Local(Clock.NowMs);
        Changed.Notify();
    }

    public void Refresh()
    {
        var old = State;
        try
        {
            using var sc = new ServiceController(ServiceName);
            State = sc.Status switch
            {
                ServiceControllerStatus.Running => WorkerState.Running,
                ServiceControllerStatus.StartPending or ServiceControllerStatus.ContinuePending => WorkerState.Starting,
                _ => WorkerState.Stopped,
            };
            Error = null;
        }
        catch (InvalidOperationException)
        {
            // 服务不存在
            State = WorkerState.NotInstalled;
            Error = null;
        }
        catch (Exception e)
        {
            State = WorkerState.Unknown;
            Error = e.Message;
        }
        CheckedAt = DateTime.Now;
        if (old != State) Changed.Notify();
    }

    private static string Q(string s) => "\"" + s.Replace("\"", "") + "\"";

    /// <summary>以管理员身份打开控制台：先用当前服务器与账号登录（用户在控制台里输密码），再安装系统服务</summary>
    public void Enable()
    {
        if (!WorkerPresent) { app.ShowToast(L.T("local.win.noWorker"), true); return; }
        var server = ServerAddress.Parse(app.Server?.Address ?? "") != null ? app.Server!.Address : app.Settings.LastServer;
        var user = app.User?.Username ?? app.Settings.LastUsername;
        var w = Q(AppInfo.WorkerExe);
        var title = "LemonGalaxy";
        var cmd = $"title {title} & {w} login --server {Q(server)} --user {Q(user)} && {w} install-service & echo. & pause";
        RunElevated(cmd);
    }

    public void Disable()
    {
        if (!WorkerPresent) { app.ShowToast(L.T("local.win.noWorker"), true); return; }
        RunElevated($"title LemonGalaxy & {Q(AppInfo.WorkerExe)} uninstall-service & echo. & pause");
    }

    private void RunElevated(string command)
    {
        try
        {
            // cmd /c "…"：外层引号在执行时被去掉，里面的路径引号保持原样
            Process.Start(new ProcessStartInfo("cmd.exe", "/c \"" + command + "\"")
            {
                UseShellExecute = true, Verb = "runas", WorkingDirectory = AppInfo.BaseDir,
            });
            Ui.After(1500, Refresh);
        }
        catch (Win32Exception e) when (e.NativeErrorCode == 1223)
        {
            // 用户在 UAC 弹窗里点了「否」
            app.ShowToast(L.T("common.cancelled"));
        }
        catch (Exception e) { app.ShowError(e); }
    }
}

internal static class LocalViews
{
    public static FrameworkElement Middle(AppModel app)
    {
        var w = app.Worker;
        var card = new Region(() =>
        {
            var (text, color) = StateText(w.State);
            return new Border
            {
                Margin = new Thickness(12), Padding = new Thickness(12), Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(9),
                Child = K.Row(10, K.StatusDot(w.State == WorkerState.Running ? DotState.Online : w.State is WorkerState.Stopped or WorkerState.Starting ? DotState.Busy : DotState.Offline, 8, Th.Card),
                    K.Grow(K.Col(2, K.Text(L.T("local.win.service"), 13, Th.Text, FontWeights.Medium), K.Text(text, 11.5, Th.B(color))))),
            };
        }).On(w.Changed);
        var nav = new Btn
        {
            Height = 34, CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 0, 10, 0), Normal = app.ActiveTabId == null ? Th.Selection : Brushes.Transparent, HoverBrush = Th.Hover,
            Child = K.Row(10, K.Icon("radio", 15, Th.Accent), K.Text(L.T("local.page.overview"), 13.5, Th.Text, FontWeights.SemiBold)),
        };
        nav.UpdateLook();
        nav.Click += () => app.ActivateTab(null);
        var dock = new DockPanel();
        var top = K.Col(0, Shell.ColumnHeader(L.T("nav.local")), K.HLine(), K.Margin(nav, 8, 8, 8, 0));
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        DockPanel.SetDock(card, Dock.Bottom);
        dock.Children.Add(card);
        dock.Children.Add(new Border());
        return dock;
    }

    private static (string text, Color color) StateText(WorkerState s) => s switch
    {
        WorkerState.Running => (L.T("local.win.running"), Th.P.Success),
        WorkerState.Starting => (L.T("local.win.starting"), Th.P.Warning),
        WorkerState.Stopped => (L.T("local.win.stopped"), Th.P.Warning),
        WorkerState.NotInstalled => (L.T("local.win.notInstalled"), Th.P.TertiaryText),
        _ => (L.T("local.win.unknown"), Th.P.TertiaryText),
    };

    public static FrameworkElement Home(AppModel app) => new Region(() => Page(app)).On(app.Worker.Changed, app.Data);

    private static FrameworkElement Page(AppModel app)
    {
        var w = app.Worker;
        var (text, color) = StateText(w.State);
        var installed = w.State is WorkerState.Running or WorkerState.Stopped or WorkerState.Starting;
        var col = new List<UIElement?>
        {
            K.Row(8, K.Semi(L.T("local.page.overview"), 21), K.Info(L.T("local.win.info")), K.Spacer(), K.IconButton("refresh", L.T("common.refresh"), w.Refresh, size: 14)),
        };
        if (!w.WorkerPresent && !app.SelfTest)
            col.Add(K.Box(K.Row(10, K.Icon("warning", 16, Th.Warning), K.Grow(K.Text(L.T("local.win.noWorker"), 13, Th.Text, wrap: true))),
                Th.B(Th.P.Warning, 0.1), 9, Th.B(Th.P.Warning, 0.35), 1, new Thickness(12)));
        // 状态卡片
        FrameworkElement Stat(string icon, string title, string value, Brush valueColor)
            => K.Grow(K.Card(K.Row(12, new Border { Width = 36, Height = 36, CornerRadius = new CornerRadius(9), Background = Th.Elevated, Child = K.Center(K.Icon(icon, 17)) },
                K.Grow(K.Col(3, K.Text(title, 12, Th.Secondary), K.Text(value, 14, valueColor, FontWeights.SemiBold)))), 14));
        col.Add(K.Row(12,
            Stat("power", L.T("local.win.service"), text, Th.B(color)),
            Stat("server", L.T("local.server"), app.Server?.Name ?? "—", Th.Text),
            Stat("user", L.T("local.account"), app.User?.Username ?? "—", Th.Text)));
        // 开启 / 关闭
        var action = installed
            ? K.SecondaryButton(L.T("local.win.disable"), () => app.Confirm(L.T("local.win.disable"), L.T("local.win.disable.message"), L.T("local.win.disable"), w.Disable), destructive: true, icon: "power")
            : K.PrimaryButton(L.T("local.win.enable"), w.Enable, icon: "shieldCheck");
        action.IsEnabled = w.WorkerPresent || app.SelfTest;
        col.Add(K.Card(K.Col(14,
            K.Row(10, K.Grow(K.Col(4, K.Semi(installed ? L.T("local.win.enabledTitle") : L.T("local.win.enableTitle"), 14),
                K.Text(installed ? L.T("local.win.enabledHint") : L.T("local.win.enableHint"), 12.5, Th.Secondary, wrap: true))), action),
            K.HLine(),
            Steps(app, installed))));
        col.Add(K.SectionLabel(L.T("local.win.features")));
        var grid = new AdaptiveGrid { MinItem = 220, Spacing = 10 };
        foreach (var (icon, title, key) in new[]
        {
            ("display", "local.feature.display", "local.win.f.display"), ("lockScreen", "local.win.f.lock", "local.win.f.lock.info"),
            ("terminal", "local.feature.terminal", "local.win.f.terminal"), ("folder", "local.feature.files", "local.win.f.files"),
            ("users", "local.win.f.account", "local.win.f.account.info"), ("sparkles", "local.event.ai", "local.win.f.ai"),
        })
            grid.Children.Add(K.Card(K.Row(10, K.Top(K.Icon(icon, 16, Th.Accent)), K.Grow(K.Col(3, K.Text(L.T(title), 13, Th.Text, FontWeights.Medium), K.Text(L.T(key), 12, Th.Secondary, wrap: true)))), 12));
        col.Add(grid);
        col.Add(K.Text(L.T("local.win.checked", w.CheckedAt.ToString("HH:mm:ss")), 11.5, Th.Tertiary));
        return K.PageScroll(K.Col(18, col), 860);
    }

    private static FrameworkElement Steps(AppModel app, bool installed)
    {
        var w = IOPath.GetFileName(AppInfo.WorkerExe);
        var server = app.Server?.Address ?? app.Settings.LastServer;
        var user = app.User?.Username ?? "";
        string[] lines = installed
            ? new[] { $"{w} uninstall-service" }
            : new[] { $"{w} login --server \"{server}\" --user \"{user}\"", $"{w} install-service" };
        var code = K.Box(K.SelectableText(string.Join("\n", lines), 12, Th.Text, mono: true, wrap: true), Th.CodeBg, 7, null, 0, new Thickness(10, 8, 10, 8));
        return K.Col(8, K.Row(5, K.Text(L.T("local.win.commands"), 12.5, Th.Secondary, FontWeights.Medium), K.Info(L.T("local.win.commands.info"))), code);
    }
}
