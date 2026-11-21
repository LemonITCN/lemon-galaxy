using System.Collections.Concurrent;
using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

internal enum RemotePhase { Connecting, Streaming, Failed }

/// <summary>
/// 远程桌面标签：list_displays → open_display(VideoSink) → 解码线程 H.264 软解 → 界面线程写 WriteableBitmap；
/// 鼠标键盘转 InputEvent（键盘为 USB HID 用法码，坐标为被控屏幕像素）。界面忙时只显示最新一帧，解码跟不上时丢帧并请求关键帧。
/// </summary>
internal sealed class RemoteTab : TabModel
{
    public RemotePhase Phase = RemotePhase.Connecting;
    public string FailReason = "";
    public PeerConnection? Peer;
    public DisplayInfo[] Displays = Array.Empty<DisplayInfo>();
    public uint DisplayId;
    public int StreamW, StreamH;
    public bool FitMode = true;
    public string Quality;
    public int Fps, BitrateKbps;
    public bool ClipboardSync;
    public int MeasuredFps, MeasuredKbps;
    public int FramesDecoded;
    /// <summary>隐私屏已开启</summary>
    public bool Privacy;
    public bool PrivacySupported => Peer?.Info.Capabilities.Contains("privacy") == true;
    public Window? FullscreenWindow;

    /// <summary>画面元素（常驻，视图重建或进入全屏时挂到新父元素上）</summary>
    public readonly Grid VideoHost;
    private readonly Image image;
    private WriteableBitmap? bitmap;
    private ICoreDisplay? channel;
    private SinkAdapter? sink;
    private readonly IFrameDecoder decoder;
    private readonly BlockingCollection<byte[]> queue = new(new ConcurrentQueue<byte[]>());
    private readonly Thread decodeThread;
    private volatile bool closed;
    private volatile bool waitKeyframe;
    private DecodedFrame? latest;
    private readonly object frameLock = new();
    private int frameCount, byteCount;
    private DispatcherTimer? stats, clipTimer;
    private string? lastClipboard;
    private readonly HashSet<uint> pressed = new();
    private bool imeComposing;

    public RemoteTab(AppModel app, string deviceId, string title) : base(app, TabKind.Remote, title)
    {
        DeviceId = deviceId;
        var s = app.Settings;
        Quality = s.DisplayQuality;
        Fps = s.DisplayFps;
        BitrateKbps = s.DisplayBitrateKbps;
        ClipboardSync = s.ClipboardSync;
        decoder = app.Client.CreateDecoder();
        image = new Image { Stretch = Stretch.Uniform, SnapsToDevicePixels = true, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        RenderOptions.SetBitmapScalingMode(image, BitmapScalingMode.Linear);
        VideoHost = new Grid { Background = Brushes.Black, Focusable = true, FocusVisualStyle = null, ClipToBounds = true, Cursor = Cursors.Arrow };
        InputMethod.SetIsInputMethodEnabled(VideoHost, true);
        VideoHost.Children.Add(image);
        HookInput();
        decodeThread = new Thread(DecodeLoop) { IsBackground = true, Name = "LemonGalaxy 画面解码" };
        decodeThread.Start();
    }

    public DisplayInfo? Display => Displays.FirstOrDefault(d => d.Id == DisplayId);
    public Platform Platform => Peer?.Info.Platform ?? App.Device(DeviceId ?? "")?.Platform ?? Platform.Windows;

    public void Start()
    {
        Phase = RemotePhase.Connecting;
        Notify();
        Ui.Run(async () =>
        {
            try
            {
                var p = await App.Peer(DeviceId!);
                Peer = p;
                Displays = await p.Session.ListDisplays();
                var first = Displays.FirstOrDefault(d => d.Primary) ?? Displays.FirstOrDefault();
                if (first == null)
                {
                    Fail(L.T("remote.noDisplay"));
                    return;
                }
                await Open(first.Id);
            }
            catch (Exception e) { Fail(Fmt.Error(e)); }
        });
        stats?.Stop();
        stats = Ui.Every(1000, () =>
        {
            MeasuredFps = frameCount;
            MeasuredKbps = byteCount * 8 / 1000;
            frameCount = byteCount = 0;
            if (Phase == RemotePhase.Streaming && !App.SelfTest) Notify();
        });
        clipTimer?.Stop();
        if (!App.SelfTest) clipTimer = Ui.Every(1000, PollClipboard);
    }

    private void Fail(string reason)
    {
        Phase = RemotePhase.Failed;
        FailReason = reason;
        Live = false;
        Notify();
    }

    public async Task Open(uint id)
    {
        if (Peer == null) return;
        try { channel?.Stop(); } catch { }
        channel = null;
        DisplayId = id;
        Phase = RemotePhase.Connecting;
        waitKeyframe = false;
        Notify();
        var adapter = new SinkAdapter(this);
        sink = adapter;
        try
        {
            // Windows 主控端只有 H.264 软解
            var opts = new DisplayOptions(id, (uint)Fps, (uint)BitrateKbps, new[] { VideoCodec.H264 }, !Peer.Info.Permissions.Control);
            channel = await Peer.Session.OpenDisplay(opts, adapter);
            Live = true;
            Notify();
        }
        catch (Exception e) { Fail(Fmt.Error(e)); }
    }

    // ---------- 收帧与解码 ----------

    private void FrameArrived(byte[] data, bool keyframe)
    {
        if (closed) return;
        if (waitKeyframe)
        {
            if (!keyframe) return;
            waitKeyframe = false;
        }
        // 解码跟不上：丢掉积压的帧，等下一个关键帧
        if (queue.Count > 6)
        {
            while (queue.TryTake(out _)) { }
            waitKeyframe = !keyframe;
            if (!keyframe) { try { channel?.RequestKeyframe(); } catch { } return; }
        }
        try { queue.Add(data); }
        catch (InvalidOperationException) { return; }
        Interlocked.Add(ref byteCount, data.Length);
    }

    private void DecodeLoop()
    {
        try
        {
            foreach (var data in queue.GetConsumingEnumerable())
            {
                if (closed) break;
                DecodedFrame? f;
                try { f = decoder.Decode(data); }
                catch (Exception e)
                {
                    Log.Error("解码失败：" + e.Message);
                    waitKeyframe = true;
                    try { channel?.RequestKeyframe(); } catch { }
                    continue;
                }
                if (f == null) continue;
                lock (frameLock) latest = f;
                Ui.Coalesce(this, Present, DispatcherPriority.Render);
            }
        }
        catch (Exception e) { Log.Error("解码线程退出：" + e.Message); }
    }

    private void Present()
    {
        DecodedFrame? f;
        lock (frameLock) { f = latest; latest = null; }
        if (f == null || closed) return;
        int w = (int)f.Width, h = (int)f.Height;
        if (bitmap == null || bitmap.PixelWidth != w || bitmap.PixelHeight != h)
        {
            bitmap = new WriteableBitmap(w, h, 96, 96, PixelFormats.Bgra32, null);
            image.Source = bitmap;
            StreamW = w;
            StreamH = h;
            UpdateScaleMode();
        }
        bitmap.WritePixels(new Int32Rect(0, 0, w, h), f.Bgra, w * 4, 0);
        frameCount++;
        FramesDecoded++;
        if (Phase != RemotePhase.Streaming) { Phase = RemotePhase.Streaming; Notify(); }
    }

    private void Configured(VideoCodec codec, uint width, uint height)
    {
        StreamW = (int)width;
        StreamH = (int)height;
        Notify();
    }

    public void UpdateScaleMode()
    {
        if (FitMode)
        {
            image.Stretch = Stretch.Uniform;
            image.Width = image.Height = double.NaN;
        }
        else
        {
            // 1:1：按被控屏幕的逻辑尺寸显示
            image.Stretch = Stretch.Fill;
            var d = Display;
            var scale = d?.Scale is > 0 ? d.Scale : 1f;
            image.Width = (d?.Width ?? (uint)StreamW) / scale;
            image.Height = (d?.Height ?? (uint)StreamH) / scale;
        }
    }

    // ---------- 设置 ----------

    public void ApplyQuality(string q)
    {
        Quality = q;
        var (fps, kbps) = AppSettings.Quality(q);
        var fpsChanged = fps != Fps;
        Fps = fps;
        BitrateKbps = kbps;
        if (fpsChanged) Ui.Run(() => Open(DisplayId));
        else channel?.SetBitrate((uint)kbps);
        Notify();
    }

    public void SetBitrate(int kbps)
    {
        Quality = "custom";
        BitrateKbps = kbps;
        channel?.SetBitrate((uint)kbps);
        Notify();
    }

    public void SetFps(int f)
    {
        Quality = "custom";
        Fps = f;
        Ui.Run(() => Open(DisplayId));
    }

    public void Special(ComboKey c) => Send(new InputEvent.SpecialCombo(c));

    /// <summary>隐私屏：被控端本地屏幕黑掉、本地键鼠失效，远端照常操作；本会话结束时被控端自动关闭</summary>
    public void TogglePrivacy()
    {
        if (Peer == null) return;
        var on = !Privacy;
        var p = Peer;
        Ui.Run(async () =>
        {
            try
            {
                await p.Session.SetPrivacy(on);
                Privacy = on;
                Notify();
                App.ShowToast(on ? L.T("remote.privacy.on") : L.T("remote.privacy.off"));
            }
            catch (Exception e) { App.ShowError(e); }
        });
    }

    private void Send(InputEvent e)
    {
        try { channel?.SendInput(e); }
        catch (Exception ex) { Log.Error("发送输入失败：" + ex.Message); }
    }

    // ---------- 剪贴板同步（文字）：本机变化推给远端；鼠标进入画面时拉取远端 ----------

    private void PollClipboard()
    {
        if (!ClipboardSync || Peer == null || Phase != RemotePhase.Streaming || FullscreenWindow == null && App.ActiveTabId != Id) return;
        var t = SysClipboard.GetText();
        if (t == null || t == lastClipboard) return;
        lastClipboard = t;
        var p = Peer;
        Ui.Run(async () => { try { await p.Session.ClipboardSet(new ClipboardData.Text(t)); } catch { } });
    }

    private void PullRemoteClipboard()
    {
        if (!ClipboardSync || Peer == null || App.SelfTest) return;
        var p = Peer;
        Ui.Run(async () =>
        {
            try
            {
                if (await p.Session.ClipboardGet() is ClipboardData.Text t && t.TextValue != SysClipboard.GetText())
                {
                    lastClipboard = t.TextValue;
                    SysClipboard.SetText(t.TextValue);
                }
            }
            catch { }
        });
    }

    // ---------- 输入 ----------

    /// <summary>界面坐标 → 被控屏幕像素</summary>
    private (int x, int y)? Remote(Point pInHost)
    {
        var d = Display;
        if (d == null || bitmap == null || image.ActualWidth <= 0 || image.ActualHeight <= 0) return null;
        var p = VideoHost.TranslatePoint(pInHost, image);
        // Uniform 时图片元素本身就是等比缩放后的画面区域
        var nx = Math.Min(Math.Max(p.X / image.ActualWidth, 0), 1);
        var ny = Math.Min(Math.Max(p.Y / image.ActualHeight, 0), 1);
        return ((int)(nx * (d.Width - 1)), (int)(ny * (d.Height - 1)));
    }

    private static PointerButton? ButtonOf(MouseButton b) => b switch
    {
        MouseButton.Left => PointerButton.Left,
        MouseButton.Right => PointerButton.Right,
        MouseButton.Middle => PointerButton.Middle,
        MouseButton.XButton1 => PointerButton.Back,
        MouseButton.XButton2 => PointerButton.Forward,
        _ => null,
    };

    private void HookInput()
    {
        var h = VideoHost;
        h.MouseEnter += (_, _) => PullRemoteClipboard();
        h.MouseMove += (_, e) => { if (Remote(e.GetPosition(h)) is { } r) Send(new InputEvent.MouseMove(r.x, r.y)); };
        h.MouseDown += (_, e) =>
        {
            h.Focus();
            Keyboard.Focus(h);
            if (ButtonOf(e.ChangedButton) is { } b && Remote(e.GetPosition(h)) is { } r) Send(new InputEvent.MouseButton(b, true, r.x, r.y));
            h.CaptureMouse();
            e.Handled = true;
        };
        h.MouseUp += (_, e) =>
        {
            if (ButtonOf(e.ChangedButton) is { } b && Remote(e.GetPosition(h)) is { } r) Send(new InputEvent.MouseButton(b, false, r.x, r.y));
            if (Mouse.LeftButton == MouseButtonState.Released && Mouse.RightButton == MouseButtonState.Released && Mouse.MiddleButton == MouseButtonState.Released) h.ReleaseMouseCapture();
            e.Handled = true;
        };
        // 滚轮一格 120 → 40 像素（与 Mac 端按行换算一致）
        h.MouseWheel += (_, e) => { Send(new InputEvent.Wheel(0, e.Delta / 3)); e.Handled = true; };
        h.ContextMenuOpening += (_, e) => e.Handled = true;
        h.PreviewKeyDown += (_, e) =>
        {
            if (e.Key == Key.ImeProcessed) { imeComposing = true; e.Handled = false; return; }
            if (HidKeyMap.Hid(e) is { } hid)
            {
                pressed.Add(hid);
                Send(new InputEvent.Key(hid, true));
            }
            // 不让 Tab、方向键、Alt 等触发本地的焦点切换与菜单
            e.Handled = true;
        };
        h.PreviewKeyUp += (_, e) =>
        {
            if (e.Key == Key.ImeProcessed) return;
            if (HidKeyMap.Hid(e) is { } hid)
            {
                pressed.Remove(hid);
                Send(new InputEvent.Key(hid, false));
            }
            e.Handled = true;
        };
        // 输入法提交的文字按 Text 发送（物理键已按 HID 发过，不重复）
        h.TextInput += (_, e) =>
        {
            if (!imeComposing || string.IsNullOrEmpty(e.Text)) return;
            imeComposing = false;
            Send(new InputEvent.Text(e.Text));
            e.Handled = true;
        };
        // 失去焦点时把按住的键都抬起来，避免远端卡住修饰键
        h.LostKeyboardFocus += (_, _) =>
        {
            foreach (var k in pressed.ToList()) Send(new InputEvent.Key(k, false));
            pressed.Clear();
        };
    }

    // ---------- 全屏 ----------

    public void EnterFullscreen()
    {
        if (FullscreenWindow != null) return;
        Region.Detach(VideoHost);
        var grid = new Grid { Background = Brushes.Black };
        grid.Children.Add(VideoHost);
        var exit = K.TintButton(L.T("remote.exitFullscreen"), ExitFullscreen, Colors.White, "fit", 30);
        exit.Background = Th.B(Colors.Black, 0.55);
        exit.Normal = Th.B(Colors.Black, 0.55);
        exit.HoverBrush = Th.B(Colors.Black, 0.75);
        exit.HorizontalAlignment = HorizontalAlignment.Right;
        exit.VerticalAlignment = VerticalAlignment.Top;
        exit.Margin = new Thickness(12);
        exit.Opacity = 0.85;
        grid.Children.Add(exit);
        var w = new Window
        {
            WindowStyle = WindowStyle.None, ResizeMode = ResizeMode.NoResize, Background = Brushes.Black, Title = Title + " - LemonGalaxy",
            Content = grid, ShowInTaskbar = true, Owner = Application.Current?.MainWindow,
        };
        if (MainWindow.AppIcon is { } icon) w.Icon = icon;
        // 先放到主窗口所在的显示器再最大化
        if (Application.Current?.MainWindow is { } mw) { w.Left = mw.Left + 20; w.Top = mw.Top + 20; }
        w.Loaded += (_, _) => { w.WindowState = WindowState.Maximized; VideoHost.Focus(); };
        w.KeyDown += (_, e) => { if (e.Key == Key.F11) ExitFullscreen(); };
        w.Closed += (_, _) =>
        {
            FullscreenWindow = null;
            Region.Detach(VideoHost);
            Notify();
        };
        FullscreenWindow = w;
        w.Show();
        Notify();
    }

    public void ExitFullscreen() => FullscreenWindow?.Close();

    public override void Close()
    {
        closed = true;
        try { channel?.Stop(); } catch { }
        channel = null;
        stats?.Stop();
        clipTimer?.Stop();
        queue.CompleteAdding();
        FullscreenWindow?.Close();
    }

    public void Disconnect() => App.CloseTab(Id);

    protected override FrameworkElement CreateView() => RemoteView.Build(App, this);

    private sealed class SinkAdapter : VideoSink
    {
        private readonly WeakReference<RemoteTab> owner;
        public SinkAdapter(RemoteTab t) { owner = new WeakReference<RemoteTab>(t); }
        private RemoteTab? Tab => owner.TryGetTarget(out var t) && ReferenceEquals(t.sink, this) ? t : null;

        public void OnConfig(VideoCodec codec, uint width, uint height) => Ui.Post(() => Tab?.Configured(codec, width, height));
        public void OnFrame(byte[] data, ulong ptsUs, bool keyframe) => Tab?.FrameArrived(data, keyframe);
        public void OnCursor(int x, int y, bool visible) { }
        public void OnClosed(string reason) => Ui.Post(() => Tab?.Fail(reason.Length == 0 ? L.T("remote.closed") : reason));
    }
}

internal static class RemoteView
{
    public static FrameworkElement Build(AppModel app, RemoteTab tab)
    {
        var dock = new DockPanel { Background = Brushes.Black };
        var bar = new Region(() => Toolbar(app, tab)).On(tab, app.Data);
        DockPanel.SetDock(bar, Dock.Top);
        dock.Children.Add(bar);
        var area = new Grid { Background = Brushes.Black };
        var video = new Region(() =>
        {
            if (tab.FullscreenWindow != null)
                return K.Center(K.Col(10, K.Center(K.Icon("fullscreen", 30, Th.B(Colors.White, 0.6), 1.5)), K.Center(K.TintButton(L.T("remote.exitFullscreen"), tab.ExitFullscreen, Colors.White))));
            Region.Detach(tab.VideoHost);
            tab.UpdateScaleMode();
            if (tab.FitMode) return tab.VideoHost;
            return new ScrollViewer { Content = tab.VideoHost, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Focusable = false };
        }).Keyed(() => (tab.FitMode, tab.FullscreenWindow != null)).On(tab);
        area.Children.Add(video);
        area.Children.Add(new Region(() => tab.Phase switch
        {
            RemotePhase.Connecting => K.Center(K.Col(10, K.Center(K.Spinner(22, Brushes.White)), K.Center(K.Text(L.T("remote.connecting"), 13, Th.B(Colors.White, 0.75))))),
            RemotePhase.Failed => K.Center(K.Col(12, K.Center(K.Icon("display", 32, Th.B(Colors.White, 0.7), 1.4)),
                K.Center(K.Text(tab.FailReason, 13.5, Th.B(Colors.White, 0.85), wrap: true).Also(t => { t.TextAlignment = TextAlignment.Center; t.MaxWidth = 420; })),
                K.Center(K.PrimaryButton(L.T("common.retry"), tab.Start)))),
            _ => null,
        }).On(tab));
        dock.Children.Add(area);
        return dock;
    }

    private static FrameworkElement Divider() => K.Margin(K.VLine(null, 18), 3, 0, 3, 0);

    private static FrameworkElement Toolbar(AppModel app, RemoteTab tab)
    {
        var items = new List<UIElement?>();
        if (tab.Displays.Length > 1)
        {
            var screens = new StackPanel { Orientation = Orientation.Horizontal };
            for (var i = 0; i < tab.Displays.Length; i++)
            {
                var d = tab.Displays[i];
                var sel = tab.DisplayId == d.Id;
                var b = new Btn
                {
                    Height = 26, Padding = new Thickness(8, 0, 8, 0), CornerRadius = new CornerRadius(6), Normal = sel ? Th.B(Th.P.Accent, 0.14) : Brushes.Transparent, HoverBrush = sel ? Th.B(Th.P.Accent, 0.2) : Th.Hover2,
                    Child = K.Row(4, K.Icon(d.IsVirtual ? "fit" : "display", 12, sel ? Th.Accent : Th.Secondary), K.Text((i + 1).ToString(CultureInfo.InvariantCulture), 12.5, sel ? Th.Accent : Th.Secondary, sel ? FontWeights.SemiBold : FontWeights.Normal)),
                };
                b.UpdateLook();
                K.Tip(b, L.T("remote.screen", i + 1) + $" · {d.Name} · {d.Width}×{d.Height}");
                var id = d.Id;
                b.Click += () => Ui.Run(() => tab.Open(id));
                screens.Children.Add(b);
            }
            items.Add(screens);
            items.Add(Divider());
        }
        items.Add(K.Segmented(new[] { (true, L.T("remote.fit")), (false, "1:1") }, tab.FitMode, v => { tab.FitMode = v; tab.Notify(); }, height: 28));
        items.Add(QualityMenu(tab));
        items.Add(Divider());
        items.Add(K.IconButton("keyboard", "Ctrl+Alt+Del", () => tab.Special(ComboKey.CtrlAltDel)));
        items.Add(K.IconButton("lockScreen", L.T("remote.lock"), () => tab.Special(ComboKey.LockScreen)));
        items.Add(K.MenuButton(K.Icon("command", 14), () => new List<MenuEntry>
        {
            new("Ctrl+Alt+Del", () => tab.Special(ComboKey.CtrlAltDel)),
            new("Alt+Tab", () => tab.Special(ComboKey.AltTab)),
            new(L.T("remote.combo.taskManager"), () => tab.Special(ComboKey.TaskManager)),
            new(L.T("remote.combo.winKey"), () => tab.Special(ComboKey.WinKey)),
            new(L.T("remote.combo.showDesktop"), () => tab.Special(ComboKey.ShowDesktop)),
            new(L.T("remote.lock"), () => tab.Special(ComboKey.LockScreen)),
        }, L.T("remote.keys")));
        items.Add(K.IconButton("clipboard", L.T("remote.clipboard"), () => { tab.ClipboardSync = !tab.ClipboardSync; tab.Notify(); }, tab.ClipboardSync));
        if (tab.PrivacySupported) items.Add(K.IconButton("eyeOff", L.T("remote.privacy"), tab.TogglePrivacy, tab.Privacy));
        items.Add(K.Spacer(8));
        var detail = tab.StreamW > 0 ? $"{tab.StreamW}×{tab.StreamH} · H.264" : "";
        items.Add(K.Tip(K.Text($"{tab.MeasuredFps} fps · {(tab.MeasuredKbps / 1000.0).ToString("0.0", CultureInfo.InvariantCulture)} Mbps", 11.5, Th.Tertiary), detail));
        if (tab.Peer != null) items.Add(TerminalView.PathBadge(tab.Peer));
        items.Add(Divider());
        items.Add(K.IconButton("fullscreen", L.T("remote.fullscreen") + " (F11)", tab.EnterFullscreen));
        items.Add(K.TintButton(L.T("remote.disconnect"), tab.Disconnect, Th.P.Danger));
        return new Border
        {
            Height = 40, Padding = new Thickness(10, 0, 10, 0), Background = Th.Content, BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 0, 0, 1),
            Child = K.Row(6, items),
        };
    }

    private static readonly int[] FpsChoices = { 30, 60, 90, 120, 144 };
    private static readonly int[] BitrateChoices = { 2000, 6000, 12000, 20000, 30000, 50000 };

    private static FrameworkElement QualityMenu(RemoteTab tab)
        => K.MenuButton(K.Row(5, K.Icon("sliders", 13), K.Text(L.T("remote.quality." + tab.Quality), 12.5, Th.Text), K.Icon("chevronDown", 10, Th.Tertiary, 2.4)), () =>
        {
            var list = new List<MenuEntry>
            {
                new(L.T("remote.quality.smooth"), () => tab.ApplyQuality("smooth"), Checked: tab.Quality == "smooth"),
                new(L.T("remote.quality.balanced"), () => tab.ApplyQuality("balanced"), Checked: tab.Quality == "balanced"),
                new(L.T("remote.quality.sharp"), () => tab.ApplyQuality("sharp"), Checked: tab.Quality == "sharp"),
            };
            if (tab.Quality == "custom") list.Add(new(L.T("remote.quality.custom"), null, Checked: true));
            list.Add(MenuEntry.Sep);
            list.Add(new(L.T("remote.fps"), Sub: FpsChoices.Select(f => new MenuEntry($"{f} fps", () => tab.SetFps(f), Checked: tab.Fps == f)).ToList()));
            list.Add(new(L.T("remote.bitrate"), Sub: BitrateChoices.Select(k => new MenuEntry($"{k / 1000} Mbps", () => tab.SetBitrate(k), Checked: tab.BitrateKbps == k)).ToList()));
            return list;
        }, L.T("remote.quality"));
}
