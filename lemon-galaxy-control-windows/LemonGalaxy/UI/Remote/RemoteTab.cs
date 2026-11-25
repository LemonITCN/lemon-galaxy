using System.Collections.Concurrent;
using System.Globalization;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

/// <summary>Reconnecting：看过画面后会话断开或画面意外结束，自动重连中（最后一帧压暗留着）</summary>
internal enum RemotePhase { Connecting, Streaming, Reconnecting, Failed }

/// <summary>
/// 远程桌面标签：list_displays → open_display(VideoSink) → 解码线程 H.264 软解 → 界面线程写 WriteableBitmap；
/// 鼠标键盘转 InputEvent（键盘为 USB HID 用法码，坐标为被控屏幕像素）。界面忙时只显示最新一帧，解码跟不上时丢帧并请求关键帧。
/// 看过画面后会话断开或画面不是自己停的却结束了：按 1、2、4、8、15 秒…的间隔自动重连，约 2 分钟仍不行才显示失败。
/// </summary>
internal sealed partial class RemoteTab : TabModel
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
    public bool ClipboardSync { get; private set; }
    public int MeasuredFps, MeasuredKbps;
    public int FramesDecoded;
    /// <summary>隐私屏已开启</summary>
    public bool Privacy;
    public bool PrivacySupported => Peer?.Info.Capabilities.Contains("privacy") == true;
    /// <summary>正在本机播放被控端的声音</summary>
    public bool AudioOn;
    public bool AudioBusy;
    public bool AudioSupported => Peer?.Info.Capabilities.Contains("audio") == true;
    /// <summary>正在录像（内核把收到的画面与声音原样封装成 MP4）</summary>
    public bool Recording;
    public bool RecordingBusy;
    /// <summary>开始录像的时刻（Clock 毫秒），工具栏显示已录时长</summary>
    public long RecordingSinceMs;
    public Window? FullscreenWindow;

    /// <summary>画面元素（常驻，视图重建或进入全屏时挂到新父元素上）</summary>
    public readonly Grid VideoHost;
    private readonly Image image;
    private WriteableBitmap? bitmap;
    private ICoreDisplay? channel;
    private SinkAdapter? sink;
    private readonly IFrameDecoder decoder;
    /// <summary>待解码的帧，带来源画面通道的代号：换了通道后旧通道的帧解出来也不显示</summary>
    private readonly BlockingCollection<(int Gen, byte[] Data)> queue = new(new ConcurrentQueue<(int, byte[])>());
    private readonly Thread decodeThread;
    private volatile bool closed;
    private volatile bool waitKeyframe;
    private (int Gen, DecodedFrame Frame)? latest;
    private readonly object frameLock = new();
    /// <summary>每打开或丢掉一路画面加一（界面线程）；新通道的 SinkAdapter 记下它</summary>
    private int streamGen;
    /// <summary>本次开始（Start）后看到过画面：之后断了才自动重连，首次连接失败照旧显示失败</summary>
    private bool hadPicture;
    private int frameCount, byteCount;
    private DispatcherTimer? stats, clipTimer;
    private string? lastClipboard;
    /// <summary>上次处理过的剪贴板序号：同一次复制只处理一次</summary>
    private uint lastClipSeq;
    /// <summary>上次和对方同步时（推过去或拉过来）本机剪贴板的序号：之后本机又复制了东西，鼠标回到画面时以本机为准</summary>
    private uint clipSeqAtSync = ClipFiles.Sequence();
    /// <summary>上次同步时对方剪贴板里的文字（推过去的或拉过来的）：对方没再复制新文字就不拉</summary>
    private string? lastRemoteText;
    private bool skipExistingFiles;
    private string? lastFilesKey;
    /// <summary>复制粘贴文件的进度条（画面上方）；null 时不显示</summary>
    public FileClip? FileClip { get; private set; }
    /// <summary>进度条变化（单独通知，进度刷新时不重建工具栏）</summary>
    public readonly Observable ClipChanged = new();
    private ICoreClipboardTransfer? clipTransfer;
    private ClipTransferListener? clipListener;
    /// <summary>每次开始或作废一次文件传输加一：迟到的回调按它丢弃</summary>
    private int clipGen;
    /// <summary>上次处理过的对方剪贴板文件列表（放进过本机剪贴板的，或老被控端推送后对方剪贴板里的暂存路径）：同一列表不重复放</summary>
    public string[]? LastRemoteFiles { get; private set; }
    /// <summary>放进本机剪贴板的对方文件（虚拟文件，粘贴时才读）；selftest 不写系统剪贴板，用它核对</summary>
    public RemoteFileData? PasteData { get; private set; }
    /// <summary>每次放对方文件或作废时加一：迟到的展开结果按它丢弃</summary>
    private int pasteGen;
    /// <summary>每次发清单或本机又复制了文字时加一：迟到的失败提示按它丢弃</summary>
    private int offerGen;
    /// <summary>最近一次把本机文字设到对方剪贴板（粘贴键等它完成）</summary>
    private Task? textSync;
    /// <summary>selftest 核对用</summary>
    public ICoreClipboardTransfer? ClipTransfer => clipTransfer;
    /// <summary>当前画面通道（selftest 模拟画面断了用）</summary>
    public ICoreDisplay? Channel => channel;
    private readonly HashSet<uint> pressed = new();
    /// <summary>发过「按下」还没抬起的鼠标键（及位置）：失去焦点、应用不在前台时一并抬起</summary>
    private readonly Dictionary<PointerButton, (int x, int y)> buttonsDown = new();

    /// <summary>修饰键（WPF 键 → HID）：远端的修饰键状态按本机实际状态对齐，不只靠按下 / 抬起事件</summary>
    private static readonly (Key key, uint hid)[] ModifierKeyMap =
    {
        (Key.LeftCtrl, 0xE0), (Key.LeftShift, 0xE1), (Key.LeftAlt, 0xE2), (Key.LWin, 0xE3),
        (Key.RightCtrl, 0xE4), (Key.RightShift, 0xE5), (Key.RightAlt, 0xE6), (Key.RWin, 0xE7),
    };

    private static bool IsModifierHid(uint hid) => hid >= 0xE0 && hid <= 0xE7;

    /// <summary>
    /// 把远端的修饰键对齐本机实际状态（多的抬起、少的按下）。键盘与鼠标事件发出前都调一次：漏了某次抬起
    /// （窗口外松开、Alt+Tab 切走后才松开等）时下一个键鼠事件就纠正，远端不会一直卡着 Ctrl / Shift / Alt，
    /// 点什么都成了 Ctrl+点击。
    /// </summary>
    private void SyncModifiers()
    {
        foreach (var (key, hid) in ModifierKeyMap)
        {
            var down = Keyboard.IsKeyDown(key);
            if (down && pressed.Add(hid)) Send(new InputEvent.Key(hid, true));
            else if (!down && pressed.Remove(hid)) Send(new InputEvent.Key(hid, false));
        }
    }

    /// <summary>把按着的键、修饰键与鼠标键全部抬起（失去焦点、应用切到后台：之后的抬起不会再交给我们）</summary>
    private void ReleaseAllInput()
    {
        foreach (var k in pressed.ToList()) Send(new InputEvent.Key(k, false));
        pressed.Clear();
        foreach (var kv in buttonsDown.ToList()) Send(new InputEvent.MouseButton(kv.Key, false, kv.Value.x, kv.Value.y));
        buttonsDown.Clear();
    }

    private void OnAppDeactivated(object? sender, EventArgs e) => ReleaseAllInput();
    /// <summary>按过输入法处理的键、还没收到它交出的文字</summary>
    private bool imeComposing;
    /// <summary>暂存的输入（粘贴前等文字同步、等输入法交出文字时，之后的输入按顺序排在这里）；null 表示直接发</summary>
    private List<InputEvent>? heldInput;
    private int holds;
    /// <summary>输入法交出的文字插到暂存队列的这个位置；-1 表示没在等输入法</summary>
    private int imeInsertAt = -1;
    private TaskCompletionSource<bool>? imeWait;
    private ICoreAudio? audio;
    /// <summary>每次开始或停止声音加一：开到一半被关掉、断线或关闭标签时，迟到的结果直接停掉丢弃</summary>
    private int audioGen;
    private ICoreDisplay? recordChannel;
    private string? recordPath;
    /// <summary>每次开始或停止录像加一（同 audioGen）</summary>
    private int recordGen;

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
        // 首次打开或手动重试：放弃正在等的自动重连，从头来
        CancelReconnect();
        hadPicture = false;
        Phase = RemotePhase.Connecting;
        StopAudio();
        Notify();
        Ui.Run(async () =>
        {
            try
            {
                var p = await App.Peer(DeviceId!);
                UsePeer(p);
                Displays = await p.Session.ListDisplays();
                PruneVirtual();
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
        if (!App.SelfTest)
        {
            // 打开画面前就在剪贴板里的文件不自动传（文字照旧同步过去）
            skipExistingFiles = true;
            clipTimer = Ui.Every(1000, PollClipboard);
        }
    }

    /// <summary>用这条会话。换了新会话：被控端随旧会话已删掉我加的虚拟屏、关掉隐私屏；盯着它断开</summary>
    private void UsePeer(PeerConnection p)
    {
        if (!ReferenceEquals(Peer, p))
        {
            ResetVirtual();
            Privacy = false;
            Peer = p;
            p.Watch(this, () => PeerChanged(p));
        }
        VirtualMax = QueryVirtualMax(p);
    }

    /// <summary>会话断了：声音通道随之失效，停掉本机播放；正在录像的结束并提示保存位置；我加的虚拟屏、隐私屏被控端已自动撤掉。看着画面的转入自动重连</summary>
    private void PeerChanged(PeerConnection p)
    {
        if (p.ClosedReason == null || !ReferenceEquals(Peer, p)) return;
        StopAudio();
        StopRecording();
        // 放进本机剪贴板的对方文件随会话失效，重连后鼠标移回画面时重新放
        CancelPaste();
        ResetVirtual();
        Privacy = false;
        ConnectionLost(p.ClosedReason.Length == 0 ? L.T("path.closed") : p.ClosedReason);
        Notify();
    }

    private void Fail(string reason)
    {
        CancelReconnect();
        StopRecording();
        Phase = RemotePhase.Failed;
        FailReason = reason;
        Live = false;
        StopAudio();
        Notify();
    }

    /// <summary>切屏、改帧率、加删虚拟屏时（重新）打开画面；失败显示失败状态。正在自动重连时只记下要看的屏，由重连去打开</summary>
    public async Task Open(uint id)
    {
        var p = Peer;
        if (p == null || closed) return;
        if (Phase == RemotePhase.Reconnecting)
        {
            DisplayId = id;
            // 重连已打开的画面还没出第一帧：换成这块屏（或新帧率）马上再试
            if (!reconnectPending)
            {
                DropChannel();
                reconnectPending = true;
                var gen = reconnectGen;
                Ui.Run(() => ReconnectOnce(gen));
            }
            Notify();
            return;
        }
        Phase = RemotePhase.Connecting;
        Notify();
        try { await OpenChannel(p, id); }
        catch (Exception e) { Fail(Fmt.Error(e)); }
    }

    /// <summary>
    /// 换掉当前画面通道，打开 id 这块屏。打开失败时抛出；等待期间又换了画面、丢掉了画面、换了会话或关了标签时，
    /// 新开的这一路直接停掉（失败也不报），返回 false。
    /// </summary>
    private async Task<bool> OpenChannel(PeerConnection p, uint id)
    {
        // 换屏幕或改帧率要重开画面：正在录的先结束（关画面时内核也会自动结束）
        StopRecording();
        DropChannel();
        DisplayId = id;
        waitKeyframe = false;
        var adapter = new SinkAdapter(this, streamGen);
        sink = adapter;
        bool Stale() => closed || !ReferenceEquals(sink, adapter) || !ReferenceEquals(Peer, p);
        ICoreDisplay ch;
        try
        {
            // Windows 主控端只有 H.264 软解
            var opts = new DisplayOptions(id, (uint)Fps, (uint)BitrateKbps, new[] { VideoCodec.H264 }, !p.Info.Permissions.Control);
            ch = await p.Session.OpenDisplay(opts, adapter);
        }
        catch when (Stale()) { return false; }
        if (Stale())
        {
            try { ch.Stop(); } catch { }
            return false;
        }
        channel = ch;
        Live = true;
        Notify();
        // 画面打开后按设置开始播放声音（已在播放时不重复开）
        if (App.Settings.RemoteAudio) StartAudio();
        return true;
    }

    /// <summary>停掉当前画面通道；之后它迟到的帧与结束通知都丢弃，积压的帧不再解码</summary>
    private void DropChannel()
    {
        var ch = channel;
        channel = null;
        sink = null;
        streamGen++;
        while (queue.TryTake(out _)) { }
        lock (frameLock) latest = null;
        try { ch?.Stop(); } catch { }
    }

    // ---------- 断线自动重连 ----------

    /// <summary>重连间隔（毫秒）：1、2、4、8、15 秒，之后每 15 秒，共 10 次（连同每次的连接耗时约 2 分钟）后放弃。selftest 会临时改短</summary>
    public static int[] ReconnectDelays = { 1000, 2000, 4000, 8000, 15000, 15000, 15000, 15000, 15000, 15000 };
    /// <summary>重连打开了画面却一直没有第一帧，按这次没连上接着重试（与内核刚打开画面时的看门狗一致）</summary>
    private const int FirstFrameTimeoutMs = 20000;
    /// <summary>每次开始、结束或取消一轮重连加一：等待中的定时器与迟到的结果按它丢弃</summary>
    private int reconnectGen;
    /// <summary>本轮已经排上的次数</summary>
    private int reconnectAttempt;
    /// <summary>正在等下一次或正在试；false 时是已打开画面、在等第一帧</summary>
    private bool reconnectPending;
    /// <summary>断开的原因；最后一次尝试的错误（放弃时优先显示它）</summary>
    private string reconnectReason = "", reconnectError = "";

    /// <summary>
    /// 会话断开或画面不是自己停的却结束了。看过画面的：最后一帧压暗留着、显示「正在重新连接」并按间隔重试；
    /// 已在重连的：重连打开的画面没出第一帧就又断了（如内核看门狗 20 秒判定没有响应），用本轮剩下的次数接着重试，不从头算（显示出新画面的第一帧才清零）。
    /// 本次开始后从没出过画面的：直接显示失败，不自动重连。已失败的不管。
    /// </summary>
    private void ConnectionLost(string reason)
    {
        if (closed) return;
        var reconnecting = Phase == RemotePhase.Reconnecting;
        if (!reconnecting && Phase == RemotePhase.Failed) return;
        if (!reconnecting && !hadPicture)
        {
            // 第一次就没打开（从没出过画面）：直接显示失败，不自动重连
            Fail(reason);
            return;
        }
        if (IsAssist && !Alive(Peer))
        {
            // 临时协助的会话断了不能重新建（要对方重新给验证码）：直接显示失败
            Fail(reconnecting ? reconnectReason : reason);
            return;
        }
        if (reconnecting)
        {
            if (reconnectPending) return;
            DropChannel();
            ScheduleReconnect(reconnectGen);
            return;
        }
        Log.Info("连接中断，自动重连：" + reason);
        StopRecording();
        DropChannel();
        CancelReconnect();
        reconnectReason = reason;
        Phase = RemotePhase.Reconnecting;
        Live = false;
        Notify();
        ScheduleReconnect(reconnectGen);
    }

    /// <summary>临时协助（设备码 + 验证码）连上的：会话断了不能经连接池重新建</summary>
    private bool IsAssist => DeviceId != null && DeviceId.StartsWith("assist:", StringComparison.Ordinal);

    private static bool Alive(PeerConnection? p) => p != null && p.ClosedReason == null && !p.Session.IsClosed();

    /// <summary>不再自动重连（手动开始、失败、关标签、重连成功）</summary>
    private void CancelReconnect()
    {
        reconnectGen++;
        reconnectAttempt = 0;
        reconnectPending = false;
        reconnectError = "";
    }

    private void ScheduleReconnect(int gen)
    {
        if (gen != reconnectGen || closed) return;
        if (reconnectAttempt >= ReconnectDelays.Length)
        {
            Fail(reconnectError.Length > 0 ? reconnectError : reconnectReason);
            return;
        }
        reconnectPending = true;
        Ui.After(ReconnectDelays[reconnectAttempt++], () =>
        {
            if (gen == reconnectGen && !closed) Ui.Run(() => ReconnectOnce(gen));
        });
    }

    /// <summary>
    /// 重连一次：会话还在就先在同一块屏上重开画面；不行（或会话断了）就经连接池重新建会话，回到原来那块屏（没了就主屏）。
    /// 打开了画面就等第一帧（Present 里结束这一轮），否则排下一次。
    /// </summary>
    private async Task ReconnectOnce(int gen)
    {
        bool Stale() => gen != reconnectGen || closed;
        if (Stale()) return;
        var id = DisplayId;
        var old = Peer;
        if (old != null && Alive(old))
        {
            try
            {
                if (await OpenChannel(old, id)) { ReconnectOpened(gen); return; }
            }
            catch (Exception e) { reconnectError = Fmt.Error(e); }
            if (Stale()) return;
            Log.Info("重连：重开画面没成功，重新建会话：" + reconnectError);
        }
        else if (IsAssist)
        {
            // 临时协助的会话断了不能重新建（要对方重新给验证码）
            Fail(reconnectReason);
            return;
        }
        try
        {
            var p = await App.Peer(DeviceId!);
            if (Stale()) return;
            UsePeer(p);
            var list = await p.Session.ListDisplays();
            if (Stale()) return;
            Displays = list;
            PruneVirtual();
            var target = list.FirstOrDefault(d => d.Id == id) ?? list.FirstOrDefault(d => d.Primary) ?? list.FirstOrDefault()
                ?? throw new InvalidOperationException(L.T("remote.noDisplay"));
            if (await OpenChannel(p, target.Id)) { ReconnectOpened(gen); return; }
        }
        catch (Exception e)
        {
            if (Stale()) return;
            reconnectError = Fmt.Error(e);
            Log.Info("重连没成功：" + reconnectError);
        }
        if (!Stale()) ScheduleReconnect(gen);
    }

    /// <summary>重连打开了画面：等第一帧；迟迟没有就按没连上接着重试</summary>
    private void ReconnectOpened(int gen)
    {
        if (gen != reconnectGen || closed || Phase != RemotePhase.Reconnecting) return;
        reconnectPending = false;
        var stream = streamGen;
        Ui.After(FirstFrameTimeoutMs, () =>
        {
            if (gen != reconnectGen || closed || reconnectPending || stream != streamGen || Phase != RemotePhase.Reconnecting) return;
            Log.Info("重连：打开的画面一直没有画面，接着重试");
            DropChannel();
            ScheduleReconnect(gen);
        });
    }

    /// <summary>画面不是我们停的却结束了（被控端结束、流断了、画面没有响应）。内核在我们自己停画面时回 "stopped"（切屏、改帧率、关标签），不算</summary>
    private void PictureEnded(string reason)
    {
        if (closed || reason == "stopped") return;
        ConnectionLost(reason.Length == 0 ? L.T("remote.closed") : reason);
    }

    // ---------- 收帧与解码 ----------

    private void FrameArrived(int gen, byte[] data, bool keyframe)
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
        try { queue.Add((gen, data)); }
        catch (InvalidOperationException) { return; }
        Interlocked.Add(ref byteCount, data.Length);
    }

    private void DecodeLoop()
    {
        try
        {
            foreach (var (gen, data) in queue.GetConsumingEnumerable())
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
                lock (frameLock) latest = (gen, f);
                Ui.Coalesce(this, Present, DispatcherPriority.Render);
            }
        }
        catch (Exception e) { Log.Error("解码线程退出：" + e.Message); }
    }

    private void Present()
    {
        (int Gen, DecodedFrame Frame)? next;
        lock (frameLock) { next = latest; latest = null; }
        // 已经换掉或丢掉的画面通道解出来的帧不显示（也不能把「正在重新连接」当成连上了）
        if (next is not { } n || closed || n.Gen != streamGen) return;
        var f = n.Frame;
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
        hadPicture = true;
        if (Phase != RemotePhase.Streaming)
        {
            // 重连打开的画面出了第一帧：这一轮重连结束，不另外提示
            if (Phase == RemotePhase.Reconnecting) CancelReconnect();
            Phase = RemotePhase.Streaming;
            Notify();
        }
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

    /// <summary>工具栏的声音开关：记住选择并立即开始或停止</summary>
    public void ToggleAudio()
    {
        if (AudioBusy) return;
        var on = !AudioOn;
        App.Settings.RemoteAudio = on;
        if (on) StartAudio();
        else
        {
            StopAudio();
            Notify();
        }
    }

    private void StartAudio()
    {
        if (closed || audio != null || AudioBusy || Peer == null || !AudioSupported) return;
        var p = Peer;
        var gen = ++audioGen;
        AudioBusy = true;
        Notify();
        Ui.Run(async () =>
        {
            ICoreAudio? a = null;
            Exception? error = null;
            try { a = await p.Session.StartAudio(1f); }
            catch (Exception e) { error = e; }
            if (gen != audioGen)
            {
                // 等待期间被关掉了：这一路不要了
                a?.Stop();
                return;
            }
            AudioBusy = false;
            if (a != null)
            {
                audio = a;
                AudioOn = true;
            }
            else if (error != null) App.ShowToast(L.T("remote.sound.failed", Fmt.Error(error)), true);
            Notify();
        });
    }

    /// <summary>停止本机播放（不改设置里的开关）</summary>
    private void StopAudio()
    {
        audioGen++;
        AudioBusy = false;
        AudioOn = false;
        var a = audio;
        audio = null;
        try { a?.Stop(); }
        catch (Exception e) { Log.Error("停止声音失败：" + e.Message); }
    }

    // ---------- 录像 ----------

    /// <summary>工具栏的录像按钮：开始或停止</summary>
    public void ToggleRecording()
    {
        if (Recording) StopRecording();
        else StartRecording();
    }

    /// <summary>当前画面通道是否在录（selftest 核对内核状态与界面一致）</summary>
    public bool ChannelRecording
    {
        get
        {
            try { return channel?.IsRecording() == true; }
            catch { return false; }
        }
    }

    private void StartRecording()
    {
        if (closed || Recording || RecordingBusy || channel == null || Phase != RemotePhase.Streaming) return;
        var ch = channel;
        string path;
        try { path = NewRecordingPath(); }
        catch (Exception e)
        {
            App.ShowToast(L.T("remote.record.startFailed", e.Message), true);
            return;
        }
        var gen = ++recordGen;
        var withAudio = AudioOn;
        RecordingBusy = true;
        Notify();
        Ui.Run(async () =>
        {
            Exception? error = null;
            try { await ch.StartRecording(path, withAudio); }
            catch (Exception e) { error = e; }
            if (gen != recordGen || closed || !ReferenceEquals(ch, channel))
            {
                // 等待期间画面关了或换了：这一路不要了（关画面时内核已自动结束录像）
                if (error == null) { try { ch.StopRecording(); } catch { } }
                return;
            }
            RecordingBusy = false;
            if (error != null) App.ShowToast(L.T("remote.record.startFailed", Fmt.Error(error)), true);
            else
            {
                Recording = true;
                recordChannel = ch;
                recordPath = path;
                RecordingSinceMs = Clock.NowMs;
                Log.Info("开始录像：" + path);
            }
            Notify();
        });
    }

    /// <summary>停止录像并提示保存位置（可「在资源管理器中显示」）。没在录时什么也不做。</summary>
    private void StopRecording()
    {
        if (!Recording && !RecordingBusy) return;
        recordGen++;
        var ch = recordChannel;
        var path = recordPath;
        var wasRecording = Recording;
        var since = RecordingSinceMs;
        Recording = false;
        RecordingBusy = false;
        recordChannel = null;
        recordPath = null;
        Notify();
        if (!wasRecording || ch == null || path == null) return;
        string[] paths;
        long ms;
        try
        {
            var info = ch.StopRecording();
            paths = info.Paths;
            // 内核给出的是媒体时长；没有时用界面计的时长
            ms = info.DurationMs > 0 ? (long)info.DurationMs : Clock.NowMs - since;
        }
        catch (Exception e)
        {
            // 会话断了或画面已关：内核已自动结束录像，拿不到汇总，按已知路径提示
            Log.Info("停止录像：" + Fmt.Error(e));
            paths = App.SelfTest || File.Exists(path) ? new[] { path } : Array.Empty<string>();
            ms = Clock.NowMs - since;
        }
        if (paths.Length == 0)
        {
            App.ShowToast(L.T("remote.record.empty"));
            return;
        }
        var first = paths[0];
        var text = paths.Length > 1
            ? L.T("remote.record.savedParts", paths.Length, Fmt.Elapsed(ms))
            : L.T("remote.record.saved", IOPath.GetFileName(first), Fmt.Elapsed(ms));
        Log.Info("录像已保存：" + string.Join("，", paths));
        App.ShowToast(text, false, L.T("remote.record.reveal"), () => App.RevealInExplorer(first));
    }

    /// <summary>%USERPROFILE%\Videos\LemonGalaxy\LemonGalaxy-设备名-yyyyMMdd-HHmmss.mp4（重名时加序号）</summary>
    private string NewRecordingPath()
    {
        var dir = App.RecordingsDir;
        if (!App.SelfTest) Directory.CreateDirectory(dir);
        var raw = DeviceId != null && DeviceId.StartsWith("assist:", StringComparison.Ordinal) ? Peer?.Info.Hostname ?? Title : App.DeviceName(DeviceId ?? "");
        var invalid = IOPath.GetInvalidFileNameChars();
        var name = new string(raw.Trim().Select(c => invalid.Contains(c) || char.IsWhiteSpace(c) ? '-' : c).ToArray());
        if (name.Length == 0) name = "remote";
        var stamp = Fmt.Local(Clock.NowMs).ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture);
        var path = IOPath.Combine(dir, $"LemonGalaxy-{name}-{stamp}.mp4");
        for (var i = 2; File.Exists(path); i++) path = IOPath.Combine(dir, $"LemonGalaxy-{name}-{stamp}-{i}.mp4");
        return path;
    }

    /// <summary>发输入；正在暂存时（粘贴前等文字同步、等输入法交出文字）排到暂存队列里，保持先后顺序</summary>
    private void Send(InputEvent e)
    {
        if (heldInput != null)
        {
            heldInput.Add(e);
            return;
        }
        SendNow(e);
    }

    private void SendNow(InputEvent e)
    {
        try { channel?.SendInput(e); }
        catch (Exception ex) { Log.Error("发送输入失败：" + ex.Message); }
    }

    /// <summary>暂存之后的输入，直到 until 完成或过了 ms 毫秒，再按原顺序发出（不阻塞界面线程；几次暂存重叠时等最后一次结束）。
    /// ime：等输入法交出文字，交出的文字插到暂存开始的位置（排在结束组字的那个键前面）</summary>
    private async void HoldInput(Task until, int ms, bool ime = false)
    {
        heldInput ??= new List<InputEvent>();
        holds++;
        if (ime) imeInsertAt = heldInput.Count;
        await Task.WhenAny(until, Task.Delay(ms));
        if (ime) imeInsertAt = -1;
        if (--holds > 0 || heldInput == null) return;
        var list = heldInput;
        heldInput = null;
        foreach (var e in list) SendNow(e);
    }

    /// <summary>粘贴键：Ctrl+V、Win+V（对方是 Mac 时即 Command+V）、Shift+Insert</summary>
    private static bool IsPasteKey(KeyEventArgs e)
    {
        var key = e.Key == Key.System ? e.SystemKey : e.Key;
        var m = Keyboard.Modifiers;
        return key == Key.V && (m & (ModifierKeys.Control | ModifierKeys.Windows)) != 0 || key == Key.Insert && (m & ModifierKeys.Shift) != 0;
    }

    /// <summary>
    /// 远程画面里按了粘贴键：本机刚复制、每秒一次的轮询还没同步过去的文字先立刻同步，这个键和之后的输入排在同步完成后
    /// （最多等 300 毫秒）再发，免得粘出对方剪贴板里的旧内容。文件走清单，不在这里等
    /// </summary>
    private void SyncBeforePaste()
    {
        PollClipboard();
        if (textSync is { IsCompleted: false } t) HoldInput(t, 300);
    }

    /// <summary>输入法交出的文字按 Text 发；只有控制字符的（回车、退格等，按键已按 HID 发过）不发</summary>
    private void SendImeText(string text)
    {
        var clean = new string(text.Where(c => c >= 0x20 && c != 0x7F).ToArray());
        if (clean.Length == 0) return;
        var e = new InputEvent.Text(clean);
        if (heldInput != null && imeInsertAt >= 0 && imeInsertAt <= heldInput.Count)
        {
            heldInput.Insert(imeInsertAt++, e);
            imeWait?.TrySetResult(true);
            return;
        }
        Send(e);
    }

    // ---------- 剪贴板同步（protocol.md 8.4.1）：本机变化推给远端，鼠标进入画面时拉取远端。文字即时同步；
    // 文件复制时只交换清单，在哪边粘贴才传、进度显示在那边（老被控端没有 clipboard_offer 时本机复制的文件仍整批上传） ----------

    /// <summary>工具栏开关。关掉时正在传的文件作废；重新打开时对方剪贴板里的文件可以再放一次</summary>
    public void SetClipboardSync(bool on)
    {
        if (ClipboardSync == on) return;
        ClipboardSync = on;
        CancelFileClip();
        LastRemoteFiles = null;
        Notify();
    }

    /// <summary>对方开放了剪贴板与文件权限才能复制粘贴文件</summary>
    public bool FileClipSupported
    {
        get
        {
            var info = Peer?.Info;
            return info != null && info.Permissions.Clipboard && info.Permissions.Files && info.Capabilities.Contains("clipboard") && info.Capabilities.Contains("files");
        }
    }

    /// <summary>被控端支持「复制时只发清单、粘贴时才传」（新版本都有）</summary>
    public bool ClipOfferSupported => Peer?.Info.Capabilities.Contains("clipboard_offer") == true;

    /// <summary>本机剪贴板变了（按剪贴板序号判断，同一次复制只处理一次）：文件交给 SendLocalFiles，文字直接设过去</summary>
    private void PollClipboard()
    {
        ReleaseStalePaste();
        if (!ClipboardSync || Peer == null || Phase != RemotePhase.Streaming || FullscreenWindow == null && App.ActiveTabId != Id) return;
        var seq = ClipFiles.Sequence();
        if (seq != 0 && seq == lastClipSeq) return;
        lastClipSeq = seq;
        var skip = skipExistingFiles;
        skipExistingFiles = false;
        // 剪贴板里是我们放进来的对方文件：不当成本机复制，不传回去（也不去问它的格式，免得等正在进行的粘贴）
        if (VirtualClipboard.OwnsClipboard) return;
        var files = ClipFiles.GetFiles();
        if (files != null)
        {
            // 拿不到序号时按内容去重
            var key = string.Join("\n", files);
            if (skip || seq == 0 && key == lastFilesKey)
            {
                lastFilesKey = key;
                return;
            }
            lastFilesKey = key;
            clipSeqAtSync = seq;
            SendLocalFiles(files);
            return;
        }
        var t = SysClipboard.GetText();
        // 有序号时每次复制都同步（对方剪贴板可能已被文件换掉）；拿不到序号时按内容去重
        if (t == null || seq == 0 && t == lastClipboard) return;
        lastClipboard = t;
        lastRemoteText = t;
        clipSeqAtSync = seq;
        // 本机又复制了东西：还在传的文件作废，之前发过去的清单也不再算数
        if (FileClip != null) CancelFileClip();
        offerGen++;
        LastRemoteFiles = null;
        textSync = SetRemoteText(Peer, t);
    }

    /// <summary>把文字设成对方剪贴板（失败只记日志）；粘贴键据此等同步完成</summary>
    private static async Task SetRemoteText(PeerConnection p, string text)
    {
        try { await p.Session.ClipboardSet(new ClipboardData.Text(text)); }
        catch (Exception e) { Log.Info("同步剪贴板文字失败：" + e.Message); }
    }

    /// <summary>
    /// 拉取对方剪贴板：文字直接放进本机剪贴板，文件做成虚拟文件放进去（粘贴时才传）。
    /// 只在「对方上次同步后又复制了新东西，且本机这段时间没再复制」时才放进本机：本机后来复制的东西
    /// （截图工具放的图片等，图片不同步给对方）不能被对方的旧文字盖掉；两边都变了以本机为准
    /// （鼠标在画面外时复制的总比进画面前在对方那边复制的晚）
    /// </summary>
    public void PullRemoteClipboard()
    {
        // 正在重新连接时不同步（本机剪贴板的变化等连上后照常推过去）
        if (!ClipboardSync || Peer == null || Phase == RemotePhase.Reconnecting) return;
        var p = Peer;
        Ui.Run(async () =>
        {
            ClipboardData? data;
            try { data = await p.Session.ClipboardGet(); }
            catch { return; }
            var seqNow = ClipFiles.Sequence();
            // 拿不到序号（0）时无从判断本机变没变，按没变处理
            var localChanged = !App.SelfTest && seqNow != 0 && seqNow != clipSeqAtSync;
            switch (data)
            {
                case ClipboardData.Text t:
                    LastRemoteFiles = null;
                    if (App.SelfTest) return;
                    var remoteChanged = t.TextValue != lastRemoteText;
                    lastRemoteText = t.TextValue;
                    clipSeqAtSync = seqNow;
                    if (!remoteChanged || localChanged) return;
                    // 本机剪贴板里是我们放的对方文件时不是文字，不用比较
                    if (!VirtualClipboard.OwnsClipboard && t.TextValue == SysClipboard.GetText()) return;
                    lastClipboard = t.TextValue;
                    SysClipboard.SetText(t.TextValue);
                    lastClipSeq = clipSeqAtSync = ClipFiles.Sequence();
                    break;
                case ClipboardData.Files f:
                    if (localChanged)
                    {
                        // 本机更新：这批对方文件当作已看过，不放进来
                        LastRemoteFiles = f.Paths;
                        clipSeqAtSync = seqNow;
                        break;
                    }
                    PlaceRemoteFiles(f.Paths);
                    break;
                case ClipboardData.Image:
                    LastRemoteFiles = null;
                    break;
            }
        });
    }

    /// <summary>本机复制了文件：被控端支持时只发清单（对方粘贴时才传，进度在对方那边，这边不显示进度条）；老被控端整批上传</summary>
    public void SendLocalFiles(string[] paths)
    {
        if (ClipOfferSupported) OfferFiles(paths);
        else PushFiles(paths);
    }

    /// <summary>本机文件 / 文件夹的清单 → 对方剪贴板。只在失败时提示</summary>
    private void OfferFiles(string[] paths)
    {
        if (Peer == null || paths.Length == 0) return;
        CancelFileClip();
        // 对方剪贴板马上换成这份清单（ClipboardGet 不会返回它）：对方之后再复制的文件照常放进本机
        LastRemoteFiles = null;
        var names = paths.Select(f => IOPath.GetFileName(f.TrimEnd('\\', '/'))).ToArray();
        if (!FileClipSupported)
        {
            ShowFileClipResult(new FileClip(FileClipDirection.Push, names) { State = FileClipState.Failed, Error = L.T("remote.files.noPermission") });
            return;
        }
        var p = Peer;
        var gen = ++offerGen;
        Ui.Run(async () =>
        {
            try { await p.Session.ClipboardOfferFiles(paths); }
            catch (Exception e)
            {
                if (gen != offerGen || closed || !ReferenceEquals(Peer, p)) return;
                ShowFileClipResult(new FileClip(FileClipDirection.Push, names) { State = FileClipState.Failed, Error = Fmt.Error(e) });
            }
        });
    }

    /// <summary>老被控端：本机文件 / 文件夹整批传到对方暂存区，完成后对方剪贴板就是它们</summary>
    public void PushFiles(string[] paths)
    {
        if (Peer == null || paths.Length == 0) return;
        CancelFileClip();
        var names = paths.Select(f => IOPath.GetFileName(f.TrimEnd('\\', '/'))).ToArray();
        if (!FileClipSupported)
        {
            ShowFileClipResult(new FileClip(FileClipDirection.Push, names) { State = FileClipState.Failed, Error = L.T("remote.files.noPermission") });
            return;
        }
        var listener = MakeClipListener(clipGen);
        FileClip = new FileClip(FileClipDirection.Push, names);
        clipListener = listener;
        try { clipTransfer = Peer.Session.ClipboardPushFiles(paths, listener); }
        catch (Exception e)
        {
            clipListener = null;
            ShowFileClipResult(new FileClip(FileClipDirection.Push, names) { State = FileClipState.Failed, Error = Fmt.Error(e) });
            return;
        }
        ClipChanged.Notify();
    }

    /// <summary>
    /// 对方复制了文件：展开成清单（不下载），做成虚拟文件放进本机剪贴板。在资源管理器里 Ctrl+V 或右键粘贴时才传，
    /// 进度用资源管理器自己的复制窗口。同一批只放一次；展开期间本机又复制了别的就不放
    /// </summary>
    public void PlaceRemoteFiles(string[] remotePaths)
    {
        if (Peer == null || remotePaths.Length == 0 || ClipFiles.SameFiles(remotePaths, LastRemoteFiles)) return;
        // 老被控端正在接收本机推过去的文件：对方剪贴板马上会被换掉，这次不放
        if (FileClip is { State: FileClipState.Running, Direction: FileClipDirection.Push }) return;
        LastRemoteFiles = remotePaths;
        var names = remotePaths.Select(ClipFiles.BaseName).ToArray();
        if (!FileClipSupported)
        {
            ShowFileClipResult(new FileClip(FileClipDirection.Pull, names) { State = FileClipState.Failed, Error = L.T("remote.files.noPermission") });
            return;
        }
        var p = Peer;
        var gen = ++pasteGen;
        var seq = ClipFiles.Sequence();
        Ui.Run(async () =>
        {
            ICoreRemoteClipOffer offer;
            try { offer = await p.Session.ClipboardRemoteOffer(remotePaths); }
            catch (Exception e)
            {
                if (gen == pasteGen && !closed) ShowFileClipResult(new FileClip(FileClipDirection.Pull, names) { State = FileClipState.Failed, Error = Fmt.Error(e) });
                return;
            }
            if (gen != pasteGen || closed || !ReferenceEquals(Peer, p))
            {
                offer.Dispose();
                return;
            }
            // 展开期间本机剪贴板变了（又复制了别的）：不盖掉；下次鼠标移回画面时再看对方剪贴板
            if (!App.SelfTest && ClipFiles.Sequence() != seq)
            {
                offer.Dispose();
                LastRemoteFiles = null;
                return;
            }
            RemoteFileData data;
            try { data = new RemoteFileData(offer); }
            catch (Exception e)
            {
                offer.Dispose();
                Log.Error("生成剪贴板文件描述失败：" + e);
                ShowFileClipResult(new FileClip(FileClipDirection.Pull, names) { State = FileClipState.Failed, Error = L.T("remote.files.localFailed") });
                return;
            }
            if (data.Count == 0)
            {
                data.Cancel();
                return;
            }
            PasteData?.Detach();
            PasteData = data;
            if (App.SelfTest) return;
            if (!await VirtualClipboard.Set(data))
            {
                if (ReferenceEquals(PasteData, data)) PasteData = null;
                data.Cancel();
                if (gen == pasteGen && !closed)
                    ShowFileClipResult(new FileClip(FileClipDirection.Pull, names) { State = FileClipState.Failed, Error = L.T("remote.files.localFailed") });
                return;
            }
            lastClipSeq = clipSeqAtSync = ClipFiles.Sequence();
        });
    }

    /// <summary>本机剪贴板已被换掉：之前放的对方文件不再归这个标签管（正在粘贴的照常读完，闲下来后作废）</summary>
    private void ReleaseStalePaste()
    {
        if (PasteData is not { Placed: true } d || VirtualClipboard.Holds(d)) return;
        PasteData = null;
        d.Detach();
    }

    /// <summary>会话断了或关标签：放进本机剪贴板的对方文件作废（还在剪贴板上就清掉），之后重新放</summary>
    private void CancelPaste()
    {
        pasteGen++;
        LastRemoteFiles = null;
        var d = PasteData;
        PasteData = null;
        if (d == null) return;
        VirtualClipboard.Clear(d);
        d.Cancel();
    }

    /// <summary>取消正在传的文件并收起进度条（取消时内核报的「已取消」随之丢弃）</summary>
    public void CancelFileClip()
    {
        clipGen++;
        var t = clipTransfer;
        clipTransfer = null;
        clipListener = null;
        try { t?.Cancel(); }
        catch (Exception e) { Log.Error("取消复制文件失败：" + e.Message); }
        if (FileClip == null) return;
        FileClip = null;
        ClipChanged.Notify();
    }

    /// <summary>收起完成或失败的提示</summary>
    public void DismissFileClip()
    {
        if (FileClip == null || FileClip.State == FileClipState.Running) return;
        FileClip = null;
        ClipChanged.Notify();
    }

    private ClipTransferListener MakeClipListener(int gen) => new((done, total) =>
    {
        if (gen != clipGen || FileClip is not { State: FileClipState.Running } c) return;
        c.Done = done;
        c.Total = total;
        ClipChanged.Notify();
    }, error => FileClipFinished(gen, error));

    private void FileClipFinished(int gen, string? error)
    {
        if (gen != clipGen || FileClip is not { State: FileClipState.Running } c) return;
        var transfer = clipTransfer;
        clipTransfer = null;
        clipListener = null;
        if (error != null)
        {
            c.State = FileClipState.Failed;
            c.Error = error;
            ShowFileClipResult(c);
            return;
        }
        // 对方剪贴板现在是暂存区里的这些文件：之后拉取看到同一列表不再放回本机
        try { LastRemoteFiles = transfer?.Paths() ?? Array.Empty<string>(); }
        catch { LastRemoteFiles = Array.Empty<string>(); }
        c.State = FileClipState.Done;
        if (c.Total > 0) c.Done = c.Total;
        ShowFileClipResult(c);
    }

    /// <summary>显示完成或失败提示，几秒后自动收起（selftest 不收，便于截图）</summary>
    private void ShowFileClipResult(FileClip c)
    {
        FileClip = c;
        ClipChanged.Notify();
        if (App.SelfTest) return;
        Ui.After(c.State == FileClipState.Done ? 4000 : 8000, () =>
        {
            if (!ReferenceEquals(FileClip, c)) return;
            FileClip = null;
            ClipChanged.Notify();
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
        h.MouseEnter += (_, _) => { if (!App.SelfTest) PullRemoteClipboard(); };
        h.MouseMove += (_, e) =>
        {
            SyncModifiers();
            if (Remote(e.GetPosition(h)) is { } r) Send(new InputEvent.MouseMove(r.x, r.y));
        };
        h.MouseDown += (_, e) =>
        {
            h.Focus();
            Keyboard.Focus(h);
            SyncModifiers();
            if (ButtonOf(e.ChangedButton) is { } b && Remote(e.GetPosition(h)) is { } r)
            {
                buttonsDown[b] = (r.x, r.y);
                Send(new InputEvent.MouseButton(b, true, r.x, r.y));
            }
            h.CaptureMouse();
            e.Handled = true;
        };
        h.MouseUp += (_, e) =>
        {
            SyncModifiers();
            if (ButtonOf(e.ChangedButton) is { } b && Remote(e.GetPosition(h)) is { } r)
            {
                buttonsDown.Remove(b);
                Send(new InputEvent.MouseButton(b, false, r.x, r.y));
            }
            if (Mouse.LeftButton == MouseButtonState.Released && Mouse.RightButton == MouseButtonState.Released && Mouse.MiddleButton == MouseButtonState.Released) h.ReleaseMouseCapture();
            e.Handled = true;
        };
        // 滚轮一格 120 → 40 像素（与 Mac 端按行换算一致）
        h.MouseWheel += (_, e) => { SyncModifiers(); Send(new InputEvent.Wheel(0, e.Delta / 3)); e.Handled = true; };
        h.ContextMenuOpening += (_, e) => e.Handled = true;
        h.PreviewKeyDown += (_, e) =>
        {
            // 输入法要处理的键交给输入法（不发 HID），组好的文字从 TextInput 来
            if (e.Key == Key.ImeProcessed) { imeComposing = true; e.Handled = false; return; }
            // 正在组字时来了输入法不处理的键（如韩文输入时按空格、标点结束组字）：输入法随后才交出文字，
            // 这个键和之后的输入先暂存，文字到了（最多等 80 毫秒）插到它前面再一起发
            if (imeComposing)
            {
                imeComposing = false;
                imeWait = new TaskCompletionSource<bool>();
                HoldInput(imeWait.Task, 80, ime: true);
            }
            SyncModifiers();
            if (HidKeyMap.Hid(e) is { } hid)
            {
                if (!e.IsRepeat && IsPasteKey(e)) SyncBeforePaste();
                // 修饰键本身已由 SyncModifiers 按实际状态发过；普通键每次（含按住的重复）都发
                if (!IsModifierHid(hid) || pressed.Add(hid))
                {
                    pressed.Add(hid);
                    Send(new InputEvent.Key(hid, true));
                }
            }
            // 触摸键盘、按 Unicode 打字的工具送来的是 VK_PACKET（Key.None，没有对应的 HID 键）：不拦，字符从 TextInput 按 Text 发
            else if (e.Key == Key.None)
            {
                e.Handled = false;
                return;
            }
            // 不让 Tab、方向键、Alt 等触发本地的焦点切换与菜单
            e.Handled = true;
        };
        h.PreviewKeyUp += (_, e) =>
        {
            if (e.Key == Key.ImeProcessed) return;
            if (HidKeyMap.Hid(e) is { } hid)
            {
                // 只给发过「按下」的发抬起（修饰键可能已由 SyncModifiers 抬过）
                if (pressed.Remove(hid)) Send(new InputEvent.Key(hid, false));
            }
            SyncModifiers();
            e.Handled = true;
        };
        // 输入法提交的文字按 Text 发送。普通键在 PreviewKeyDown 里已按 HID 发过并标记已处理，系统不会再为它们产生字符，
        // 所以这里来的都是输入法给的：中文、日文一次提交可能拆成几次 TextInput（逐字），韩文每组完一个音节交一次，全都要发
        h.TextInput += (_, e) =>
        {
            if (string.IsNullOrEmpty(e.Text)) return;
            imeComposing = false;
            SendImeText(e.Text);
            e.Handled = true;
        };
        // 失去焦点时把按住的键都抬起来，避免远端卡住修饰键
        h.LostKeyboardFocus += (_, _) => ReleaseAllInput();
        // 应用整个切到后台（Alt+Tab、按 Win 弹开始菜单）时也全部抬起
        if (Application.Current != null) Application.Current.Deactivated += OnAppDeactivated;
    }

    // ---------- 全屏 ----------

    public void EnterFullscreen()
    {
        if (FullscreenWindow != null) return;
        Region.Detach(VideoHost);
        var grid = new Grid { Background = Brushes.Black };
        grid.Children.Add(VideoHost);
        // 连接中、正在重新连接、失败的提示全屏时也要看得到
        grid.Children.Add(new Region(() => RemoteView.PhaseOverlay(this)).On(this));
        var exit = K.TintButton(L.T("remote.exitFullscreen"), ExitFullscreen, Colors.White, "fit", 30);
        exit.Background = Th.B(Colors.Black, 0.55);
        exit.Normal = Th.B(Colors.Black, 0.55);
        exit.HoverBrush = Th.B(Colors.Black, 0.75);
        exit.HorizontalAlignment = HorizontalAlignment.Right;
        exit.VerticalAlignment = VerticalAlignment.Top;
        exit.Margin = new Thickness(12);
        exit.Opacity = 0.85;
        grid.Children.Add(exit);
        grid.Children.Add(new Region(() => FileClipView.Pill(this)).On(ClipChanged));
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
        // 关标签：先结束录像拿到保存位置，再关画面；等着的自动重连作废
        StopRecording();
        if (Application.Current != null) Application.Current.Deactivated -= OnAppDeactivated;
        ReleaseAllInput();
        closed = true;
        CancelReconnect();
        StopAudio();
        try { channel?.Stop(); } catch { }
        channel = null;
        stats?.Stop();
        clipTimer?.Stop();
        CancelFileClip();
        CancelPaste();
        VirtualUi?.Close();
        // 同一会话可能还被终端、文件标签用着：删掉本标签加的虚拟屏（会话随后断开的话被控端也会删）
        var vp = Peer;
        if (vp != null && vp.ClosedReason == null && MyVirtual.Count > 0)
        {
            var ids = MyVirtual.Keys.ToList();
            MyVirtual.Clear();
            _ = Task.Run(async () =>
            {
                foreach (var id in ids)
                {
                    try { await vp.Session.RemoveVirtualDisplay(id); } catch { }
                }
            });
        }
        queue.CompleteAdding();
        FullscreenWindow?.Close();
    }

    public void Disconnect() => App.CloseTab(Id);

    protected override FrameworkElement CreateView() => RemoteView.Build(App, this);

    /// <summary>一路画面的回调（内核线程）。换掉或丢掉这一路后（tab.sink 不再是它）迟到的回调全部丢弃</summary>
    private sealed class SinkAdapter : VideoSink
    {
        private readonly WeakReference<RemoteTab> owner;
        private readonly int gen;
        public SinkAdapter(RemoteTab t, int gen) { owner = new WeakReference<RemoteTab>(t); this.gen = gen; }
        private RemoteTab? Tab => owner.TryGetTarget(out var t) && ReferenceEquals(t.sink, this) ? t : null;

        public void OnConfig(VideoCodec codec, uint width, uint height) => Ui.Post(() => Tab?.Configured(codec, width, height));
        public void OnFrame(byte[] data, ulong ptsUs, bool keyframe) => Tab?.FrameArrived(gen, data, keyframe);
        public void OnCursor(int x, int y, bool visible) { }
        public void OnClosed(string reason) => Ui.Post(() => Tab?.PictureEnded(reason));
    }
}

internal static class RemoteView
{
    public static FrameworkElement Build(AppModel app, RemoteTab tab)
    {
        // 加虚拟屏的按钮与弹出层跟着视图换（换主题时按新配色重建）
        tab.VirtualUi?.Close();
        tab.VirtualUi = new VirtualDisplayPopover(tab);
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
        // 全屏时画面在全屏窗口里，提示也跟过去
        area.Children.Add(new Region(() => tab.FullscreenWindow == null ? PhaseOverlay(tab) : null).On(tab));
        area.Children.Add(new Region(() => FileClipView.Pill(tab)).On(tab.ClipChanged));
        dock.Children.Add(area);
        return dock;
    }

    /// <summary>画面上的状态提示：连接中、正在重新连接（最后一帧压暗留着，挡住键鼠）、失败（原因 + 重试）</summary>
    public static FrameworkElement? PhaseOverlay(RemoteTab tab) => tab.Phase switch
    {
        RemotePhase.Connecting => K.Center(K.Col(10, K.Center(K.Spinner(22, Brushes.White)), K.Center(K.Text(L.T("remote.connecting"), 13, Th.B(Colors.White, 0.75))))),
        RemotePhase.Reconnecting => K.Center(K.Col(10, K.Center(K.Spinner(22, Brushes.White)), K.Center(K.Text(L.T("remote.reconnecting"), 13, Th.B(Colors.White, 0.85)))))
            .Also(g => ((Grid)g).Background = Th.B(Colors.Black, 0.55)),
        // 失败：白字压在最后一帧上，同样压暗
        RemotePhase.Failed => K.Center(K.Col(12, K.Center(K.Icon("display", 32, Th.B(Colors.White, 0.7), 1.4)),
            K.Center(K.Text(tab.FailReason, 13.5, Th.B(Colors.White, 0.85), wrap: true).Also(t => { t.TextAlignment = TextAlignment.Center; t.MaxWidth = 420; })),
            K.Center(K.PrimaryButton(L.T("common.retry"), tab.Start))))
            .Also(g => ((Grid)g).Background = Th.B(Colors.Black, 0.55)),
        _ => null,
    };

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
                var mine = tab.IsMyVirtual(d.Id);
                var b = new Btn
                {
                    Height = 26, Padding = new Thickness(8, 0, 8, 0), CornerRadius = new CornerRadius(6), Normal = sel ? Th.B(Th.P.Accent, 0.14) : Brushes.Transparent, HoverBrush = sel ? Th.B(Th.P.Accent, 0.2) : Th.Hover2,
                    Child = K.Row(4, K.Icon(d.IsVirtual ? "fit" : "display", 12, sel ? Th.Accent : Th.Secondary), K.Text((i + 1).ToString(CultureInfo.InvariantCulture), 12.5, sel ? Th.Accent : Th.Secondary, sel ? FontWeights.SemiBold : FontWeights.Normal)),
                };
                b.UpdateLook();
                K.Tip(b, L.T("remote.screen", i + 1) + $" · {d.Name} · {d.Width}×{d.Height}" + (d.IsVirtual ? " · " + L.T(mine ? "remote.vd.mine.badge" : "remote.vd.badge") : ""));
                var id = d.Id;
                b.Click += () => Ui.Run(() => tab.Open(id));
                // 我加的虚拟屏：右键可删
                if (mine) K.ContextMenuFor(b, () => new List<MenuEntry> { new(L.T("remote.vd.remove"), () => Ui.Run(() => tab.RemoveVirtualDisplay(id)), Enabled: !tab.VirtualBusy) });
                screens.Children.Add(b);
            }
            items.Add(screens);
        }
        // 加虚拟屏：支持时始终显示（只有一块屏时也在）
        tab.VirtualUi?.Sync();
        if (tab.VirtualSupported && tab.VirtualUi != null) items.Add(tab.VirtualUi.Anchor);
        if (tab.Displays.Length > 1 || tab.VirtualSupported) items.Add(Divider());
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
        items.Add(K.IconButton("clipboard", L.T("remote.clipboard"), () => tab.SetClipboardSync(!tab.ClipboardSync), tab.ClipboardSync));
        items.Add(K.Info(L.T("remote.clipboard.info")));
        if (tab.AudioSupported) items.Add(K.IconButton(tab.AudioOn ? "volume" : "volumeOff", L.T("remote.sound"), tab.ToggleAudio, tab.AudioOn));
        items.Add(RecordButton(tab));
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

    /// <summary>录像：没在录时是圆点图标；录制中是红点 + 已录时长的胶囊，点一下停止</summary>
    private static FrameworkElement RecordButton(RemoteTab tab)
    {
        if (!tab.Recording)
        {
            var b = K.IconButton("record", L.T("remote.record.start"), tab.ToggleRecording, size: 14);
            b.IsEnabled = tab.Phase == RemotePhase.Streaming && !tab.RecordingBusy;
            return b;
        }
        var elapsed = Fmt.Elapsed(Math.Max(0, Clock.NowMs - tab.RecordingSinceMs));
        var pill = new Btn
        {
            Height = 26, CornerRadius = new CornerRadius(13), Padding = new Thickness(9, 0, 10, 0),
            Normal = Th.B(Th.P.Danger, 0.12), HoverBrush = Th.B(Th.P.Danger, 0.2), PressBrush = Th.B(Th.P.Danger, 0.28),
            Child = K.Row(6, new System.Windows.Shapes.Ellipse { Width = 8, Height = 8, Fill = Th.Danger },
                K.Text(elapsed, 12.5, Th.Danger, FontWeights.SemiBold, mono: true), K.Icon("stop", 9, Th.Danger)),
        };
        pill.UpdateLook();
        K.Tip(pill, L.T("remote.record.stop"));
        pill.Click += tab.ToggleRecording;
        return pill;
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
