using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Effects;
using System.Windows.Threading;

namespace LemonGalaxy.UI;

/// <summary>虚拟屏的模式（宽 × 高 @ 刷新率），范围与内核 galaxy_proto::peer::virtual_display_mode_ok 一致</summary>
internal readonly record struct VirtualDisplayMode(uint Width, uint Height, uint Refresh)
{
    /// <summary>常用分辨率：16:9 四档 + 16:10 两档</summary>
    public static readonly (uint W, uint H)[] Presets = { (1280, 720), (1920, 1080), (2560, 1440), (3840, 2160), (1920, 1200), (2560, 1600) };
    public static readonly uint[] RefreshPresets = { 30, 60, 120, 144 };

    public static bool WidthValid(uint w) => w is >= 640 and <= 7680;
    public static bool HeightValid(uint h) => h is >= 480 and <= 4320;
    public static bool SizeValid(uint w, uint h) => WidthValid(w) && HeightValid(h);
    public static bool RefreshValid(uint hz) => hz is >= 24 and <= 240;

    public bool Valid => SizeValid(Width, Height) && RefreshValid(Refresh);
    public string SizeLabel => SizeText(Width, Height);
    public string Label => SizeLabel + " @ " + Refresh.ToString(CultureInfo.InvariantCulture) + " Hz";

    public static string SizeText(uint w, uint h) => w.ToString(CultureInfo.InvariantCulture) + "×" + h.ToString(CultureInfo.InvariantCulture);

    /// <summary>selftest 固定「本机屏幕」这一档（截图与机器无关）</summary>
    public static (uint W, uint H)? LocalOverride;

    /// <summary>本机主屏的物理像素（程序按每显示器 DPI 感知运行，Screen.Bounds 就是物理像素）；超出范围为 null，界面不给这一档</summary>
    public static (uint W, uint H)? LocalScreen
    {
        get
        {
            if (LocalOverride is { } o) return o;
            try
            {
                var screen = System.Windows.Forms.Screen.PrimaryScreen;
                if (screen == null || screen.Bounds.Width <= 0 || screen.Bounds.Height <= 0) return null;
                uint w = (uint)screen.Bounds.Width, h = (uint)screen.Bounds.Height;
                return SizeValid(w, h) ? (w, h) : null;
            }
            catch { return null; }
        }
    }
}

internal enum VdSizeKind { Preset, Local, Custom }

/// <summary>加屏表单（随标签保留：收起弹出层再打开还是上次的选择）。默认 1920×1080 @ 60</summary>
internal sealed class VirtualDisplayForm : Observable
{
    public VdSizeKind SizeKind = VdSizeKind.Preset;
    public (uint W, uint H) Preset = (1920, 1080);
    public bool CustomRefresh;
    public uint RefreshPreset = 60;
    public string Width = "1920", Height = "1080", Hz = "60";

    public static uint? Parse(string s) => uint.TryParse(s, NumberStyles.None, CultureInfo.InvariantCulture, out var v) ? v : null;

    public bool CustomWidthValid => Parse(Width) is { } w && VirtualDisplayMode.WidthValid(w);
    public bool CustomHeightValid => Parse(Height) is { } h && VirtualDisplayMode.HeightValid(h);
    public bool CustomHzValid => Parse(Hz) is { } hz && VirtualDisplayMode.RefreshValid(hz);

    /// <summary>当前选的模式；自定义值不合法（或本机屏幕不在范围内）时为 null</summary>
    public VirtualDisplayMode? Mode
    {
        get
        {
            (uint W, uint H)? size = SizeKind switch
            {
                VdSizeKind.Local => VirtualDisplayMode.LocalScreen,
                VdSizeKind.Custom => CustomWidthValid && CustomHeightValid ? (Parse(Width)!.Value, Parse(Height)!.Value) : ((uint W, uint H)?)null,
                _ => Preset,
            };
            uint? hz = CustomRefresh ? (CustomHzValid ? Parse(Hz) : null) : RefreshPreset;
            if (size is not { } s || hz is not { } r) return null;
            return new VirtualDisplayMode(s.W, s.H, r);
        }
    }

    /// <summary>界面结构（选了哪一项）；输入数字不算，输入时不重建表单</summary>
    public string StructureKey => $"{SizeKind}:{Preset.W}x{Preset.H}:{CustomRefresh}:{RefreshPreset}";

    public void PickSize(VdSizeKind kind, (uint W, uint H) preset = default)
    {
        SizeKind = kind;
        if (kind == VdSizeKind.Preset) Preset = preset;
        Notify();
    }

    public void PickRefresh(uint? preset)
    {
        CustomRefresh = preset == null;
        if (preset is { } p) RefreshPreset = p;
        Notify();
    }

    public void Reset()
    {
        SizeKind = VdSizeKind.Preset;
        Preset = (1920, 1080);
        CustomRefresh = false;
        RefreshPreset = 60;
        Width = "1920";
        Height = "1080";
        Hz = "60";
        Notify();
    }
}

// ---------- 模型：加屏、删屏（docs/protocol.md 8.6） ----------

internal sealed partial class RemoteTab
{
    /// <summary>本会话最多能加几块虚拟屏（0 = 不支持或没有控制权限，不显示入口）；每次拿到会话重新取</summary>
    public uint VirtualMax;
    /// <summary>本会话加的虚拟屏（id → 加的时候选的模式），只有这些能删；会话换了或断了清空（被控端随旧会话已删掉）</summary>
    public readonly Dictionary<uint, VirtualDisplayMode> MyVirtual = new();
    public bool VirtualBusy;
    public readonly VirtualDisplayForm VirtualForm = new();
    /// <summary>工具栏按钮与弹出层（视图重建时换新的）</summary>
    public VirtualDisplayPopover? VirtualUi;

    public bool VirtualSupported => VirtualMax > 0 && Peer is { ClosedReason: null };
    public bool VirtualFull => MyVirtual.Count >= VirtualMax;
    public bool IsMyVirtual(uint id) => MyVirtual.ContainsKey(id);

    private static uint QueryVirtualMax(PeerConnection p)
    {
        try { return p.Session.VirtualDisplayMax(); }
        catch (Exception e)
        {
            Log.Error("取虚拟屏上限失败：" + e.Message);
            return 0;
        }
    }

    /// <summary>会话换了或断了：我加的清空，弹出层收起</summary>
    private void ResetVirtual()
    {
        MyVirtual.Clear();
        VirtualUi?.Close();
    }

    /// <summary>列表里已经没有的（被控端那边没了）不再算我加的</summary>
    private void PruneVirtual()
    {
        foreach (var id in MyVirtual.Keys.Where(id => Displays.All(d => d.Id != id)).ToList()) MyVirtual.Remove(id);
    }

    /// <summary>在被控端加一块虚拟屏：成功后记进我加的、刷新屏幕列表并切过去看。失败时提示原因</summary>
    public async Task<bool> AddVirtualDisplay(VirtualDisplayMode mode)
    {
        var p = Peer;
        if (p == null || closed || VirtualBusy || !mode.Valid || !VirtualSupported || VirtualFull) return false;
        VirtualBusy = true;
        Notify();
        try
        {
            var d = await p.Session.AddVirtualDisplay(mode.Width, mode.Height, mode.Refresh);
            // 等待期间会话换了或断了：被控端随旧会话已删掉，不再理会
            if (closed || !ReferenceEquals(Peer, p) || p.ClosedReason != null) return false;
            MyVirtual[d.Id] = mode;
            await RefreshDisplays(p, d);
            if (closed || !ReferenceEquals(Peer, p)) return false;
            await Open(d.Id);
            App.ShowToast(L.T("remote.vd.added", mode.SizeLabel));
            return true;
        }
        catch (Exception e)
        {
            if (!closed) App.ShowToast(L.T("remote.vd.addFailed", Fmt.Error(e)), true);
            return false;
        }
        finally
        {
            VirtualBusy = false;
            Notify();
        }
    }

    /// <summary>删掉本会话加的虚拟屏；正在看它就先切回主屏（没有主屏就第一块），免得画面先断。不用二次确认（断开时本来也会删，误删可再加）</summary>
    public async Task RemoveVirtualDisplay(uint id)
    {
        var p = Peer;
        if (p == null || closed || VirtualBusy || !IsMyVirtual(id)) return;
        VirtualBusy = true;
        Notify();
        try
        {
            if (DisplayId == id && (Displays.FirstOrDefault(d => d.Primary && d.Id != id) ?? Displays.FirstOrDefault(d => d.Id != id)) is { } next)
                await Open(next.Id);
            await p.Session.RemoveVirtualDisplay(id);
            if (closed || !ReferenceEquals(Peer, p)) return;
            MyVirtual.Remove(id);
            await RefreshDisplays(p, null);
            // 被控端拔屏是异步的，刚删的可能还在列表里
            Displays = Displays.Where(d => d.Id != id).ToArray();
            App.ShowToast(L.T("remote.vd.removed"));
        }
        catch (GalaxyException.NotFound) when (!closed && ReferenceEquals(Peer, p))
        {
            // 被控端那边已经没有这块了（例如换了控制台会话）：当作已删掉
            MyVirtual.Remove(id);
            await RefreshDisplays(p, null);
            App.ShowToast(L.T("remote.vd.removed"));
        }
        catch (Exception e)
        {
            if (!closed) App.ShowToast(L.T("remote.vd.removeFailed", Fmt.Error(e)), true);
        }
        finally
        {
            VirtualBusy = false;
            Notify();
        }
    }

    /// <summary>重新列屏幕。added：刚加的屏（被控端系统里可能还没出现，先放进列表）</summary>
    private async Task RefreshDisplays(PeerConnection p, DisplayInfo? added)
    {
        List<DisplayInfo> list;
        try { list = (await p.Session.ListDisplays()).ToList(); }
        catch (Exception e)
        {
            Log.Error("列屏幕失败：" + Fmt.Error(e));
            list = Displays.ToList();
        }
        if (closed || !ReferenceEquals(Peer, p)) return;
        if (added != null && list.All(d => d.Id != added.Id)) list.Add(added);
        Displays = list.ToArray();
        PruneVirtual();
        Notify();
    }
}

// ---------- 工具栏按钮 + 弹出层 ----------

/// <summary>
/// 工具栏多屏切换旁的「加虚拟屏」按钮与弹出层（加屏表单 + 我加的虚拟屏列表）。
/// 工具栏随帧率统计每秒重建，按钮和弹出层常驻在这里（视图重建时换新的），工具栏重建时只把同一个按钮挂上去，弹出层与输入不受影响。
/// </summary>
internal sealed class VirtualDisplayPopover
{
    private readonly RemoteTab tab;
    public readonly Btn Anchor;
    private readonly Popup popup;
    private int closedAt = Environment.TickCount - 10_000;
    private bool? lastActive;

    public VirtualDisplayPopover(RemoteTab tab)
    {
        this.tab = tab;
        Anchor = new Btn { Width = 28, Height = 26, CornerRadius = new CornerRadius(6) };
        K.Tip(Anchor, L.T("remote.vd.add"));
        Anchor.Click += Toggle;
        popup = new Popup
        {
            PlacementTarget = Anchor, Placement = PlacementMode.Bottom, StaysOpen = false, AllowsTransparency = true, PopupAnimation = PopupAnimation.Fade,
        };
        popup.Opened += (_, _) => Sync();
        popup.Closed += (_, _) =>
        {
            closedAt = Environment.TickCount;
            Sync();
        };
        Sync();
    }

    public bool IsOpen => popup.IsOpen;

    /// <summary>工具栏重建时调用：刷新按钮外观（弹出层开着或加过虚拟屏时高亮）；按钮换了位置时让弹出层跟过去</summary>
    public void Sync()
    {
        if (!tab.VirtualSupported && popup.IsOpen) popup.IsOpen = false;
        var active = popup.IsOpen || tab.MyVirtual.Count > 0;
        if (lastActive != active)
        {
            lastActive = active;
            Anchor.Normal = active ? Th.B(Th.P.Accent, 0.14) : Brushes.Transparent;
            Anchor.HoverBrush = active ? Th.B(Th.P.Accent, 0.2) : Th.Hover2;
            Anchor.Child = K.Icon("displayPlus", 14, active ? Th.Accent : Th.Secondary);
            Anchor.UpdateLook();
        }
        if (popup.IsOpen)
            Ui.Dispatcher.BeginInvoke(DispatcherPriority.Loaded, new Action(() =>
            {
                if (!popup.IsOpen) return;
                var o = popup.HorizontalOffset;
                popup.HorizontalOffset = o + 0.5;
                popup.HorizontalOffset = o;
            }));
    }

    private void Toggle()
    {
        if (popup.IsOpen)
        {
            Close();
            return;
        }
        // 点按钮本身关掉弹出层时，这次点击不再打开
        if (unchecked(Environment.TickCount - closedAt) < 300) return;
        Open();
    }

    /// <summary>每次打开都按当前状态与配色重建内容</summary>
    public void Open()
    {
        if (!tab.VirtualSupported) return;
        popup.Child = RemoteVirtualDisplayView.Card(RemoteVirtualDisplayView.Panel(tab, Close), Close);
        popup.IsOpen = true;
    }

    public void Close() => popup.IsOpen = false;
}

internal static class RemoteVirtualDisplayView
{
    /// <summary>弹出层的卡片外壳（Esc 收起）</summary>
    public static Border Card(UIElement content, Action? close)
    {
        var card = new Border
        {
            Margin = new Thickness(0, 4, 12, 12), Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10),
            Padding = new Thickness(16, 14, 16, 16), Width = 340, SnapsToDevicePixels = true,
            Effect = new DropShadowEffect { BlurRadius = 16, ShadowDepth = 3, Opacity = 0.22, Direction = 270 },
            Child = content,
        };
        if (close != null)
            card.PreviewKeyDown += (_, e) =>
            {
                if (e.Key != Key.Escape) return;
                close();
                e.Handled = true;
            };
        return card;
    }

    /// <summary>selftest：离屏渲染不了 Popup，单独渲染弹出层的卡片</summary>
    public static FrameworkElement Preview(RemoteTab tab)
        => new Border { Padding = new Thickness(16, 8, 4, 4), HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Top, Child = Card(Panel(tab, () => { }), null) };

    /// <summary>弹出层内容：选项、进行中、我加的列表、正在看的屏变化时重建；输入数字时不重建（保持焦点），只刷新底部的范围提示与按钮</summary>
    public static FrameworkElement Panel(RemoteTab tab, Action onAdded)
    {
        var form = tab.VirtualForm;
        return new Region(() => Build(tab, form, onAdded)).Keyed(() => PanelKey(tab, form)).On(tab, form);
    }

    private static string PanelKey(RemoteTab tab, VirtualDisplayForm form)
        => string.Join("|", tab.VirtualMax, tab.VirtualBusy, tab.DisplayId, string.Join(",", tab.Displays.Select(d => d.Id)), string.Join(",", tab.MyVirtual.Keys), form.StructureKey);

    private static FrameworkElement Build(RemoteTab tab, VirtualDisplayForm form, Action onAdded)
    {
        var local = VirtualDisplayMode.LocalScreen;
        if (form.SizeKind == VdSizeKind.Local && local == null) form.SizeKind = VdSizeKind.Preset;
        void Submit()
        {
            if (CanAdd(tab, form)) Add(tab, form, onAdded);
        }

        var full = tab.VirtualFull;
        var count = K.Text(full ? L.T("remote.vd.full", tab.VirtualMax) : L.T("remote.vd.count", tab.MyVirtual.Count, tab.VirtualMax), 11.5, full ? Th.Warning : Th.Tertiary);
        var col = new List<UIElement?> { K.Row(6, K.Semi(L.T("remote.vd.title"), 13.5), K.Info(L.T("remote.vd.info")), K.Spacer(8), count) };

        // 分辨率：常用档位 + 本机屏幕 + 自定义（两列）
        col.Add(K.Margin(Label(L.T("remote.vd.resolution")), 0, 14, 0, 6));
        var sizes = new UniformGrid { Columns = 2, Margin = new Thickness(-3) };
        foreach (var p in VirtualDisplayMode.Presets)
            sizes.Children.Add(Option(VirtualDisplayMode.SizeText(p.W, p.H), null, form.SizeKind == VdSizeKind.Preset && form.Preset == p, () => form.PickSize(VdSizeKind.Preset, p)));
        if (local is { } l) sizes.Children.Add(Option(L.T("remote.vd.local"), VirtualDisplayMode.SizeText(l.W, l.H), form.SizeKind == VdSizeKind.Local, () => form.PickSize(VdSizeKind.Local)));
        sizes.Children.Add(Option(L.T("remote.vd.custom"), null, form.SizeKind == VdSizeKind.Custom, () => form.PickSize(VdSizeKind.Custom)));
        col.Add(sizes);
        if (form.SizeKind == VdSizeKind.Custom)
        {
            var w = NumberField(form.Width, L.T("remote.vd.width"), 96, Submit);
            var h = NumberField(form.Height, L.T("remote.vd.height"), 96, Submit);
            void Check()
            {
                w.Invalid = form.Width.Length > 0 && !form.CustomWidthValid;
                h.Invalid = form.Height.Length > 0 && !form.CustomHeightValid;
            }
            Check();
            w.Changed += t => { form.Width = t; Check(); form.Notify(); };
            h.Changed += t => { form.Height = t; Check(); form.Notify(); };
            col.Add(K.Margin(K.Row(8, w, K.Text("×", 13, Th.Secondary), h), 0, 10, 0, 0));
        }

        // 刷新率：30 / 60 / 120 / 144 Hz + 自定义（一行）
        col.Add(K.Margin(Label(L.T("remote.vd.refresh")), 0, 16, 0, 6));
        var rates = new UniformGrid { Columns = VirtualDisplayMode.RefreshPresets.Length + 1, Margin = new Thickness(-3) };
        foreach (var hz in VirtualDisplayMode.RefreshPresets)
            rates.Children.Add(Option(hz.ToString(CultureInfo.InvariantCulture) + " Hz", null, !form.CustomRefresh && form.RefreshPreset == hz, () => form.PickRefresh(hz)));
        rates.Children.Add(Option(L.T("remote.vd.custom"), null, form.CustomRefresh, () => form.PickRefresh(null)));
        col.Add(rates);
        if (form.CustomRefresh)
        {
            var f = NumberField(form.Hz, "60", 72, Submit);
            void Check() => f.Invalid = form.Hz.Length > 0 && !form.CustomHzValid;
            Check();
            f.Changed += t => { form.Hz = t; Check(); form.Notify(); };
            col.Add(K.Margin(K.Row(8, f, K.Text("Hz", 12.5, Th.Secondary)), 0, 10, 0, 0));
        }

        col.Add(new Region(() => Footer(tab, form, onAdded)).Keyed(() => (form.Mode, CanAdd(tab, form), tab.VirtualBusy)).On(tab, form));

        // 我加的虚拟屏：切过去看、删掉
        var mine = tab.Displays.Where(d => tab.IsMyVirtual(d.Id)).ToList();
        if (mine.Count > 0)
        {
            col.Add(K.Margin(K.HLine(), 0, 16, 0, 12));
            col.Add(K.Margin(Label(L.T("remote.vd.mine")), 0, 0, 0, 6));
            col.Add(K.Col(2, mine.Select(d => (UIElement?)MineRow(tab, d))));
        }
        return K.Col(0, col);
    }

    private static TextBlock Label(string text) => K.Text(text, 12, Th.Secondary, FontWeights.Medium);

    private static bool CanAdd(RemoteTab tab, VirtualDisplayForm form) => form.Mode != null && tab.VirtualSupported && !tab.VirtualFull && !tab.VirtualBusy;

    private static void Add(RemoteTab tab, VirtualDisplayForm form, Action onAdded)
    {
        if (form.Mode is not { } m) return;
        Ui.Run(async () =>
        {
            if (await tab.AddVirtualDisplay(m)) onAdded();
        });
    }

    /// <summary>范围提示（自定义值不合法时）+ 主按钮（进行中转圈）</summary>
    private static FrameworkElement Footer(RemoteTab tab, VirtualDisplayForm form, Action onAdded)
    {
        var title = L.T("remote.vd.add");
        var b = K.PrimaryButton(title, () => Add(tab, form, onAdded), enabled: CanAdd(tab, form));
        b.Height = 32;
        FrameworkElement content = tab.VirtualBusy
            ? K.Row(7, K.Spinner(13, Th.AccentText), K.Text(title, 13, Th.AccentText, FontWeights.SemiBold))
            : K.Text(title, 13, Th.AccentText, FontWeights.SemiBold);
        content.HorizontalAlignment = HorizontalAlignment.Center;
        content.VerticalAlignment = VerticalAlignment.Center;
        b.Child = content;
        var hint = form.Mode == null ? K.Text(L.T("remote.vd.range"), 11.5, Th.Danger, wrap: true) : null;
        return K.Margin(K.Col(8, hint, b), 0, hint == null ? 18 : 10, 0, 0);
    }

    /// <summary>单选项：选中时强调色描边与文字</summary>
    private static Btn Option(string title, string? detail, bool selected, Action pick)
    {
        var content = K.Row(5, K.Text(title, 12, selected ? Th.Accent : Th.Text, selected ? FontWeights.SemiBold : FontWeights.Normal),
            detail == null ? null : K.Text(detail, 11, selected ? Th.B(Th.P.Accent, 0.8) : Th.Tertiary));
        content.HorizontalAlignment = HorizontalAlignment.Center;
        var b = new Btn
        {
            Height = 28, Margin = new Thickness(3), CornerRadius = new CornerRadius(7), BorderThickness = new Thickness(1), Padding = new Thickness(6, 0, 6, 0),
            Normal = selected ? Th.B(Th.P.Accent, 0.12) : Th.Elevated,
            HoverBrush = selected ? Th.B(Th.P.Accent, 0.16) : Th.B(K.Mix(Th.P.Elevated, Th.P.Text, 0.05)),
            NormalBorder = selected ? Th.Accent : Th.Border,
            Child = content,
        };
        b.UpdateLook();
        b.Click += pick;
        return b;
    }

    /// <summary>只能输入数字的输入框（最多 4 位），Enter 提交</summary>
    private static Field NumberField(string text, string placeholder, double width, Action submit)
    {
        var f = new Field(placeholder, text, mono: true) { Width = width };
        K.Tip(f, placeholder);
        if (f.Box is { } box)
        {
            box.MaxLength = 4;
            box.PreviewTextInput += (_, e) => e.Handled = !IsDigits(e.Text);
            // 空格不走 PreviewTextInput
            box.PreviewKeyDown += (_, e) => { if (e.Key == Key.Space) e.Handled = true; };
            DataObject.AddPastingHandler(box, (_, e) =>
            {
                if (e.DataObject.GetData(typeof(string)) is not string s || !IsDigits(s)) e.CancelCommand();
            });
        }
        f.Submitted += submit;
        return f;
    }

    private static bool IsDigits(string s) => s.All(c => c is >= '0' and <= '9');

    /// <summary>我加的虚拟屏一行：屏号、模式、切过去看、删掉</summary>
    private static FrameworkElement MineRow(RemoteTab tab, DisplayInfo d)
    {
        var index = Array.FindIndex(tab.Displays, x => x.Id == d.Id) + 1;
        var viewing = tab.DisplayId == d.Id;
        var mode = tab.MyVirtual.TryGetValue(d.Id, out var m) ? m.Label : VirtualDisplayMode.SizeText(d.Width, d.Height);
        var id = d.Id;
        var view = viewing ? null : K.IconButton("eye", L.T("remote.vd.view"), () => Ui.Run(() => tab.Open(id)), size: 13);
        var remove = K.IconButton("trash", L.T("remote.vd.remove"), () => Ui.Run(() => tab.RemoveVirtualDisplay(id)), size: 13);
        remove.IsEnabled = !tab.VirtualBusy;
        var text = K.Col(1, K.Text(L.T("remote.screen", index), 12.5, Th.Text, viewing ? FontWeights.SemiBold : FontWeights.Normal), K.Text(mode, 11, Th.Tertiary, mono: true));
        return new Border
        {
            Padding = new Thickness(8, 5, 4, 5), CornerRadius = new CornerRadius(7), Background = viewing ? Th.B(Th.P.Accent, 0.08) : Brushes.Transparent,
            Child = K.Row(9, K.Icon("fit", 14, viewing ? Th.Accent : Th.Secondary), K.Grow(text), view, remove),
        };
    }
}
