using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using LemonGalaxy.Ipc;
using Forms = System.Windows.Forms;

namespace LemonGalaxy.UI;

/// <summary>
/// 托盘图标：状态（在线 / 被控 / AI / 禁止远程 / 没连上服务）画在标志右下角的小圆点上，矢量现画，跟随 DPI；
/// 右键是系统原生菜单（与 Windows 自己的托盘菜单同样外观）；双击打开主窗口；有人连入、请求协助时发系统通知。
/// </summary>
internal sealed class Tray : IDisposable
{
    public static Tray? Instance;

    private readonly AppModel app;
    private readonly MainWindow window;
    private readonly Forms.NotifyIcon icon;
    private IntPtr hicon;
    private (Overall, bool)? lastLook;
    private Action? balloonAction;

    public Tray(AppModel app, MainWindow window)
    {
        this.app = app;
        this.window = window;
        Instance = this;
        icon = new Forms.NotifyIcon { Visible = false };
        var menu = new Forms.ContextMenu();
        menu.Popup += (_, _) => BuildMenu(menu);
        icon.ContextMenu = menu;
        icon.DoubleClick += (_, _) => window.ShowPage();
        icon.MouseClick += (_, e) =>
        {
            if (e.Button == Forms.MouseButtons.Left) window.ShowPage();
        };
        icon.BalloonTipClicked += (_, _) =>
        {
            var a = balloonAction;
            balloonAction = null;
            if (a != null) a();
            else window.ShowPage();
        };
        app.StatusChanged.Watch(this, Update);
        app.SessionStarted += OnSessionStarted;
        app.AssistRequested += (id, user, device) =>
        {
            AssistAskWindow.Ask(app, id, user, device);
            Balloon(L.T("assist.request.title"), L.T("assist.request.body", Fmt.Who(user), Fmt.ShortDevice(device)), () => window.ShowPage(Page.Assist), Forms.ToolTipIcon.Warning);
        };
        app.AssistRequestClosed += AssistAskWindow.Dismiss;
        app.AllAssistRequestsClosed += AssistAskWindow.CloseAll;
        Update();
        icon.Visible = true;
    }

    private void OnSessionStarted(Session s)
    {
        if (!s.Notify || !app.Settings.Notifications) return;
        Balloon(L.T("notify.connected.title"), L.T("notify.connected.body", Fmt.Who(s.FromUser), Fmt.ShortDevice(s.FromDevice)), () => window.ShowPage(Page.Overview));
    }

    public void Balloon(string title, string text, Action? onClick, Forms.ToolTipIcon kind = Forms.ToolTipIcon.None)
    {
        balloonAction = onClick;
        try { icon.ShowBalloonTip(6000, title, text, kind); }
        catch (Exception e) { Log.Error("托盘通知失败：" + e.Message); }
    }

    private void Update()
    {
        var o = States.Of(app);
        var tip = L.T("app.name") + " · " + TipText(o);
        icon.Text = tip.Length > 63 ? tip.Substring(0, 62) + "…" : tip;
        var look = (o, Th.P.IsDark);
        if (lastLook == look) return;
        lastLook = look;
        SetIcon(o);
    }

    private string TipText(Overall o)
    {
        var s = app.Status;
        if (s is { Sessions.Count: 1 }) return L.T("tray.controlledBy", Fmt.Who(s.Sessions[0].FromUser));
        if (s is { Sessions.Count: > 1 }) return L.T("tray.controlledByMany", s.Sessions.Count);
        return States.Text(o);
    }

    private void BuildMenu(Forms.ContextMenu menu)
    {
        menu.MenuItems.Clear();
        var s = app.Status;
        Forms.MenuItem Item(string text, Action a, bool enabled = true, bool check = false)
        {
            var m = new Forms.MenuItem(text, (_, _) => a()) { Enabled = enabled, Checked = check };
            menu.MenuItems.Add(m);
            return m;
        }
        void Sep() => menu.MenuItems.Add("-");
        menu.MenuItems.Add(new Forms.MenuItem(L.T("app.name") + " · " + TipText(States.Of(app))) { Enabled = false });
        if (s != null)
        {
            var r = s.Rights;
            if (s.Sessions.Count > 0)
            {
                Sep();
                foreach (var x in s.Sessions)
                {
                    var id = x.Id;
                    Item(L.T("tray.disconnectOne", Fmt.Who(x.FromUser)), () => Ui.Run(async () => await app.Do(Req.Disconnect(id))), r.Operate);
                }
                if (s.Sessions.Count > 1) Item(L.T("overview.disconnectAll"), () => Ui.Run(async () => await app.Do(Req.DisconnectAll())), r.Operate);
                if (s.AiActive) Item(L.T("overview.stopAi"), () => Ui.Run(async () => await app.Do(Req.StopAi())), r.Operate);
            }
            Sep();
            Item(L.T("overview.allowRemote"), () => Ui.Run(async () => await app.Do(Req.SetRemoteEnabled(!s.RemoteEnabled))), r.CanManage || (r.Operate && s.RemoteEnabled), s.RemoteEnabled);
            if (s.Assist is { Mine: true } a) Item(L.T("tray.assistCodes", a.DeviceCode, a.Code), () => { SysClipboard.SetText(a.DeviceCode.Replace(" ", "") + " " + a.Code); });
            if (s.Assist != null) Item(L.T("assist.stop"), () => Ui.Run(async () => await app.Do(Req.AssistStop())), r.Operate);
            else Item(L.T("tray.startAssist"), () => window.ShowPage(Page.Assist), r.Operate);
        }
        Sep();
        var open = Item(L.T("tray.open"), () => window.ShowPage());
        open.DefaultItem = true;
        Item(L.T("tray.quit"), Program.Exit);
    }

    // ---------------- 图标 ----------------

    [DllImport("user32.dll")]
    private static extern bool DestroyIcon(IntPtr h);

    private void SetIcon(Overall o)
    {
        try
        {
            var size = Forms.SystemInformation.SmallIconSize.Width;
            var png = Render(o, size);
            using var bmp = new System.Drawing.Bitmap(new MemoryStream(png));
            var h = bmp.GetHicon();
            var old = hicon;
            icon.Icon = System.Drawing.Icon.FromHandle(h);
            hicon = h;
            if (old != IntPtr.Zero) DestroyIcon(old);
        }
        catch (Exception e) { Log.Error("托盘图标失败：" + e.Message); }
    }

    /// <summary>托盘用的标志：小尺寸下线条加粗；没连上服务时整体变灰；状态点在右下角</summary>
    public static byte[] Render(Overall o, int size)
    {
        var dv = new DrawingVisual();
        using (var dc = dv.RenderOpen())
        {
            var gray = o is Overall.NotRunning or Overall.Connecting;
            var s = (double)size;
            var bg = gray ? new LinearGradientBrush(Palettes.Hex("#4A5058"), Palettes.Hex("#2E3238"), 90) : new LinearGradientBrush(Palettes.Hex("#15363A"), Palettes.Hex("#0A1719"), 90);
            dc.DrawRoundedRectangle(bg, null, new Rect(0, 0, s, s), s * 0.22, s * 0.22);
            var pad = s * 0.17;
            var inner = s - pad * 2;
            var c = new Point(s / 2, s / 2);
            var geo = new PathGeometry();
            void Arc(double r, double from, double to)
            {
                Point P(double deg) => new(c.X + r * Math.Cos(deg * Math.PI / 180), c.Y + r * Math.Sin(deg * Math.PI / 180));
                var span = ((to - from) % 360 + 360) % 360;
                var f = new PathFigure { StartPoint = P(from), IsClosed = false };
                f.Segments.Add(new ArcSegment(P(to), new Size(r, r), 0, span > 180, SweepDirection.Clockwise, true));
                geo.Figures.Add(f);
            }
            Arc(inner * 0.5, 205, 335);
            Arc(inner * 0.5, 25, 155);
            var stroke = gray ? (Brush)Th.Hex("#B8BEC6") : new LinearGradientBrush(Palettes.Hex("#F5D90A"), Palettes.Hex("#7BD35A"), new Point(0, 0), new Point(1, 1));
            dc.DrawGeometry(null, new Pen(stroke, Math.Max(1.4, s * 0.1)) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round }, geo);
            dc.DrawEllipse(gray ? Th.Hex("#B8BEC6") : Th.Hex("#F5D90A"), null, c, s * 0.12, s * 0.12);
            Color? dot = o switch
            {
                Overall.Online => Palettes.Hex("#34C77B"),
                Overall.Controlled or Overall.Ai => Palettes.Hex("#F0A43A"),
                Overall.Disabled or Overall.NotRunning => Palettes.Hex("#F2555A"),
                _ => null,
            };
            if (dot is { } d)
            {
                var r = s * 0.2;
                var p = new Point(s - r - 0.5, s - r - 0.5);
                dc.DrawEllipse(new SolidColorBrush(d), new Pen(new SolidColorBrush(Palettes.Hex("#0A1719")), Math.Max(1, s * 0.06)), p, r, r);
            }
        }
        var rtb = new RenderTargetBitmap(size, size, 96, 96, PixelFormats.Pbgra32);
        rtb.Render(dv);
        var enc = new PngBitmapEncoder();
        enc.Frames.Add(BitmapFrame.Create(rtb));
        using var ms = new MemoryStream();
        enc.Save(ms);
        return ms.ToArray();
    }

    public void Dispose()
    {
        icon.Visible = false;
        icon.Dispose();
        if (hicon != IntPtr.Zero) DestroyIcon(hicon);
        hicon = IntPtr.Zero;
    }
}
