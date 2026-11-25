using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using LemonGalaxy.Core;
using LemonGalaxy.Core.Mock;
using LemonGalaxy.UI;
using ComTypes = System.Runtime.InteropServices.ComTypes;

namespace LemonGalaxy;

/// <summary>
/// --selftest &lt;输出目录&gt; [--scale 1.5] [--only 名称片段] [--lang zh-Hans|en] [--theme light|dark]
/// 用模拟内核把主要界面离屏渲染成 PNG：浅色 / 深色 × 中文 / 英文。不显示窗口（元素不挂到任何窗口上，直接 RenderTargetBitmap），
/// 不连网络、不读写用户设置与系统安全存储。退出码：0 全部成功；1 有问题（见输出目录里的 report.txt）。
/// </summary>
internal static class SelfTest
{
    private static readonly Size WindowSize = new(1280, 800);
    private static double scale = 1;
    private static string dir = "selftest";
    private static string? only;
    private static int count;
    private static readonly List<string> Problems = new();
    /// <summary>本机一侧的假主目录（复制粘贴文件从这里取文件）</summary>
    private static string fakeHomeDir = "";

    public static int Run(string outputDir, string[] args)
    {
        string? Arg(string name)
        {
            var i = Array.IndexOf(args, name);
            return i >= 0 && i + 1 < args.Length ? args[i + 1] : null;
        }
        try
        {
            scale = double.TryParse(Arg("--scale"), NumberStyles.Float, CultureInfo.InvariantCulture, out var sc) && sc > 0 ? sc : 1;
            only = Arg("--only");
            dir = IOPath.GetFullPath(outputDir);
            Directory.CreateDirectory(dir);
            Log.FilePath = IOPath.Combine(dir, "ui.log");
            if (File.Exists(Log.FilePath)) File.Delete(Log.FilePath);

            var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
            L.LoadPacks();
            if (L.Packs.Count == 0) Problems.Add("没有找到语言包（Resources\\Locales）");
            if (TerminalTheme.All.Count < 2) Problems.Add("终端主题少于 2 个（Resources\\TerminalThemes）");
            var settings = AppSettings.Shared;
            settings.ResetForSelfTest();
            Th.Apply(settings);
            Styles.Install(app);
            if (Styles.LoadError != null) Problems.Add("控件模板解析失败：" + Styles.LoadError);
            CheckIcons();
            CheckForwardDraft();
            CheckMarkdown();
            Clock.FrozenMs = 1_795_225_692_000;      // 2026-11-21 09:48:12（北京时间）
            VirtualDisplayMode.LocalOverride = (2736, 1824);
            var fakeHome = MakeFakeHome();
            LocalFileSource.HomeOverride = fakeHome;
            fakeHomeDir = fakeHome;
            // 复制粘贴文件下载到临时目录，不碰 %LOCALAPPDATA%；系统剪贴板 selftest 不读不写
            ClipFiles.RootOverride = IOPath.Combine(IOPath.GetDirectoryName(fakeHome)!, "clipboard-cache");
            CheckClipboardCacheCleanup();

            var langs = Arg("--lang") is { } l ? new[] { l } : new[] { "zh-Hans", "en" };
            var themes = Arg("--theme") is { } t ? new[] { t } : new[] { "light", "dark" };
            foreach (var lang in langs)
                foreach (var theme in themes)
                {
                    try { RenderAll(settings, lang, theme); }
                    catch (Exception e) { Problems.Add($"渲染 {lang}-{theme} 出错：{e}"); }
                }
            IconSheet();
            try { Directory.Delete(IOPath.GetDirectoryName(fakeHome)!, true); } catch { }
        }
        catch (Exception e) { Problems.Add("自测出错：" + e); }

        var missing = L.MissingKeys;
        if (missing.Length > 0) Problems.Add("缺失的语言键：" + string.Join(", ", missing));
        if (Log.ErrorCount > 0) Problems.Add($"界面日志里有 {Log.ErrorCount} 条错误（见 ui.log）");
        var report = new StringBuilder();
        report.AppendLine("LemonGalaxy Windows selftest");
        report.AppendLine($"输出 {count} 张，尺寸 {WindowSize.Width}×{WindowSize.Height} @{scale.ToString(CultureInfo.InvariantCulture)}x");
        report.AppendLine($"系统 {AppInfo.OsVersion}，.NET {Environment.Version}");
        report.AppendLine();
        report.AppendLine($"问题（{Problems.Count}）：");
        foreach (var p in Problems) report.AppendLine(p);
        try { File.WriteAllText(IOPath.Combine(dir, "report.txt"), report.ToString(), new UTF8Encoding(false)); } catch { }
        Console.WriteLine($"selftest: 输出 {count} 张到 {dir}；问题 {Problems.Count} 个");
        foreach (var p in Problems) Console.WriteLine("问题：" + p);
        return Problems.Count == 0 && count > 0 ? 0 : 1;
    }

    private static void CheckIcons()
    {
        foreach (var n in Icons.AllNames)
        {
            try { _ = Icons.Get(n); }
            catch (Exception e) { Problems.Add($"图标 {n} 解析失败：{e.Message}"); }
        }
    }

    /// <summary>端口映射编辑草稿的校验：经由恰好一个、目标主机必填、端口 1–65535、本机端口 0（或留空）为自动</summary>
    private static void CheckForwardDraft()
    {
        void Expect(bool ok, string what) { if (!ok) Problems.Add("端口映射草稿：" + what); }
        var d = new ForwardDraft { RemoteHost = "10.0.0.5", RemotePort = "5432" };
        Expect(!d.IsValid, "没选经由也通过了校验");
        d.Via = "dev:dev-build";
        Expect(d.IsValid, "合法的规则没通过校验");
        Expect(d.LocalPortValue == 0, "本机端口留空应为 0（自动）");
        d.RemotePort = "70000";
        Expect(!d.IsValid, "目标端口 70000 通过了校验");
        d.RemotePort = "0";
        Expect(!d.IsValid, "目标端口 0 通过了校验");
        d.RemotePort = "5432";
        d.LocalPort = "abc";
        Expect(!d.IsValid, "本机端口 abc 通过了校验");
        d.LocalPort = "65536";
        Expect(!d.IsValid, "本机端口 65536 通过了校验");
        d.LocalPort = "0";
        Expect(d.IsValid && d.LocalPortValue == 0, "本机端口 0 应表示自动");
        d.RemoteHost = "  ";
        Expect(!d.IsValid, "目标主机为空也通过了校验");
        d.RemoteHost = "db.internal";
        Expect(d.Build().Data is VaultData.PortForward { ViaDeviceId: "dev-build", ViaSshHostId: null, LocalPort: 0, RemotePort: 5432, Label: "db.internal:5432" }, "经由设备的规则组装不对");
        d.Via = "ssh:s-web1";
        d.LocalPort = "15432";
        Expect(d.Build().Data is VaultData.PortForward { ViaDeviceId: null, ViaSshHostId: "s-web1", LocalPort: 15432 }, "经由 SSH 主机的规则组装不对");
    }

    private static void ClearToast(AppModel app)
    {
        app.CurrentToast = null;
        app.Overlay.Notify();
    }

    /// <summary>启动时清理复制粘贴文件缓存：一天前的目录删掉，新的留着</summary>
    private static void CheckClipboardCacheCleanup()
    {
        var root = ClipFiles.Root;
        var old = IOPath.Combine(root, "1795000000000-123456");
        var fresh = IOPath.Combine(root, "1795225692000-654321");
        foreach (var d in new[] { old, fresh })
        {
            Directory.CreateDirectory(d);
            File.WriteAllText(IOPath.Combine(d, "a.txt"), "x");
        }
        var past = DateTime.Now.AddDays(-2);
        Directory.SetCreationTime(old, past);
        Directory.SetLastWriteTime(old, past);
        var removed = ClipFiles.CleanStale(root);
        if (removed != 1 || Directory.Exists(old) || !Directory.Exists(fresh))
            Problems.Add($"复制粘贴文件缓存清理不对：删了 {removed} 个，旧目录还在 {Directory.Exists(old)}，新目录还在 {Directory.Exists(fresh)}");
        try { Directory.Delete(fresh, true); } catch { }
    }

    /// <summary>
    /// 远程桌面复制粘贴文件（protocol.md 8.4.1，复制时只交换清单，粘贴时才传）：
    /// 本机复制 → 只发清单（文件夹展开、不显示进度条）；对方复制 → 虚拟文件数据对象（复制时不读内容，核对描述表、按项读、Seek、各格式）；
    /// 老被控端整批上传的进度条、取消、完成与失败提示，推过去的暂存文件不再放回本机。selftest 不碰系统剪贴板，直接调。
    /// </summary>
    private static void CheckFileClip(RemoteTab r, FrameworkElement root, string prefix)
    {
        if (r.Peer?.Session is not MockPeer session)
        {
            Problems.Add("复制粘贴文件：没有模拟会话");
            return;
        }
        if (!r.FileClipSupported) Problems.Add("复制粘贴文件：模拟的 Windows 设备应支持");
        if (!r.ClipOfferSupported) Problems.Add("复制粘贴文件：模拟的 Windows 设备应支持只发清单（clipboard_offer）");
        var home = fakeHomeDir;
        var report = IOPath.Combine(home, "report-2026Q3.pdf");
        var notes = IOPath.Combine(home, "deploy-notes.md");
        var held = IOPath.Combine(home, "Desktop", "assets-hold.zip");
        var failing = IOPath.Combine(home, "Desktop", "upload-fail.bin");
        File.WriteAllBytes(held, new byte[2_000_000]);
        File.WriteAllBytes(failing, new byte[300_000]);
        // 要发清单的文件夹放在假主目录外面，不影响文件列表截图
        var folder = IOPath.Combine(IOPath.GetDirectoryName(home)!, "offer", "handoff");
        Directory.CreateDirectory(IOPath.Combine(folder, "assets"));
        File.WriteAllText(IOPath.Combine(folder, "readme.txt"), "handoff");
        File.WriteAllBytes(IOPath.Combine(folder, "assets", "logo.png"), new byte[4096]);
        bool Failed() => r.FileClip?.State == FileClipState.Failed;

        // 1. 本机复制 → 只发清单：不传内容、不显示进度条，对方剪贴板换成清单（拉取时不会再放回本机）
        r.SendLocalFiles(new[] { report, notes, folder });
        Wait("复制文件只发清单", () => session.LastOffer != null || Failed());
        if (session.LastOffer is { } offered)
        {
            var paths = offered.Entries.Select(e => e.Path).ToArray();
            var want = new[] { "report-2026Q3.pdf", "deploy-notes.md", "handoff", "handoff/assets", "handoff/assets/logo.png", "handoff/readme.txt" };
            if (!paths.SequenceEqual(want) || !offered.Entries[2].IsDir || offered.Entries[4].Size != 4096)
                Problems.Add("复制粘贴文件：发出去的清单不对 " + string.Join(", ", paths));
        }
        else Problems.Add("复制粘贴文件：本机复制后没有发清单（" + r.FileClip?.Error + "）");
        if (r.FileClip != null || r.ClipTransfer != null) Problems.Add("复制粘贴文件：只发清单时不应上传或显示进度条");
        if (session.RemoteClipboard != null) Problems.Add("复制粘贴文件：发清单后对方剪贴板应是这份清单");
        r.PullRemoteClipboard();
        Ui.Pump(150);
        if (r.PasteData != null) Problems.Add("复制粘贴文件：对方剪贴板是自己发的清单时不应放回本机");

        // 2. 对方复制 → 虚拟文件放进本机剪贴板，复制时不读内容；同一批不重复放
        var remote = MockClipboardTransfer.RemoteSampleFiles(session.Device, session.Client.Zh);
        session.RemoteClipboard = new ClipboardData.Files(remote);
        r.PullRemoteClipboard();
        Wait("对方文件放进本机剪贴板", () => r.PasteData != null || Failed());
        var placed = r.PasteData;
        if (placed == null) Problems.Add("复制粘贴文件：对方复制的文件没有放进本机剪贴板（" + r.FileClip?.Error + "）");
        else CheckPasteData(placed, remote);
        if (r.FileClip != null) Problems.Add("复制粘贴文件：对方文件放进本机剪贴板时不应显示进度条");
        r.PullRemoteClipboard();
        Ui.Pump(150);
        if (!ReferenceEquals(r.PasteData, placed)) Problems.Add("复制粘贴文件：同一批对方文件又放了一遍");

        // 3. 老被控端整批上传：进度条（其中一项名字带 hold，模拟传输停在 45%）、取消
        r.PushFiles(new[] { report, IOPath.Combine(home, "screenshot.png"), held });
        Wait("复制文件进度到 45%", () => r.FileClip?.Percent == 45);
        var heldTransfer = r.ClipTransfer as MockClipboardTransfer;
        if (r.FileClip is not { Direction: FileClipDirection.Push, Names.Length: 3 }) Problems.Add("复制粘贴文件：进度条内容不对");
        Capture($"{prefix}-44-remote-files-pushing", root);
        r.CancelFileClip();
        Ui.Pump(30);
        if (r.FileClip != null || heldTransfer?.Cancelled != true) Problems.Add("复制粘贴文件：取消后进度条没收起或传输没停");

        // 4. 完成提示：对方剪贴板变成暂存区里的这两项；推过去的暂存文件不再放回本机
        r.PushFiles(new[] { report, notes });
        Wait("复制文件到对方", () => r.FileClip?.State == FileClipState.Done || Failed());
        if (session.RemoteClipboard is ClipboardData.Files staged)
        {
            var names = staged.Paths.Select(ClipFiles.BaseName).ToArray();
            if (!names.SequenceEqual(new[] { "report-2026Q3.pdf", "deploy-notes.md" }) || !staged.Paths.All(x => x.StartsWith("/C:/Users/alice/.lemon-galaxy/clipboard/", StringComparison.Ordinal)))
                Problems.Add("复制粘贴文件：对方剪贴板里的暂存路径不对 " + string.Join(", ", staged.Paths));
        }
        else Problems.Add("复制粘贴文件：推送完成后对方剪贴板不是文件");
        Capture($"{prefix}-45-remote-files-pushed", root);
        var pushed = r.FileClip;
        r.PullRemoteClipboard();
        Ui.Pump(150);
        if (!ReferenceEquals(r.FileClip, pushed) || !ReferenceEquals(r.PasteData, placed)) Problems.Add("复制粘贴文件：推过去的文件又被放回本机");
        r.DismissFileClip();

        // 5. 失败：名字带 fail 的在 60% 处失败，显示原因
        r.PushFiles(new[] { failing });
        Wait("复制文件失败", Failed);
        Capture($"{prefix}-47-remote-files-failed", root);
        r.DismissFileClip();
        if (r.FileClip != null) Problems.Add("复制粘贴文件：失败提示收不起来");
    }

    /// <summary>虚拟文件数据对象：复制时没读内容；描述表（FILEGROUPDESCRIPTORW）路径、属性、大小、标志；按项读出的内容、Seek、Stat；其余格式</summary>
    private static void CheckPasteData(RemoteFileData d, string[] remote)
    {
        void Expect(bool ok, string what) { if (!ok) Problems.Add("复制粘贴文件（对方 → 本机）：" + what); }
        if (d.Offer is not MockRemoteClipOffer mock)
        {
            Problems.Add("复制粘贴文件（对方 → 本机）：没有模拟清单");
            return;
        }
        Expect(mock.Opened == 0, "复制时就读了文件内容");
        var info = mock.Info();
        var top = remote.Select(ClipFiles.BaseName).ToArray();
        var want = new[] { top[0], top[1], top[1] + "\\README.md", top[1] + "\\deploy.ps1" };
        Expect(d.Names.SequenceEqual(want), "描述表路径不对 " + string.Join(", ", d.Names));

        ComTypes.FORMATETC Etc(short cf, ComTypes.TYMED tymed, int lindex = -1) => new() { cfFormat = cf, dwAspect = ComTypes.DVASPECT.DVASPECT_CONTENT, lindex = lindex, tymed = tymed };
        // 描述表
        var f = Etc(RemoteFileData.CfDescriptor, ComTypes.TYMED.TYMED_HGLOBAL);
        if (d.GetData(ref f, out var m) == 0 && m.tymed == ComTypes.TYMED.TYMED_HGLOBAL)
        {
            var p = NativeClip.GlobalLock(m.unionmember);
            var n = Marshal.ReadInt32(p);
            Expect(n == want.Length && (ulong)NativeClip.GlobalSize(m.unionmember) >= (ulong)(4 + RemoteFileData.DescriptorSize * n), $"描述表项数或大小不对（{n}）");
            for (var i = 0; i < Math.Min(n, info.Entries.Length); i++)
            {
                var at = p + 4 + i * RemoteFileData.DescriptorSize;
                var flags = (uint)Marshal.ReadInt32(at);
                var attrs = (uint)Marshal.ReadInt32(at + 36);
                var write = Marshal.ReadInt64(at + 56);
                var sizeHigh = (uint)Marshal.ReadInt32(at + 64);
                var sizeLow = (uint)Marshal.ReadInt32(at + 68);
                var name = Marshal.PtrToStringUni(at + 72);
                var e = info.Entries[i];
                var okFlags = (flags & RemoteFileData.FdProgressUi) != 0 && (flags & RemoteFileData.FdAttributes) != 0 && (flags & RemoteFileData.FdWriteTime) != 0
                    && ((flags & RemoteFileData.FdFileSize) != 0) == !e.IsDir;
                Expect(okFlags, $"第 {i} 项标志不对 0x{flags:X}");
                Expect(attrs == (e.IsDir ? RemoteFileData.AttrDirectory : RemoteFileData.AttrNormal), $"第 {i} 项属性不对 0x{attrs:X}");
                Expect(e.IsDir || ((ulong)sizeHigh << 32 | sizeLow) == e.Size, $"第 {i} 项大小不对");
                Expect(write == RemoteFileData.FileTime(e.Mtime), $"第 {i} 项修改时间不对");
                Expect(name == want[i], $"第 {i} 项名字不对 {name}");
            }
            NativeClip.GlobalUnlock(m.unionmember);
            NativeClip.ReleaseStgMedium(ref m);
        }
        else Problems.Add("复制粘贴文件（对方 → 本机）：取不到描述表");

        // 其余格式：首选复制、我们的标记；没有 CF_HDROP、文件夹没有内容
        f = Etc(RemoteFileData.CfDropEffect, ComTypes.TYMED.TYMED_HGLOBAL);
        if (d.GetData(ref f, out m) == 0)
        {
            Expect(Marshal.ReadInt32(NativeClip.GlobalLock(m.unionmember)) == 1, "Preferred DropEffect 不是复制");
            NativeClip.GlobalUnlock(m.unionmember);
            NativeClip.ReleaseStgMedium(ref m);
        }
        else Problems.Add("复制粘贴文件（对方 → 本机）：取不到 Preferred DropEffect");
        f = Etc(RemoteFileData.CfMarker, ComTypes.TYMED.TYMED_HGLOBAL);
        Expect(d.QueryGetData(ref f) == 0, "没有私有标记格式");
        f = Etc(15 /* CF_HDROP */, ComTypes.TYMED.TYMED_HGLOBAL);
        Expect(d.QueryGetData(ref f) != 0, "不应提供 CF_HDROP");
        f = Etc(RemoteFileData.CfContents, ComTypes.TYMED.TYMED_ISTREAM, 1);
        Expect(d.GetData(ref f, out _) == Hr.Lindex, "文件夹不应给内容");
        f = Etc(RemoteFileData.CfContents, ComTypes.TYMED.TYMED_ISTREAM);
        Expect(d.QueryGetData(ref f) == 0, "没有 FileContents");
        Expect(d.EnumFormatEtc(ComTypes.DATADIR.DATADIR_GET, out var en) == 0 && en != null, "列不出格式");
        if (en != null)
        {
            var got = new ComTypes.FORMATETC[8];
            var fetched = new int[1];
            en.Next(8, got, fetched);
            Expect(fetched[0] == 4, $"格式个数不对（{fetched[0]}）");
        }

        // 按项读：第 0 项约 300 KB，读取器每次最多给 64 KB，流要读满再返回
        f = Etc(RemoteFileData.CfContents, ComTypes.TYMED.TYMED_ISTREAM, 0);
        if (d.GetData(ref f, out m) != 0 || m.tymed != ComTypes.TYMED.TYMED_ISTREAM)
        {
            Problems.Add("复制粘贴文件（对方 → 本机）：取不到第 0 项内容");
            return;
        }
        Expect(mock.Opened == 0, "取到内容流时就向对方打开了（应等第一次读）");
        var stream = (IOleStream)Marshal.GetObjectForIUnknown(m.unionmember);
        var expect = mock.Content(0);
        var buf = Marshal.AllocHGlobal(100_000);
        var cnt = Marshal.AllocHGlobal(8);
        try
        {
            var all = new List<byte>();
            var shortRead = false;
            while (true)
            {
                if (stream.Read(buf, 100_000, cnt) != 0) { Problems.Add("复制粘贴文件（对方 → 本机）：读内容出错"); break; }
                var n = Marshal.ReadInt32(cnt);
                if (n == 0) break;
                var chunk = new byte[n];
                Marshal.Copy(buf, chunk, 0, n);
                all.AddRange(chunk);
                if (n < 100_000 && all.Count < expect.Length) shortRead = true;
            }
            Expect(all.SequenceEqual(expect), $"读出来的内容不对（{all.Count} / {expect.Length} 字节）");
            Expect(!shortRead, "没到末尾就少给了数据");
            Expect(stream.Stat(out var st, 1) == 0 && st.cbSize == expect.Length, "Stat 大小不对");
            // Seek 回去从中间读
            Expect(stream.Seek(10, 0, IntPtr.Zero) == 0 && stream.Read(buf, 5, cnt) == 0 && Marshal.ReadInt32(cnt) == 5, "Seek 后读不对");
            var mid = new byte[5];
            Marshal.Copy(buf, mid, 0, 5);
            Expect(mid.SequenceEqual(expect.Skip(10).Take(5)), "Seek 后读出的内容不对");
            Expect(mock.Opened == 2, $"打开次数不对（{mock.Opened}，应为第一次读与 Seek 后各一次）");
        }
        finally
        {
            Marshal.FreeHGlobal(buf);
            Marshal.FreeHGlobal(cnt);
            NativeClip.ReleaseStgMedium(ref m);
        }
    }

    /// <summary>
    /// 虚拟屏（docs/protocol.md 8.6）：加一块 2560×1440 → 屏幕列表多一块我加的虚拟屏并切过去看；弹出层（我加的列表、自定义值不合法时的范围提示）；
    /// 删掉正在看的虚拟屏 → 切回主屏。离屏渲染不了 Popup，弹出层单独渲染。
    /// </summary>
    private static void CheckVirtualDisplay(AppModel app, RemoteTab r, FrameworkElement root, string prefix)
    {
        void Expect(bool ok, string what) { if (!ok) Problems.Add("虚拟屏：" + what); }
        Expect(r.VirtualSupported && r.VirtualMax == MockData.VirtualDisplayMax, $"模拟的 Windows 设备应能加 {MockData.VirtualDisplayMax} 块，实际 {r.VirtualMax}");
        if (!r.VirtualSupported) return;
        var physical = r.Displays.Length;
        ClearToast(app);

        // 1. 加一块：记进我加的、列表多一块、切过去看
        bool? added = null;
        Ui.Run(async () => added = await r.AddVirtualDisplay(new VirtualDisplayMode(2560, 1440, 60)));
        if (!Wait("加虚拟屏", () => added != null && !r.VirtualBusy)) return;
        var id = r.DisplayId;
        Expect(added == true, "加屏失败（" + app.CurrentToast?.Text + "）");
        Expect(r.Displays.Length == physical + 1 && r.Display is { IsVirtual: true, Width: 2560, Height: 1440 } && r.IsMyVirtual(id), "加好后没有切到新加的虚拟屏");
        Expect(app.CurrentToast?.Text == L.T("remote.vd.added", "2560×1440"), "加好后的提示不对（" + app.CurrentToast?.Text + "）");
        Wait("虚拟屏画面", () => r.Phase == RemotePhase.Streaming);
        Capture($"{prefix}-48-remote-virtual-display", root);
        ClearToast(app);

        // 2. 弹出层：表单 + 我加的虚拟屏；自定义宽 500 不合法时按钮置灰并提示范围
        Capture($"{prefix}-49-remote-virtual-display-panel", RemoteVirtualDisplayView.Preview(r), new Size(376, 500));
        var form = r.VirtualForm;
        form.PickSize(VdSizeKind.Custom);
        form.Width = "500";
        form.Notify();
        Expect(form.Mode == null, "宽 500 应不合法");
        Capture($"{prefix}-49b-remote-virtual-display-custom", RemoteVirtualDisplayView.Preview(r), new Size(376, 560));
        form.Reset();

        // 3. 删掉正在看的这块：先切回主屏，列表里没了
        var removed = false;
        Ui.Run(async () =>
        {
            await r.RemoveVirtualDisplay(id);
            removed = true;
        });
        if (!Wait("删虚拟屏", () => removed && !r.VirtualBusy)) return;
        Expect(!r.IsMyVirtual(id) && r.Displays.Length == physical && r.Displays.All(d => d.Id != id), "删掉后列表里还有这块屏");
        Expect(r.Display?.Primary == true, "删掉正在看的虚拟屏后没有切回主屏");
        Expect(app.CurrentToast?.Text == L.T("remote.vd.removed"), "删掉后的提示不对（" + app.CurrentToast?.Text + "）");
        Wait("切回主屏的画面", () => r.Phase == RemotePhase.Streaming);
        ClearToast(app);
    }

    /// <summary>
    /// 断线自动重连：画面被被控端结束 → 最后一帧压暗、显示「正在重新连接」，会话还在就在同一会话、同一块屏上重开画面；
    /// 会话断开 → 经连接池重新建会话、回到原来那块屏，声音照设置恢复；一直连不上 → 按间隔重试，次数用完显示失败与重试按钮，点重试从头再来。
    /// </summary>
    private static void CheckReconnect(AppModel app, RemoteTab r, FrameworkElement root, string prefix, bool zh)
    {
        void Expect(bool ok, string what) { if (!ok) Problems.Add("断线重连：" + what); }
        if (!Wait("重连前的画面", () => r.Phase == RemotePhase.Streaming)) return;
        if (app.Client is not MockClient mock || r.Channel is not MockDisplayChannel ch || r.Peer is not { Session: MockPeer peer } oldPeer)
        {
            Problems.Add("断线重连：拿不到模拟的画面通道或会话");
            return;
        }
        var display = r.DisplayId;

        // 1. 画面被被控端结束、会话还在：同一会话、同一块屏重开画面
        var frames = r.FramesDecoded;
        ch.SimulateEnded(zh ? "画面没有响应" : "The display stopped responding");
        if (!Wait("画面断了转入重连", () => r.Phase == RemotePhase.Reconnecting)) return;
        Expect(r.Channel == null && r.Live == false, "重连中还留着旧画面通道");
        Capture($"{prefix}-50-remote-reconnecting", root);
        if (!Wait("重开画面", () => r.Phase == RemotePhase.Streaming)) return;
        Expect(ReferenceEquals(r.Peer, oldPeer) && r.DisplayId == display && r.Channel != null && !ReferenceEquals(r.Channel, ch) && r.FramesDecoded > frames,
            "会话还在时应在同一会话、同一块屏上重开画面");

        // 2. 会话断开：重新建会话，回到原来那块屏，声音照设置恢复
        peer.SimulateClosed(zh ? "连接已断开" : "Connection closed");
        if (!Wait("会话断了转入重连", () => r.Phase == RemotePhase.Reconnecting)) return;
        Expect(!r.AudioOn, "会话断了声音没有停");
        if (!Wait("重新建会话", () => r.Phase == RemotePhase.Streaming)) return;
        Expect(r.Peer is { ClosedReason: null } p2 && !ReferenceEquals(p2, oldPeer) && r.DisplayId == display, "会话断开后没有重新建会话、回到原来那块屏");
        if (r.AudioSupported && app.Settings.RemoteAudio) Wait("重连后声音恢复", () => r.AudioOn, 5000);

        // 3. 一直连不上：间隔改短，次数用完显示最后一次的原因与重试按钮；点重试从头再来
        var delays = RemoteTab.ReconnectDelays;
        var offline = zh ? "设备离线" : "The device is offline";
        RemoteTab.ReconnectDelays = new[] { 10, 10, 10 };
        mock.ConnectFailure = offline;
        try
        {
            if (r.Peer?.Session is MockPeer peer2) peer2.SimulateClosed(zh ? "连接已断开" : "Connection closed");
            if (Wait("重连放弃", () => r.Phase == RemotePhase.Failed))
            {
                Expect(r.FailReason == offline, "放弃后显示的原因不对（" + r.FailReason + "）");
                Capture($"{prefix}-51-remote-reconnect-failed", root);
            }
        }
        finally
        {
            mock.ConnectFailure = null;
            RemoteTab.ReconnectDelays = delays;
        }
        r.Start();
        Wait("点重试后重新连上", () => r.Phase == RemotePhase.Streaming);
        ClearToast(app);
    }

    /// <summary>本机一侧用的假主目录（文件名、时间都固定）</summary>
    private static string MakeFakeHome()
    {
        var baseDir = IOPath.Combine(IOPath.GetTempPath(), "lemongalaxy-selftest-" + Environment.TickCount);
        var home = IOPath.Combine(baseDir, "alice");
        Directory.CreateDirectory(home);
        var t = DateTimeOffset.FromUnixTimeMilliseconds(1_795_225_692_000).LocalDateTime;
        var items = new (string name, bool dir, int size, double daysAgo)[]
        {
            ("Desktop", true, 0, 2), ("Documents", true, 0, 5), ("Downloads", true, 0, 0.2), ("Projects", true, 0, 9), (".ssh", true, 0, 40), ("Pictures", true, 0, 60),
            ("deploy-notes.md", false, 2430, 1), ("galaxy-server.toml", false, 812, 3), ("report-2026Q3.pdf", false, 1_284_112, 12),
            ("screenshot.png", false, 482_330, 0.5), ("backup-2026-11-01.tar.gz", false, 3_412_000, 20),
        };
        foreach (var (name, isDir, size, days) in items)
        {
            var p = IOPath.Combine(home, name);
            if (isDir)
            {
                Directory.CreateDirectory(p);
                Directory.SetLastWriteTime(p, t.AddDays(-days));
            }
            else
            {
                File.WriteAllBytes(p, new byte[size]);
                File.SetLastWriteTime(p, t.AddDays(-days));
            }
        }
        return home;
    }

    // ---------- 渲染 ----------

    private static void Settle(FrameworkElement root, Size size)
    {
        for (var i = 0; i < 4; i++)
        {
            Ui.DoEvents();
            root.Measure(size);
            root.Arrange(new Rect(size));
            root.UpdateLayout();
        }
        Ui.Pump(40);
        root.Measure(size);
        root.Arrange(new Rect(size));
        root.UpdateLayout();
    }

    private static void Capture(string name, FrameworkElement root, Size? sizeOpt = null)
    {
        if (only != null && !name.Contains(only)) return;
        var size = sizeOpt ?? WindowSize;
        try
        {
            Settle(root, size);
            var rtb = new RenderTargetBitmap((int)Math.Round(size.Width * scale), (int)Math.Round(size.Height * scale), 96 * scale, 96 * scale, PixelFormats.Pbgra32);
            var bg = new DrawingVisual();
            using (var dc = bg.RenderOpen()) dc.DrawRectangle(Th.Content, null, new Rect(size));
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

    private static bool Wait(string what, Func<bool> cond, int timeoutMs = 8000)
    {
        if (Ui.WaitFor(cond, timeoutMs)) return true;
        Problems.Add("等待超时：" + what);
        return false;
    }

    /// <summary>
    /// AI 页面：不可用的四种提示（加载中、失败、没开通、没模型）、新对话、Markdown 回复、连接设备确认、危险命令确认、提问与密码、
    /// 图形界面目标、引用标签、文件选择器、对话列表分组。
    /// </summary>
    private static void RenderAi(AppModel app, FrameworkElement root, string prefix, bool zh)
    {
        var ai = app.Ai;
        var mock = app.Client as MockClient;
        app.SelectHome(NavSection.Ai);

        // 不可用时的提示
        if (mock != null)
        {
            ai.Models = null;
            ai.ModelsError = null;
            ai.LoadingModels = true;
            ai.Changed.Notify();
            Capture($"{prefix}-50-ai-loading", root);
            ai.LoadingModels = false;
            foreach (var (state, name) in new[] { (MockAiState.Error, "51-ai-failed"), (MockAiState.Disabled, "52-ai-disabled"), (MockAiState.NoModels, "53-ai-no-models") })
            {
                mock.AiState = state;
                ai.Retry();
                Wait("AI 模型列表 " + state, () => !ai.LoadingModels);
                if (ai.Availability.ToString() != state switch { MockAiState.Error => "Failed", MockAiState.Disabled => "Disabled", _ => "NoModels" })
                    Problems.Add($"AI 不可用提示：模拟 {state} 时状态是 {ai.Availability}");
                Capture($"{prefix}-{name}", root);
            }
            mock.AiState = MockAiState.Ready;
        }
        ai.Retry();
        Wait("AI 模型列表", () => ai.Availability == AiAvailability.Ready);
        if (ai.DraftMode != AiMode.ConfirmDangerous) Problems.Add("AI：新对话默认档位应为危险操作确认，实际 " + ai.DraftMode);
        if (ai.DraftModel != "1:kimi-k2") Problems.Add("AI：没有上次的选择时应取列表第一个模型，实际 " + ai.DraftModel);
        CheckMention(ai);

        // 新对话：问候 + 输入框居中，输入框里有一个设备引用
        ai.NewChat();
        ai.NewDraft.Refs.Clear();
        ai.NewDraft.AddRef(AiHub.DeviceRef(app, "dev-build"));
        ai.NewDraft.Text = zh ? "/data 快满了，帮我看看是什么占的空间" : "/data is almost full, find out what uses the space";
        ai.NewDraft.Notify();
        Capture($"{prefix}-54-ai-new", root);

        // 没引用设备的问答：账号查询（只读工具行）+ Markdown 回复
        ai.NewDraft.Refs.Clear();
        ai.NewDraft.Text = zh ? "我的账号里有哪些设备？" : "Which devices are in my account?";
        ai.SendNew();
        Wait("AI 回答", () => ai.Current is { Running: false, Items.Count: > 2 });
        if (ai.Current?.Items.OfType<ToolItem>().FirstOrDefault() is not { Call.Tool: "list_devices", Phase: ToolPhase.Finished })
            Problems.Add("AI：没引用设备时应先用 list_devices 查账号");
        if (ai.NewDraft.Text.Length != 0) Problems.Add("AI：发出后输入框应清空");
        if (ai.Current?.Items.OfType<ToolItem>().FirstOrDefault() is { } listTool) { listTool.Expanded = true; listTool.Notify(); }
        Capture($"{prefix}-55-ai-markdown", root);

        // 消息里点名了没引用的设备：先弹「AI 想连接」卡片（只有允许与拒绝）
        var chat = ai.Current;
        if (chat != null)
        {
            chat.Draft.Text = zh ? "看看 build-server-01 的 /data 为什么满了，清理一下旧备份" : "Find out why /data on build-server-01 is full and clean up old backups";
            ai.Send(chat);
            if (Wait("AI 想连接设备", () => chat.HasPendingApproval))
            {
                var use = chat.Items.OfType<ToolItem>().Last();
                if (!use.IsUseTarget) Problems.Add("AI：点名没引用的设备时第一张卡片应是 use_target");
                Capture($"{prefix}-56-ai-use-target", root);
                chat.Decide(use, new ApprovalDecision.AllowOnce());
                // 危险命令确认
                if (Wait("AI 危险命令确认", () => chat.Items.OfType<ToolItem>().Any(c => c.Decision == ToolDecision.Pending && c.Call.Risk == RiskLevel.Dangerous)))
                    Capture($"{prefix}-57-ai-approval", root);
                var card = chat.Items.OfType<ToolItem>().FirstOrDefault(c => c.Decision == ToolDecision.Pending);
                if (card != null)
                {
                    chat.Decide(card, new ApprovalDecision.AllowOnce());
                    if (Wait("AI 提问", () => chat.Items.OfType<AskItem>().Any()))
                    {
                        Capture($"{prefix}-58-ai-ask", root);
                        var ask = chat.Items.OfType<AskItem>().First();
                        chat.Answer(ask, ask.Options[0]);
                        Wait("AI 请求密码", () => chat.Items.OfType<SecretItem>().Any());
                    }
                }
                Capture($"{prefix}-59-ai-secret", root);
                if (chat.Items.OfType<SecretItem>().FirstOrDefault() is { } sec) chat.CancelSecret(sec);
                Wait("AI 任务结束", () => !chat.Running);
            }
        }

        // 图形界面目标（引用了 Windows 设备）：截图 + 点击标注图，每步确认档，模型换成能看图的
        ai.NewChat();
        ai.SetModel(null, ai.Models!.Items.First(m => m.Vision));
        ai.SetMode(null, AiMode.ConfirmAll, 0);
        ai.NewDraft.AddRef(AiHub.DeviceRef(app, "dev-office"));
        ai.NewDraft.Text = zh ? "把记事本里的内容保存一下" : "Save the Notepad content";
        ai.SendNew();
        if (Wait("AI 图形界面确认", () => ai.Current is { HasPendingApproval: true }))
        {
            var shot = ai.Current!.Items.OfType<ToolItem>().FirstOrDefault(c => c.Call.Tool == "screenshot");
            if (shot != null) { shot.Expanded = true; shot.Notify(); }
        }
        Capture($"{prefix}-60-ai-gui", root);
        if (app.Settings.AiLastModel != "1:qwen3-vl-plus" || app.Settings.AiMode != AiMode.ConfirmAll)
            Problems.Add("AI：选过的模型与档位应记到本机设置里");
        ai.Current?.Stop();
        if (ai.Current is { } gui) RenderAiLocalPlan(ai, gui, root, prefix, zh);

        // 引用了文件、片段、端口映射、连接记录的消息 + 历史列表分组
        ai.NewChat();
        foreach (var r in new[]
        {
            new ContextRef(ContextRefKind.File, "dev-build", "/var/log/nginx/error.log", "error.log"),
            new ContextRef(ContextRefKind.Snippet, "", "n-df", zh ? "磁盘占用" : "Disk usage"),
            new ContextRef(ContextRefKind.PortForward, "", "p-db", zh ? "生产数据库" : "Production database"),
            AiPickers.ActivityRef(),
        }) ai.NewDraft.AddRef(r);
        ai.NewDraft.Text = zh ? "最近有没有异常登录？" : "Any unusual sign-ins lately?";
        ai.SendNew();
        Wait("AI 连接记录", () => ai.Current is { Running: false, Items.Count: > 2 });
        Capture($"{prefix}-61-ai-refs", root);

        // 文件选择器：选设备后浏览主目录
        var listed = AiPickers.PickFileOn(app, ai.NewDraft, "dev-build");
        Wait("文件选择器列出主目录", listed);
        Capture($"{prefix}-62-ai-file-picker", root);
        app.CloseModal(true);

        // 历史对话：重新打开时用 messages() 恢复
        var first = ai.Rows().LastOrDefault();
        if (first != null)
        {
            ai.Live.Clear();
            ai.Select(first.Id);
            Wait("AI 打开历史对话", () => ai.Current is { Opening: false });
            if (ai.Current is not { Items.Count: > 0 }) Problems.Add("AI：打开历史对话后消息流是空的");
            Capture($"{prefix}-63-ai-history", root);
        }
        ai.NewChat();
    }

    /// <summary>本机目标、任务清单（update_plan，同一轮只展开最新一份）、应用操作确认（use_app）、应用工具行、连续执行暂停后的「继续」</summary>
    private static void RenderAiLocalPlan(AiHub ai, AiChat chat, FrameworkElement root, string prefix, bool zh)
    {
        Wait("AI 停止", () => !chat.Running);
        var local = ai.LocalRef;
        var img = chat.Items.OfType<ToolItem>().Select(c => c.Result?.Image ?? c.Call.Image).FirstOrDefault(i => i != null);
        ToolCallView Call(string id, string tool, string summary, string detail, RiskLevel risk = RiskLevel.Read, string[]? reasons = null, string reason = "")
            => new(id, tool, AiHub.LocalId, local.Label, summary, detail, risk, reasons ?? Array.Empty<string>(), reason, null);
        void Done(ToolCallView c, string summary, string output = "", ImageData? image = null)
        {
            chat.Handle(new AgentEvent.ToolProposed(c, false));
            chat.Handle(new AgentEvent.ToolFinished(c.CallId, new ToolResultView(true, summary, output, image, false)));
        }
        Done(Call("st-p1", "update_plan", zh ? "任务清单 0/3：打开 Safari" : "Task list 0/3: Open Safari",
            zh ? "[>] 打开 Safari\n[ ] 进入内网首页\n[ ] 找到值班表\n" : "[>] Open Safari\n[ ] Go to the intranet home page\n[ ] Find the on-call rota\n"), "0/3");
        Done(Call("st-a1", "app_open", zh ? "打开 Safari" : "Open Safari", "Safari"), zh ? "已打开" : "Opened");
        Done(Call("st-a2", "app_state", zh ? "读取 Safari 界面" : "Read the Safari window", "Safari"), zh ? "142 个控件" : "142 elements", "[1] AXWindow \"Safari\"\n  [2] AXTextField", img);
        if (chat.Items.OfType<ToolItem>().LastOrDefault(c => c.Call.Tool == "app_state") is { } st) { st.Expanded = true; st.Notify(); }
        Done(Call("st-p2", "update_plan", zh ? "任务清单 1/3：进入内网首页" : "Task list 1/3: Go to the intranet home page",
            zh ? "[x] 打开 Safari\n[>] 进入内网首页\n[ ] 找到值班表\n" : "[x] Open Safari\n[>] Go to the intranet home page\n[ ] Find the on-call rota\n"), "1/3");
        Done(Call("st-w", "web_fetch", "https://intranet.example.com/oncall", ""), "200");
        Done(Call("st-s", "system_info", zh ? "查看系统信息" : "System info", ""), "Windows 11");
        var plans = chat.Items.OfType<ToolItem>().Where(c => c.IsPlan).ToList();
        if (plans.Count != 2 || !plans[0].PlanSuperseded || plans[1].PlanSuperseded) Problems.Add("AI：同一轮里只有最新的任务清单展开");
        var parsed = AiViews.ParsePlan("[x] a\n[>] b\n[ ] c\n");
        if (parsed.Count != 3 || parsed[0].Status != AiViews.PlanStatus.Done || parsed[1].Status != AiViews.PlanStatus.Active || parsed[2].Status != AiViews.PlanStatus.Pending)
            Problems.Add("AI：任务清单解析不对");
        chat.Handle(new AgentEvent.ToolProposed(Call("st-ua-app", "use_app", zh ? "AI 想操作「Safari」" : "AI wants to use “Safari”",
            zh ? "允许后，本对话里 AI 可以读取「Safari」的界面并在里面点击、输入（不会占用你的鼠标键盘）" : "Once allowed, the agent can read Safari's window and click and type in it during this conversation (without taking over your mouse and keyboard)",
            RiskLevel.Normal, new[] { zh ? "本对话第一次操作这个应用" : "First time this conversation uses this app" }, zh ? "要在地址栏输入内网地址" : "Needs to type the intranet address"), true));
        Capture($"{prefix}-60a-ai-local-plan-app", root);
        if (chat.Items.OfType<ToolItem>().LastOrDefault() is { IsUseApp: true } ua)
        {
            ua.Decision = ToolDecision.Approved;
            chat.Handle(new AgentEvent.ToolFinished(ua.Call.CallId, new ToolResultView(true, zh ? "已允许" : "Allowed", "", null, false)));
        }
        else Problems.Add("AI：use_app 没有成为确认卡片");
        chat.Handle(new AgentEvent.Error("已连续执行 40 步，先暂停一下。回复「继续」可以接着做"));
        Capture($"{prefix}-60b-ai-continue", root);
        if (WirePathOk() is { } bad) Problems.Add("AI：本机路径换算不对：" + bad);
    }

    private static string? WirePathOk()
    {
        foreach (var (a, b) in new[] { (@"C:\Users\alice\a.txt", "/C:/Users/alice/a.txt"), (@"d:\", "/D:/"), (@"\\nas\share\x", "//nas/share/x"), (@"\\?\C:\x", "/C:/x") })
            if (AiPickers.WirePath(a) != b) return a + " → " + AiPickers.WirePath(a);
        return null;
    }

    /// <summary>@ 引用：光标前的 @ 片段识别（开头或空白后才算，邮件地址不算）与候选过滤</summary>
    private static void CheckMention(AiHub ai)
    {
        void Expect(bool ok, string what) { if (!ok) Problems.Add("@ 引用：" + what); }
        Expect(Mention.Find("@bu", 3) is { at: 0, query: "bu" }, "开头的 @bu 没识别");
        Expect(Mention.Find("看看 @gpu", 7) is { at: 3, query: "gpu" }, "空格后的 @gpu 没识别");
        Expect(Mention.Find("a@b.com", 7) == null, "邮件地址被当成了引用");
        Expect(Mention.Find("@bu ild", 7) == null, "@ 片段里有空格还被识别");
        var c = ai.TargetCandidates("build");
        Expect(c.Count == 1 && c[0].Kind == ContextRefKind.Device && c[0].Id == "dev-build", "按 build 过滤的候选不对");
        Expect(!ai.TargetCandidates("").Any(r => r.Id is "dev-self" or "dev-phone"), "候选里不应有本机设备记录与手机");
        Expect(ai.TargetCandidates("").FirstOrDefault() is { Kind: ContextRefKind.Device, TargetId: "", Id: AiHub.LocalId }, "第一项应是本机");
        Expect(ai.WhereName(AiHub.LocalId) == ai.LocalRef.Label, "本机的显示名不对");
        Expect(ai.TargetCandidates("db-").All(r => r.Kind == ContextRefKind.SshHost) && ai.TargetCandidates("db-").Count == 2, "按 db- 过滤应只有两台 SSH 主机");
    }

    /// <summary>Markdown 解析：标题、列表嵌套、代码块（含未闭合）、表格</summary>
    private static void CheckMarkdown()
    {
        void Expect(bool ok, string what) { if (!ok) Problems.Add("Markdown：" + what); }
        var b = Md.Parse("# 标题\n\n段落 **粗体** `code`\n\n- a\n  - b\n- c\n\n1. x\n2. y\n\n| A | B |\n|---|:-:|\n| 1 | 2 |\n\n```bash\nls -la\n```\n\n> 引用\n\n---\n\n```py\nprint(1)");
        Expect(b.Count == 9, "块数应为 9，实际 " + b.Count);
        Expect(b.ElementAtOrDefault(0) is Md.Heading { Level: 1, Text: "标题" }, "标题");
        Expect(b.ElementAtOrDefault(2) is Md.ListBlock { Ordered: false, Items.Count: 2 } l && l.Items[0].Child is { Items.Count: 1 }, "无序列表与嵌套");
        Expect(b.ElementAtOrDefault(3) is Md.ListBlock { Ordered: true, Items.Count: 2 }, "有序列表");
        Expect(b.ElementAtOrDefault(4) is Md.Table { Header.Length: 2, Rows.Count: 1 } t && t.Align[1] == TextAlignment.Center, "表格");
        Expect(b.ElementAtOrDefault(5) is Md.Code { Lang: "bash", Text: "ls -la" }, "代码块");
        Expect(b.ElementAtOrDefault(8) is Md.Code { Lang: "py", Text: "print(1)" }, "流式输出中未闭合的代码块");
        var inl = Md.Inlines("a **b** `c` [d](https://e.example) *f*");
        Expect(inl.OfType<System.Windows.Documents.Bold>().Count() == 1 && inl.OfType<System.Windows.Documents.Hyperlink>().Count() == 1 && inl.OfType<System.Windows.Documents.Italic>().Count() == 1, "行内粗体 / 链接 / 斜体");
        Expect(Md.Inlines("snake_case_name").Count == 1, "下划线不应当成斜体");
    }

    private static void RenderAll(AppSettings settings, string lang, string theme)
    {
        settings.ResetForSelfTest();
        settings.Language = lang;
        settings.Appearance = theme;
        settings.Palette = "galaxy";
        settings.TerminalTheme = theme == "dark" ? "galaxy-dark" : "galaxy-light";
        L.Apply(lang);
        Th.Apply(settings);
        var prefix = (lang == "en" ? "en" : "zh") + "-" + theme;
        var zh = lang != "en";
        MockTerminalRegistry.Shared.Reset();
        var app = new AppModel(settings, selfTest: true, forceMock: true, new MockOptions { LatencyScale = 0, LiveSimulation = false });
        app.Worker.Fake(WorkerState.Running);
        const string server = "galaxy.lemonit.cn:7443#sha256-Q2F0Y2hNZUlmWW91Q2FuX0xlbW9uR2FsYXh5X1Rlc3Q";

        // 1. 登录三步
        Capture($"{prefix}-01-login", LoginView.Build(app, m => { m.Server = server; m.Username = "alice"; m.Password = "password"; }));
        Capture($"{prefix}-02-login-totp", LoginView.Build(app, m => { m.Server = server; m.Username = "alice-totp"; m.Password = "x"; m.Step = LoginStep.Totp; m.Totp = "48213"; }));
        Capture($"{prefix}-03-login-change-password", LoginView.Build(app, m => { m.Server = server; m.Username = "new-user"; m.Step = LoginStep.ChangePassword; m.NewPassword = "Galaxy-2026"; m.ConfirmPassword = "Galaxy-202"; }));
        Capture($"{prefix}-04-login-invalid-server", LoginView.Build(app, m => { m.Server = "galaxy.lemonit.cn:7443"; m.Username = "alice"; m.Error = zh ? "用户名或密码错误" : "Incorrect username or password"; }));

        // 登录（模拟内核立即返回）
        var loggedIn = false;
        Ui.Run(async () =>
        {
            await app.Client.Login(server, "alice", "pw", null);
            app.DidLogin();
            loggedIn = true;
        });
        Wait("登录", () => loggedIn);
        var root = Shell.Root(app);

        // 2. 设备与主机
        app.SelectHome(NavSection.Hosts);
        Capture($"{prefix}-10-hosts", root);
        app.Selection = "dev:dev-office";
        app.Nav.Notify();
        Capture($"{prefix}-11-hosts-device", root);
        app.Selection = "s-dbp";
        app.Editing = new EditTarget.Vault("s-dbp");
        app.Nav.Notify();
        Capture($"{prefix}-12-hosts-ssh-editor", root);
        app.Editing = null;
        app.Selection = "dev:dev-office";
        app.Nav.Notify();
        app.ActivateDevice("dev-office");
        Capture($"{prefix}-13-hosts-connect-chooser", root);
        app.CloseModal();
        app.HostsTab = HostsListTab.Credentials;
        app.Selection = "c-deploy";
        app.Editing = new EditTarget.Vault("c-deploy");
        app.Nav.Notify();
        Capture($"{prefix}-14-hosts-credential", root);
        app.Editing = null;
        app.HostsTab = HostsListTab.Hosts;
        app.Selection = null;
        app.SearchText = "web";
        app.Nav.Notify();
        Capture($"{prefix}-15-hosts-search", root);
        app.SearchText = "";
        app.Nav.Notify();
        app.ShowTerminalSessions("dev-build", MockTerminalRegistry.Shared.Sessions("dev-build").Where(s => !s.Attached).ToArray());
        Capture($"{prefix}-16-terminal-reattach", root);
        app.ShowModal(new ModalSpec { Width = 460, Build = () => Modals.HostKeyCard(app, "10.0.2.10", 22, "ssh-ed25519", "SHA256:Lm9uR2FsYXh5TW9ja0hvc3RLZXlGaW5nZXJwcmludA", true, _ => { }) });
        Capture($"{prefix}-17-ssh-hostkey-changed", root);
        app.ShowModal(new ModalSpec { Width = 420, Build = () => Modals.AssistConnectCard(app) });
        Capture($"{prefix}-18-assist-connect", root);
        app.CloseModal();
        // 离线设备远程开机：详情里的远程开机、右键菜单、发出后的提示
        if (!app.CanWake("dev-reception")) Problems.Add("远程开机：离线且上报过网卡地址的前台电脑应能远程开机");
        if (app.CanWake("dev-nas")) Problems.Add("远程开机：没有上报网卡地址的 NAS 不应显示远程开机");
        if (app.CanWake("dev-office")) Problems.Add("远程开机：在线设备不应显示远程开机");
        if (app.Nodes().FirstOrDefault(n => n.Id == "dev:dev-reception") is not { } wakeNode || !app.ContextMenu(wakeNode).Any(m => m.Title == L.T("wake.action")))
            Problems.Add("远程开机：离线设备的右键菜单里没有远程开机");
        app.Selection = "dev:dev-reception";
        app.Nav.Notify();
        app.WakeDevice("dev-reception");
        Wait("远程开机", () => !app.IsWaking("dev-reception") && app.CurrentToast != null);
        if (app.CurrentToast is not { IsError: false }) Problems.Add("远程开机：没有成功提示（" + app.CurrentToast?.Text + "）");
        Capture($"{prefix}-19-hosts-device-wake", root);
        ClearToast(app);
        app.Selection = null;
        app.Nav.Notify();

        // 3. 终端
        app.OpenTerminal("dev-build");
        if (app.ActiveTab is TerminalTab term)
        {
            term.Term.ForceFocusedLook = true;
            Wait("终端打开", () => term.Phase == TerminalPhase.Running);
            Ui.Pump(80);
            foreach (var cmd in new[] { "ls -la\r", "df -h\r", "colors\r" })
            {
                term.SendText(cmd);
                Ui.Pump(80);
            }
        }
        Capture($"{prefix}-20-terminal", root);
        app.OpenSshTerminal("s-web1");
        if (app.ActiveTab is TerminalTab ssh)
        {
            ssh.Term.ForceFocusedLook = true;
            Wait("SSH 终端打开", () => ssh.Phase == TerminalPhase.Running);
            ssh.SendText("top\r");
            Ui.Pump(120);
        }
        Capture($"{prefix}-21-terminal-ssh", root);

        // 4. 文件
        app.OpenSftp("dev-build");
        if (app.ActiveTab is SftpTab sftp)
        {
            Wait("文件远端列表", () => sftp.RightPane.Items.Count > 0 && !sftp.RightPane.Loading);
            Wait("文件本机列表", () => !sftp.LeftPane.Loading);
            // 演示传输队列：一个完成、一个失败
            if (sftp.RightPane.Source is RemoteFileSource r)
            {
                var tmp = IOPath.Combine(IOPath.GetTempPath(), "lemongalaxy-selftest-dl-" + Environment.TickCount);
                Directory.CreateDirectory(tmp);
                app.Transfers.Start("config.yaml", false, r.Client, IOPath.Combine(tmp, "config.yaml"), r.Join(sftp.RightPane.Path, "app/config.yaml"), sftp.RightPane.EndpointName);
                app.Transfers.Start("backup-fail.tar.gz", false, r.Client, IOPath.Combine(tmp, "b.tgz"), r.Join(sftp.RightPane.Path, "backup-fail.tar.gz"), sftp.RightPane.EndpointName);
                Wait("传输完成", () => app.Transfers.Items.All(i => i.State is not (TransferState.Running or TransferState.Queued)));
                try { Directory.Delete(tmp, true); } catch { }
            }
            var deploy = sftp.RightPane.Items.FirstOrDefault(i => i.Name == "deploy.sh");
            if (deploy != null) { sftp.RightPane.Selection = new HashSet<string> { deploy.Path }; sftp.RightPane.Notify(); }
        }
        Capture($"{prefix}-30-sftp", root);
        if (app.ActiveTab is SftpTab sftp2 && sftp2.RightPane.Items.FirstOrDefault(i => i.Name == "deploy.sh") is { } item)
        {
            SftpViews.ShowChmod(app, sftp2.RightPane, item);
            Capture($"{prefix}-31-sftp-chmod", root);
            app.CloseModal();
        }

        // 5. 远程桌面
        app.OpenRemoteDesktop("dev-office");
        if (app.ActiveTab is RemoteTab remote)
        {
            Wait("远程画面解码", () => remote.FramesDecoded > 0, 10000);
            // 声音默认开：画面打开后自动开始
            if (!remote.AudioSupported) Problems.Add("远程桌面：对端能力里没有 audio，工具栏不显示声音开关");
            else Wait("远端声音开始播放", () => remote.AudioOn, 5000);
            remote.MeasuredFps = 30;
            remote.MeasuredKbps = 8400;
            remote.Notify();
        }
        Capture($"{prefix}-40-remote-desktop", root);
        if (app.ActiveTab is RemoteTab remote2 && remote2.AudioOn)
        {
            // 关掉声音：记进设置、停掉播放（模拟声音少一路），再打开恢复
            var playing = MockAudio.Playing;
            remote2.ToggleAudio();
            if (remote2.AudioOn || app.Settings.RemoteAudio || MockAudio.Playing != playing - 1)
                Problems.Add($"远程桌面：关掉声音后状态不对（AudioOn={remote2.AudioOn}，设置={app.Settings.RemoteAudio}，播放路数 {playing} → {MockAudio.Playing}）");
            Capture($"{prefix}-41-remote-sound-off", root);
            remote2.ToggleAudio();
            Wait("远端声音重新开始", () => remote2.AudioOn, 5000);
        }
        // 录像：工具栏红点与计时；停止后提示保存位置并可在资源管理器中显示
        if (app.ActiveTab is RemoteTab remote3)
        {
            remote3.ToggleRecording();
            if (Wait("开始录像", () => remote3.Recording))
            {
                if (!remote3.ChannelRecording) Problems.Add("录像：界面显示在录，画面通道没有在录");
                remote3.RecordingSinceMs = Clock.NowMs - 157_000;
                remote3.Notify();
                Capture($"{prefix}-42-remote-recording", root);
                remote3.ToggleRecording();
                if (remote3.Recording || remote3.ChannelRecording) Problems.Add("录像：停止后状态不对");
                if (app.CurrentToast is not { IsError: false, ActionTitle: not null })
                    Problems.Add("录像：停止后没有带「在资源管理器中显示」的提示（" + app.CurrentToast?.Text + "）");
                Capture($"{prefix}-43-remote-recording-saved", root);
                ClearToast(app);
            }
        }

        // 复制粘贴文件
        if (app.ActiveTab is RemoteTab remote4) CheckFileClip(remote4, root, prefix);

        // 虚拟屏
        if (app.ActiveTab is RemoteTab remote5) CheckVirtualDisplay(app, remote5, root, prefix);

        // 断线自动重连
        if (app.ActiveTab is RemoteTab remote6) CheckReconnect(app, remote6, root, prefix, zh);

        // 6. AI 聊天（docs/ai-chat-ui.md）
        RenderAi(app, root, prefix, zh);

        // 7. 片段
        app.SelectedSnippet = "n-docker";
        app.SelectHome(NavSection.Snippets);
        Capture($"{prefix}-60-snippets", root);

        // 7b. 端口映射：登录后自动开启（经由离线设备的会失败）、手动开启（自动选端口）、详情、编辑、经由 SSH 主机时问主机密钥
        app.SelectedForward = null;
        app.Editing = null;
        app.SelectHome(NavSection.Forwards);
        Wait("端口映射自动开启", () => app.Forwards.StateOf("p-db") == ForwardState.Running && app.Forwards.StateOf("p-nas") == ForwardState.Failed);
        if (app.Forwards.StateOf("p-grafana") != ForwardState.Stopped || app.Forwards.StateOf("p-redis") != ForwardState.Stopped)
            Problems.Add("端口映射：没勾「登录后自动开启」的规则被自动开启了");
        app.Forwards.Start("p-grafana");
        Wait("端口映射手动开启", () => app.Forwards.StateOf("p-grafana") == ForwardState.Running);
        if (app.Forwards.Run("p-grafana") is not { Port: > 0 }) Problems.Add("端口映射：自动选的本机端口没有显示");
        Capture($"{prefix}-65-forwards", root);
        app.SelectedForward = "p-db";
        app.Nav.Notify();
        Capture($"{prefix}-66-forwards-detail", root);
        app.SelectedForward = "p-redis";
        app.Editing = new EditTarget.Forward("p-redis");
        app.Nav.Notify();
        Capture($"{prefix}-67-forwards-editor", root);
        app.Editing = null;
        app.Nav.Notify();
        if (app.Modal != null) app.CloseModal();
        app.Forwards.Start("p-redis");
        if (Wait("端口映射经 SSH 主机确认主机密钥", () => app.Modal != null && app.Forwards.StateOf("p-redis") == ForwardState.Starting))
        {
            Capture($"{prefix}-68-forwards-ssh-prompt", root);
            app.CloseModal(true);
            Wait("取消后端口映射回到已停止", () => app.Forwards.StateOf("p-redis") == ForwardState.Stopped);
        }
        ClearToast(app);
        app.SelectedForward = null;
        app.Nav.Notify();

        // 8. 本机被控（Windows 被控端系统服务）
        app.SelectHome(NavSection.Local);
        app.Worker.Fake(WorkerState.Running);
        Capture($"{prefix}-70-local-running", root);
        app.Worker.Fake(WorkerState.NotInstalled);
        Capture($"{prefix}-71-local-not-installed", root);

        // 9. 设置
        var pages = new[] { SettingsPage.Account, SettingsPage.Appearance, SettingsPage.Terminal, SettingsPage.Connection, SettingsPage.Ai, SettingsPage.About };
        for (var i = 0; i < pages.Length; i++)
        {
            app.SettingsPage = pages[i];
            app.SelectHome(NavSection.Settings);
            Capture($"{prefix}-8{i}-settings-{pages[i].ToString().ToLowerInvariant()}", root);
        }
        var qr = new Border
        {
            Background = Th.Content, Padding = new Thickness(24),
            Child = K.Row(18, new Border { Background = Brushes.White, Padding = new Thickness(4), CornerRadius = new CornerRadius(8), Child = QrImage.Image("otpauth://totp/LemonGalaxy:alice?secret=JBSWY3DPEHPK3PXPLGALAXY&issuer=LemonGalaxy", 180) },
                K.Col(8, K.Semi(L.T("account.totp"), 15), K.Text(L.T("account.totp.scan"), 13, Th.Secondary, wrap: true).Also(t => t.Width = 360), K.SelectableText("JBSWY3DPEHPK3PXPLGALAXY", 14, Th.Text, mono: true))),
        };
        Capture($"{prefix}-86-settings-totp-qr", qr, new Size(680, 260));

        // 10. 文本编辑器、紧凑侧栏
        if (app.Tabs.OfType<SftpTab>().FirstOrDefault()?.RightPane.Source is { } src)
        {
            var model = new TextEditorModel(src, "/home/deploy/app/config.yaml");
            Ui.Run(model.Load);
            Wait("编辑器读入", () => !model.Loading);
            model.Text += "  timeout: 30s\n";
            model.Notify();
            Capture($"{prefix}-90-text-editor", TextEditorWindow.Build(app, model), new Size(760, 480));
        }
        settings.SidebarCompact = true;
        settings.AutoHideList = false;
        app.Section = NavSection.Hosts;
        if (app.Tabs.OfType<TerminalTab>().FirstOrDefault() is { } t2) app.ActivateTab(t2.Id);
        Capture($"{prefix}-91-compact-terminal", root);
        settings.SidebarCompact = false;
        settings.AutoHideList = true;

        app.CloseAllTabs();
        app.Forwards.StopAll();
        Ui.Pump(50);
    }

    /// <summary>全部矢量图标一览（核对图标画得对不对）</summary>
    private static void IconSheet()
    {
        var names = Icons.AllNames.Distinct().OrderBy(n => n, StringComparer.Ordinal).ToList();
        var wrap = new WrapPanel { Width = 1000, Margin = new Thickness(16) };
        foreach (var n in names)
            wrap.Children.Add(new Border
            {
                Width = 118, Height = 72, Margin = new Thickness(4), Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8),
                Child = K.Center(K.Col(6, K.Center(K.Icon(n, 22, Th.Text)), K.Center(K.Text(n, 11, Th.Secondary)))),
            });
        var root = new Border { Background = Th.Content, Child = wrap };
        var rows = (names.Count + 7) / 8;
        Capture("99-icons", root, new Size(1032, rows * 80 + 40));
    }
}
