using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using LemonGalaxy.Ipc;

namespace LemonGalaxy.UI;

internal static partial class Pages
{
    // ---------------- 临时协助 ----------------

    private static FrameworkElement AssistPage(AppModel app)
    {
        var server = K.TextField(L.T("login.server.placeholder"), "", mono: true);
        var main = new Region(() => AssistMain(app, server)).On(app.StatusChanged)
            .Keyed(() => (app.Status?.Assist, app.Rights.Operate, app.Status?.Server.Length == 0, app.Busy));
        var fixedCode = new Region(() => FixedCode(app)).On(app.StatusChanged, app.WorkerChanged).Keyed(() => (app.Rights, app.Worker?.AssistCode));
        return Frame(W.Title(L.T("page.assist"), L.T("assist.info")), main, fixedCode);
    }

    private static FrameworkElement AssistMain(AppModel app, Field server)
    {
        var s = app.Status!;
        var a = s.Assist;
        if (a is { Mine: true })
        {
            FrameworkElement Code(string title, string value, string copy) => K.Grow(K.Col(6,
                K.Center(K.Text(title, 12, Th.Secondary)),
                K.Center(K.Row(6, K.Text(value, 26, Th.Text, FontWeights.SemiBold, mono: true), W.CopyButton(() => copy)))));
            return K.Col(14,
                K.Card(K.Row(12, Code(L.T("assist.deviceCode"), GroupBy(a.DeviceCode.Replace(" ", ""), 3), a.DeviceCode.Replace(" ", "")), K.VLine(), Code(L.T("assist.code"), a.Code, a.Code)), 22),
                K.Row(8, K.Spinner(13), K.Text(L.T("assist.waiting"), 12.5, Th.Secondary), K.Info(L.T("assist.waiting.info")), K.Spacer(),
                    K.SecondaryButton(L.T("assist.stop"), () => Ui.Run(async () => await app.Do(Req.AssistStop())), destructive: true)));
        }
        if (a != null)
            return K.Card(K.Row(10, K.Icon("info", 15, Th.Info), K.Grow(K.Text(L.T("assist.other"), 12.5, Th.Text, wrap: true)),
                W.Secondary(L.T("assist.stop"), () => Ui.Run(async () => await app.Do(Req.AssistStop())), destructive: true, enabled: s.Rights.Operate)));
        var busy = app.Busy == "assist";
        var start = W.Primary(L.T("assist.start"), () => Ui.Run(async () =>
        {
            app.Busy = "assist";
            app.StatusChanged.Notify();
            try { await app.Do(Req.AssistStart(server.Text)); }
            finally
            {
                app.Busy = null;
                app.StatusChanged.Notify();
            }
        }), enabled: s.Rights.Operate && !busy);
        return K.Col(12,
            s.Server.Length == 0 ? W.Rows(W.Row(L.T("assist.server"), K.Width(server, 320), L.T("assist.server.info"))) : null,
            K.Row(10, start, busy ? W.Busy(L.T("assist.starting")) : null),
            s.Rights.Operate ? null : W.Banner("lock", Th.P.TertiaryText, L.T("assist.denied")));
    }

    private static string GroupBy(string s, int n)
    {
        var parts = new List<string>();
        for (var i = 0; i < s.Length; i += n) parts.Add(s.Substring(i, Math.Min(n, s.Length - i)));
        return string.Join(" ", parts);
    }

    /// <summary>固定验证码：打开时先出输入框，填好点保存才生效；关掉恢复每次随机</summary>
    private static FrameworkElement FixedCode(AppModel app)
    {
        var r = app.Rights;
        if (!r.ViewConfig || app.Worker == null) return new Border();
        var current = app.Worker.AssistCode;
        var field = K.TextField("", current ?? "", mono: true, width: 220);
        var editing = current != null;
        var row = W.Row(L.T("assist.fixedCode.value"), K.Row(8, field, W.Secondary(L.T("common.save"), () => Ui.Run(async () =>
        {
            if (await app.Do(Req.SetAssistCode(field.Text.Trim())) != null) app.ShowToast(L.T("assist.fixedCode.saved"));
        }), shield: r.NeedsElevation, enabled: r.CanManage)));
        row.Visibility = editing ? Visibility.Visible : Visibility.Collapsed;
        var toggle = W.Switch(current != null, async on =>
        {
            if (on)
            {
                row.Visibility = Visibility.Visible;
                field.FocusInput();
                return true;
            }
            var ok = await app.Do(Req.SetAssistCode(null)) != null;
            if (ok) row.Visibility = Visibility.Collapsed;
            return ok;
        }, r.CanManage, r.NeedsElevation);
        return K.Col(8, W.Section(L.T("assist.fixedCode")), W.Rows(W.Row(L.T("assist.fixedCode.use"), toggle, L.T("assist.fixedCode.info")), row));
    }
}

/// <summary>有人请求临时协助：置顶的小窗口（主窗口藏在托盘时也看得到），对方取消或超时自动关掉</summary>
internal sealed class AssistAskWindow : Window
{
    private static readonly Dictionary<long, AssistAskWindow> Open = new();
    private readonly AppModel app;
    private readonly long id;
    private bool answered;

    public static void Ask(AppModel app, long id, string user, string device)
    {
        if (Open.ContainsKey(id)) return;
        var w = new AssistAskWindow(app, id, user, device);
        Open[id] = w;
        w.Show();
        w.Activate();
    }

    public static void Dismiss(long id)
    {
        if (Open.TryGetValue(id, out var w))
        {
            w.answered = true;
            w.Close();
        }
    }

    public static void CloseAll()
    {
        foreach (var w in Open.Values.ToList())
        {
            w.answered = true;
            w.Close();
        }
    }

    private AssistAskWindow(AppModel app, long id, string user, string device)
    {
        this.app = app;
        this.id = id;
        Title = L.T("assist.request.title");
        Width = 440;
        SizeToContent = SizeToContent.Height;
        ResizeMode = ResizeMode.NoResize;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        Topmost = true;
        ShowInTaskbar = true;
        Background = Th.Content;
        if (MainWindow.AppIcon is { } icon) Icon = icon;
        var who = Fmt.Who(user);
        Content = K.Pad(K.Col(16,
            K.Row(12, new Grid { Width = 40, Height = 40, Children = { new System.Windows.Shapes.Ellipse { Fill = Th.B(Th.P.Warning, 0.16) }, K.Center(K.Icon("hand", 18, Th.Warning)) } },
                K.Grow(K.Col(3, K.Row(6, K.Semi(L.T("assist.request.heading"), 15), K.Info(L.T("assist.request.info"))), K.Text(L.T("assist.request.body", who, Fmt.ShortDevice(device)), 12.5, Th.Secondary, wrap: true)))),
            Dialogs.Buttons(K.SecondaryButton(L.T("assist.request.deny"), () => Answer(false)), K.PrimaryButton(L.T("assist.request.allow"), () => Answer(true)))), 20);
        Closing += (_, _) =>
        {
            Open.Remove(id);
            if (!answered) Send(false);
        };
    }

    private void Answer(bool allow)
    {
        answered = true;
        Send(allow);
        Close();
    }

    private void Send(bool allow)
    {
        if (app.Client != null) _ = app.Client.Call(Req.AssistAnswer(id, allow));
    }
}
