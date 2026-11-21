using System.Diagnostics;
using System.IO;
using System.Text;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

/// <summary>双栏文件管理里一侧的数据来源：本机、设备（端到端文件通道）、SSH 主机（SFTP）</summary>
internal sealed record FileEndpoint(string Kind, string Id)
{
    public static readonly FileEndpoint Local = new("local", "");
    public static FileEndpoint ForDevice(string id) => new("device", id);
    public static FileEndpoint ForSsh(string id) => new("ssh", id);
    public bool IsLocal => Kind == "local";
}

internal sealed class FileItem
{
    public string Name = "", Path = "", Owner = "";
    public FileKind Kind;
    public ulong Size;
    public uint Mode;
    public long Modified;
    public string? LinkTarget;
    /// <summary>本机「此电脑」里的磁盘</summary>
    public bool IsDrive;

    public static FileItem From(FileEntry e) => new()
    {
        Name = e.Name, Path = e.Path, Kind = e.Kind, Size = e.Size, Mode = e.Mode, Modified = e.ModifiedAt, Owner = e.Owner, LinkTarget = e.LinkTarget,
    };

    public bool IsDir => Kind == FileKind.Dir || IsDrive || Kind == FileKind.Symlink && LinkTarget != null && !IOPath.HasExtension(LinkTarget);
    public string ModeString => (Kind == FileKind.Dir ? "d" : Kind == FileKind.Symlink ? "l" : "-") + Fmt.Mode(Mode);

    private static readonly HashSet<string> TextExt = new(StringComparer.OrdinalIgnoreCase)
    {
        "txt", "md", "log", "yml", "yaml", "json", "toml", "ini", "conf", "cfg", "sh", "bash", "zsh", "py", "js", "ts", "html", "css", "xml", "env", "service",
        "csv", "rs", "go", "swift", "c", "h", "cpp", "java", "kt", "sql", "properties", "ps1", "bat", "cmd", "cs", "reg",
    };

    /// <summary>文本文件（可在线编辑）的粗略判断：按扩展名，或没有扩展名的小文件</summary>
    public bool LooksLikeText
    {
        get
        {
            var ext = IOPath.GetExtension(Name).TrimStart('.');
            if (TextExt.Contains(ext)) return true;
            return ext.Length == 0 && Kind == FileKind.File && Size < 512 * 1024;
        }
    }
}

internal interface IFileSource
{
    FileEndpoint Endpoint { get; }
    Task<string> Home();
    Task<List<FileItem>> List(string path);
    Task Mkdir(string path);
    Task Rename(string from, string to);
    Task Remove(string path, bool recursive);
    Task Chmod(string path, uint mode);
    Task<string> ReadText(string path);
    Task WriteText(string path, string text);
    string Join(string dir, string name);
    string Parent(string path);
    bool SupportsChmod { get; }
}

/// <summary>本机文件（Windows 路径；空路径表示「此电脑」，列出各个盘）</summary>
internal sealed class LocalFileSource : IFileSource
{
    /// <summary>selftest 用假目录，截图里不出现本机真实文件</summary>
    public static string? HomeOverride;
    public FileEndpoint Endpoint => FileEndpoint.Local;
    public bool SupportsChmod => false;

    public Task<string> Home() => Task.FromResult(HomeOverride ?? Environment.GetFolderPath(Environment.SpecialFolder.UserProfile));

    public Task<List<FileItem>> List(string path) => Task.Run(() =>
    {
        var list = new List<FileItem>();
        if (path.Length == 0)
        {
            foreach (var d in DriveInfo.GetDrives())
            {
                ulong size = 0;
                try { if (d.IsReady) size = (ulong)d.TotalSize; } catch { }
                list.Add(new FileItem { Name = d.Name.TrimEnd('\\'), Path = d.Name, Kind = FileKind.Dir, IsDrive = true, Size = size });
            }
            return list;
        }
        var dir = new DirectoryInfo(path);
        foreach (var fsi in dir.EnumerateFileSystemInfos())
        {
            try
            {
                var isDir = (fsi.Attributes & FileAttributes.Directory) != 0;
                var link = (fsi.Attributes & FileAttributes.ReparsePoint) != 0;
                var hidden = (fsi.Attributes & (FileAttributes.Hidden | FileAttributes.System)) != 0;
                list.Add(new FileItem
                {
                    Name = hidden && !fsi.Name.StartsWith(".", StringComparison.Ordinal) ? "." + fsi.Name : fsi.Name,
                    Path = fsi.FullName,
                    Kind = isDir ? FileKind.Dir : link ? FileKind.Symlink : FileKind.File,
                    Size = fsi is FileInfo fi ? (ulong)fi.Length : 0,
                    Mode = isDir ? 0x1EDu : (fsi.Attributes & FileAttributes.ReadOnly) != 0 ? 0x124u : 0x1A4u,
                    Modified = new DateTimeOffset(fsi.LastWriteTimeUtc).ToUnixTimeMilliseconds(),
                });
            }
            catch { }
        }
        return list;
    });

    /// <summary>隐藏文件在列表里显示名前加了「.」便于统一过滤；真实名字从路径取</summary>
    public static string RealName(FileItem i) => IOPath.GetFileName(i.Path.TrimEnd('\\')) is { Length: > 0 } n ? n : i.Name;

    public Task Mkdir(string path) => Task.Run(() => { Directory.CreateDirectory(path); });

    public Task Rename(string from, string to) => Task.Run(() =>
    {
        if (Directory.Exists(from)) Directory.Move(from, to);
        else File.Move(from, to);
    });

    public Task Remove(string path, bool recursive) => Task.Run(() =>
    {
        // 本机删除走回收站，可以找回
        if (Directory.Exists(path))
            Microsoft.VisualBasic.FileIO.FileSystem.DeleteDirectory(path, Microsoft.VisualBasic.FileIO.UIOption.OnlyErrorDialogs, Microsoft.VisualBasic.FileIO.RecycleOption.SendToRecycleBin);
        else
            Microsoft.VisualBasic.FileIO.FileSystem.DeleteFile(path, Microsoft.VisualBasic.FileIO.UIOption.OnlyErrorDialogs, Microsoft.VisualBasic.FileIO.RecycleOption.SendToRecycleBin);
    });

    public Task Chmod(string path, uint mode) => Task.CompletedTask;
    public Task<string> ReadText(string path) => Task.Run(() => File.ReadAllText(path, Encoding.UTF8));
    public Task WriteText(string path, string text) => Task.Run(() => File.WriteAllText(path, text, new UTF8Encoding(false)));

    public string Join(string dir, string name) => dir.Length == 0 ? name : IOPath.Combine(dir, name);

    public string Parent(string path)
    {
        if (path.Length == 0) return "";
        var p = IOPath.GetDirectoryName(path.TrimEnd('\\'));
        if (string.IsNullOrEmpty(p)) return "";
        // C:\ 的上一级是「此电脑」
        return path.TrimEnd('\\').Length <= 2 ? "" : p!;
    }

    public static void Reveal(string path)
    {
        try { Process.Start(new ProcessStartInfo("explorer.exe", "/select,\"" + path + "\"") { UseShellExecute = true }); } catch { }
    }
}

/// <summary>远端文件（内核 FileClient：设备文件通道或 SSH 的 SFTP；路径一律用「/」，Windows 被控端写成 /C:/Users/x）</summary>
internal sealed class RemoteFileSource : IFileSource
{
    public FileEndpoint Endpoint { get; }
    public readonly ICoreFiles Client;
    public RemoteFileSource(FileEndpoint ep, ICoreFiles client) { Endpoint = ep; Client = client; }
    public bool SupportsChmod => true;

    public Task<string> Home() => Client.Home();
    public async Task<List<FileItem>> List(string path) => (await Client.List(path)).Select(FileItem.From).ToList();
    public Task Mkdir(string path) => Client.Mkdir(path);
    public Task Rename(string from, string to) => Client.Rename(from, to);
    public Task Remove(string path, bool recursive) => Client.Remove(path, recursive);
    public Task Chmod(string path, uint mode) => Client.Chmod(path, mode);
    public Task<string> ReadText(string path) => Client.ReadText(path, 2 * 1024 * 1024);
    public Task WriteText(string path, string text) => Client.WriteText(path, text);
    public string Join(string dir, string name) => dir.EndsWith("/", StringComparison.Ordinal) ? dir + name : dir + "/" + name;

    public string Parent(string path)
    {
        var p = path.TrimEnd('/');
        var i = p.LastIndexOf('/');
        return i <= 0 ? "/" : p.Substring(0, i);
    }
}

// ---------- 传输队列 ----------

internal enum TransferState { Queued, Running, Paused, Done, Failed, Cancelled }

internal sealed class TransferItem : Observable
{
    public readonly string Id = Guid.NewGuid().ToString("N");
    public string Name = "";
    public bool Upload;
    public string Source = "", Destination = "", EndpointName = "";
    public ulong Done, Total;
    public TransferState State = TransferState.Queued;
    public string? Error;
    public double Speed;
    public ICoreTransfer? Handle;
    public Action? Restart;
    public Action? OnFinished;
    private (DateTime at, ulong done)? lastSample;

    public double Fraction => Total == 0 ? 0 : Math.Min(1, (double)Done / Total);

    public void Progress(ulong d, ulong t)
    {
        if (State is TransferState.Done or TransferState.Failed or TransferState.Cancelled) return;
        var now = DateTime.UtcNow;
        if (lastSample is { } ls && (now - ls.at).TotalSeconds > 0.3)
        {
            Speed = (d - Math.Min(d, ls.done)) / (now - ls.at).TotalSeconds;
            lastSample = (now, d);
        }
        else if (lastSample == null) lastSample = (now, d);
        Done = d;
        Total = t;
        if (State == TransferState.Queued) State = TransferState.Running;
        Notify();
    }

    public void Finished(string? error)
    {
        if (State == TransferState.Cancelled) return;
        if (error != null) { State = TransferState.Failed; Error = error; }
        else { State = TransferState.Done; Done = Total; OnFinished?.Invoke(); }
        Handle = null;
        Notify();
    }
}

internal sealed class TransferQueue : Observable
{
    public readonly List<TransferItem> Items = new();

    public int ActiveCount => Items.Count(i => i.State is TransferState.Running or TransferState.Queued or TransferState.Paused);

    public TransferItem Start(string name, bool upload, ICoreFiles client, string local, string remote, string endpointName, Action? onFinished = null)
    {
        var item = new TransferItem
        {
            Name = name, Upload = upload, Source = upload ? local : remote, Destination = upload ? remote : local, EndpointName = endpointName, OnFinished = onFinished,
        };
        Items.Insert(0, item);
        void Launch()
        {
            var l = new ListenerAdapter(item);
            item.State = TransferState.Running;
            item.Done = 0;
            item.Error = null;
            try { item.Handle = upload ? client.Upload(local, remote, l) : client.Download(remote, local, l); }
            catch (Exception e) { item.Finished(Fmt.Error(e)); }
            item.Notify();
        }
        item.Restart = Launch;
        Launch();
        Notify();
        return item;
    }

    public void Pause(TransferItem i) { i.Handle?.Pause(); i.State = TransferState.Paused; i.Notify(); Notify(); }
    public void Resume(TransferItem i) { i.Handle?.Resume(); i.State = TransferState.Running; i.Notify(); Notify(); }

    public void Cancel(TransferItem i)
    {
        i.State = TransferState.Cancelled;
        try { i.Handle?.Cancel(); } catch { }
        i.Handle = null;
        i.Notify();
        Notify();
    }

    public void Retry(TransferItem i) { i.Restart?.Invoke(); Notify(); }

    public void ClearFinished()
    {
        Items.RemoveAll(i => i.State is TransferState.Done or TransferState.Cancelled);
        Notify();
    }

    private sealed class ListenerAdapter : TransferListener
    {
        private readonly TransferItem item;
        private ulong lastDone, lastTotal;
        public ListenerAdapter(TransferItem i) { item = i; }

        public void OnProgress(ulong done, ulong total)
        {
            lock (this) { lastDone = done; lastTotal = total; }
            Ui.Coalesce(this, () =>
            {
                ulong d, t;
                lock (this) { d = lastDone; t = lastTotal; }
                item.Progress(d, t);
            });
        }

        public void OnFinished(string? error) => Ui.Post(() =>
        {
            item.Finished(error);
            AppModel.Current?.Transfers.Notify();
        });
    }
}
