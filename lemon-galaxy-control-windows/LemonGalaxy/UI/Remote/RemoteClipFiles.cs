using System.Collections.Specialized;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

// 远程桌面里的复制粘贴文件（protocol.md 8.4.1）：本机剪贴板里的文件经内核传到对方暂存区并设成对方剪贴板；
// 对方剪贴板里的文件下载到本机缓存目录再放进本机剪贴板。这里是剪贴板读写、缓存目录、进度回调与进度条视图。

/// <summary>本机剪贴板里的文件与下载缓存目录</summary>
internal static class ClipFiles
{
    /// <summary>selftest 换成临时目录，不碰 %LOCALAPPDATA%</summary>
    public static string? RootOverride;
    public static string Root => RootOverride ?? AppInfo.ClipboardDir;
    private static readonly Random Rand = new();

    /// <summary>每次拉取一个新目录（&lt;毫秒时间戳&gt;-&lt;随机数&gt;），同名文件不会互相覆盖</summary>
    public static string NewPullDir()
    {
        int r;
        lock (Rand) r = Rand.Next(1_000_000);
        return IOPath.Combine(Root, $"{Clock.NowMs}-{r}");
    }

    /// <summary>删掉一天前的拉取目录（应用启动时在后台调用），返回删掉的个数</summary>
    public static int CleanStale(string root, TimeSpan? age = null)
    {
        var limit = age ?? TimeSpan.FromDays(1);
        var removed = 0;
        try
        {
            if (!Directory.Exists(root)) return 0;
            foreach (var d in Directory.GetDirectories(root))
            {
                try
                {
                    var c = Directory.GetCreationTimeUtc(d);
                    var w = Directory.GetLastWriteTimeUtc(d);
                    if (DateTime.UtcNow - (c > w ? c : w) <= limit) continue;
                    Directory.Delete(d, true);
                    removed++;
                }
                catch (Exception e) { Log.Info("清理剪贴板缓存失败：" + e.Message); }
            }
        }
        catch (Exception e) { Log.Info("清理剪贴板缓存失败：" + e.Message); }
        return removed;
    }

    /// <summary>两份远端文件列表是不是同一批（被控端回来的路径可能换了斜杠或盘符大小写）</summary>
    public static bool SameFiles(string[]? a, string[]? b)
    {
        if (a == null || b == null || a.Length != b.Length) return false;
        static string Norm(string p) => p.Replace('\\', '/').TrimEnd('/').ToLowerInvariant();
        for (var i = 0; i < a.Length; i++)
            if (Norm(a[i]) != Norm(b[i])) return false;
        return true;
    }

    /// <summary>远端路径的文件名（协议格式是 /C:/x 或 /home/x，兼容反斜杠）</summary>
    public static string BaseName(string path) => path.Split(new[] { '/', '\\' }, StringSplitOptions.RemoveEmptyEntries).LastOrDefault() ?? path;

    [DllImport("user32.dll")]
    private static extern uint GetClipboardSequenceNumber();

    /// <summary>剪贴板序号：每次有人写剪贴板都会变；拿不到时为 0</summary>
    public static uint Sequence()
    {
        try { return GetClipboardSequenceNumber(); }
        catch { return 0; }
    }

    /// <summary>资源管理器里复制的文件 / 文件夹；剪贴板里不是文件时为 null</summary>
    public static string[]? GetFiles()
    {
        for (var i = 0; i < 5; i++)
        {
            try
            {
                if (!Clipboard.ContainsFileDropList()) return null;
                var list = Clipboard.GetFileDropList();
                var arr = new string[list.Count];
                list.CopyTo(arr, 0);
                return arr.Length == 0 ? null : arr;
            }
            catch (COMException) { Thread.Sleep(30); }
            catch { return null; }
        }
        return null;
    }

    /// <summary>把文件放进剪贴板（CF_HDROP + Preferred DropEffect = 复制），资源管理器里粘贴即复制</summary>
    public static bool SetFiles(string[] paths)
    {
        var list = new StringCollection();
        list.AddRange(paths);
        for (var i = 0; i < 5; i++)
        {
            try
            {
                var data = new DataObject();
                data.SetFileDropList(list);
                data.SetData("Preferred DropEffect", new MemoryStream(BitConverter.GetBytes(1)));
                Clipboard.SetDataObject(data, true);
                return true;
            }
            catch (COMException) { Thread.Sleep(30); }
            catch (Exception e)
            {
                Log.Error("写剪贴板失败：" + e.Message);
                return false;
            }
        }
        return false;
    }
}

internal enum FileClipDirection { Push, Pull }
internal enum FileClipState { Running, Done, Failed }

/// <summary>一次复制粘贴文件在界面上的状态</summary>
internal sealed class FileClip
{
    public readonly FileClipDirection Direction;
    /// <summary>顶层文件 / 文件夹名</summary>
    public readonly string[] Names;
    public ulong Done, Total;
    public FileClipState State = FileClipState.Running;
    public string? Error;

    public FileClip(FileClipDirection direction, string[] names)
    {
        Direction = direction;
        Names = names;
    }

    public double Fraction => Total > 0 ? Math.Min(1, (double)Done / Total) : 0;
    public int Percent => (int)Math.Floor(Fraction * 100);
}

/// <summary>内核的进度回调（内核线程）→ 界面线程；进度合并成每次消息处理最多一次刷新</summary>
internal sealed class ClipTransferListener : TransferListener
{
    private readonly Action<ulong, ulong> progress;
    private readonly Action<string?> finished;
    private ulong lastDone, lastTotal;

    public ClipTransferListener(Action<ulong, ulong> progress, Action<string?> finished)
    {
        this.progress = progress;
        this.finished = finished;
    }

    public void OnProgress(ulong done, ulong total)
    {
        lock (this) { lastDone = done; lastTotal = total; }
        Ui.Coalesce(this, () =>
        {
            ulong d, t;
            lock (this) { d = lastDone; t = lastTotal; }
            progress(d, t);
        });
    }

    public void OnFinished(string? error) => Ui.Post(() => finished(error));
}

internal static class FileClipView
{
    /// <summary>远程画面上方的进度条：正在复制（百分比 + 取消）→ 完成提示 / 失败原因</summary>
    public static FrameworkElement? Pill(RemoteTab tab)
    {
        var c = tab.FileClip;
        if (c == null) return null;
        var running = c.State == FileClipState.Running;
        string title;
        if (running)
        {
            var key = c.Direction == FileClipDirection.Push ? "remote.files.pushing" : "remote.files.pulling";
            title = c.Names.Length == 1 ? L.T(key + "One", c.Names[0]) : L.T(key + "Many", c.Names.Length);
        }
        else if (c.State == FileClipState.Done) title = L.T(c.Direction == FileClipDirection.Push ? "remote.files.pushed" : "remote.files.pulled");
        else title = L.T("remote.files.failed", c.Error ?? "");

        var icon = running ? K.Icon(c.Direction == FileClipDirection.Push ? "upload" : "download", 16, Th.Accent)
            : c.State == FileClipState.Done ? K.Icon("checkCircle", 16, Th.Success) : K.Icon("warning", 16, Th.Danger);
        var text = K.Text(title, 13, Th.Text, running ? null : FontWeights.Medium, wrap: !running);
        if (running) text.TextTrimming = TextTrimming.CharacterEllipsis;

        UIElement body;
        if (running)
        {
            const double barWidth = 290;
            var bar = new Grid { Width = barWidth, Height = 4, HorizontalAlignment = HorizontalAlignment.Left };
            bar.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = Th.Hover2 });
            bar.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = Th.Accent, HorizontalAlignment = HorizontalAlignment.Left, Width = Math.Max(4, barWidth * c.Fraction) });
            var pct = K.Text(c.Percent + "%", 12.5, Th.Secondary, FontWeights.Medium, mono: true);
            body = K.Col(6, K.Row(8, K.Grow(text), pct), bar);
        }
        else
        {
            // 完成 / 失败提示按文字取宽（Flex 里 Grow 会撑满可用宽度，这里不用），过长时换行
            text.MaxWidth = 470;
            body = text;
        }

        var close = K.IconButton("close", L.T(running ? "common.cancel" : "common.close"), running ? tab.CancelFileClip : tab.DismissFileClip, size: 10, w: 22, h: 22);
        close.CornerRadius = new CornerRadius(11);
        close.Normal = Th.Hover2;
        close.UpdateLook();

        return new Border
        {
            Background = Th.Elevated, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10),
            Padding = new Thickness(12, 9, 9, 9), Margin = new Thickness(16, 12, 16, 0),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Top,
            Width = running ? 390 : double.NaN, MaxWidth = 560,
            Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 16, ShadowDepth = 3, Direction = 270, Opacity = 0.28 },
            Child = K.Row(10, icon, running ? K.Grow(body) : body, close),
        };
    }
}
