using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>双栏中的一栏</summary>
internal sealed class FilePane : Observable
{
    public FileEndpoint Endpoint;
    public string Path = "";
    public List<FileItem> Items = new();
    public HashSet<string> Selection = new();
    public string SortKey = "name";
    public bool Ascending = true;
    public bool Loading;
    public string? Error;
    public bool ShowHidden;
    public IFileSource? Source;
    public string HomePath = "";
    public readonly Stack<string> Back = new();
    public readonly SftpTab Tab;

    public FilePane(SftpTab tab, FileEndpoint ep) { Tab = tab; Endpoint = ep; }

    public List<FileItem> Visible
    {
        get
        {
            IEnumerable<FileItem> list = Items;
            if (!ShowHidden) list = list.Where(i => !i.Name.StartsWith(".", StringComparison.Ordinal));
            IOrderedEnumerable<FileItem> ordered = list.OrderBy(i => i.IsDir ? 0 : 1);
            ordered = SortKey switch
            {
                "size" => Ascending ? ordered.ThenBy(i => i.IsDir ? 0 : i.Size) : ordered.ThenByDescending(i => i.IsDir ? 0 : i.Size),
                "modified" => Ascending ? ordered.ThenBy(i => i.Modified) : ordered.ThenByDescending(i => i.Modified),
                _ => Ascending ? ordered.ThenBy(i => i.Name, StringComparer.CurrentCultureIgnoreCase) : ordered.ThenByDescending(i => i.Name, StringComparer.CurrentCultureIgnoreCase),
            };
            return ordered.ToList();
        }
    }

    public async Task Connect(FileEndpoint ep)
    {
        var app = Tab.AppRef;
        Endpoint = ep;
        Loading = true;
        Error = null;
        Items = new List<FileItem>();
        Notify();
        try
        {
            if (ep.IsLocal) Source = new LocalFileSource();
            else if (ep.Kind == "device") Source = new RemoteFileSource(ep, (await app.Peer(ep.Id)).Session.Files());
            else Source = new RemoteFileSource(ep, (await app.Ssh(ep.Id)).Session.Files());
            var home = await Source.Home();
            HomePath = home;
            Back.Clear();
            await Load(home);
        }
        catch (Exception e)
        {
            Error = Fmt.Error(e);
            Loading = false;
            Notify();
        }
    }

    public async Task Load(string p, bool pushHistory = false)
    {
        if (Source == null) return;
        Loading = true;
        Notify();
        try
        {
            var list = await Source.List(p);
            if (pushHistory && Path.Length > 0 && Path != p) Back.Push(Path);
            Path = p;
            Items = list;
            Selection.Clear();
            Error = null;
        }
        catch (Exception e) { Error = Fmt.Error(e); }
        Loading = false;
        Notify();
    }

    public void Refresh() => Ui.Run(() => Load(Path));
    public void GoUp() { if (Source != null) Ui.Run(() => Load(Source.Parent(Path), true)); }
    public void GoBack() { if (Back.Count > 0) { var prev = Back.Pop(); Ui.Run(() => Load(prev)); } }
    public void GoHome() { if (Source != null) Ui.Run(async () => await Load(await Source.Home(), true)); }

    public void Open(FileItem item)
    {
        if (item.IsDir) Ui.Run(() => Load(item.Path, true));
        else if (item.LooksLikeText && Source != null) TextEditorWindow.Open(Tab.AppRef, Source, item.Path, item.Name);
    }

    public string EndpointName => Endpoint.Kind switch
    {
        "local" => L.T("sftp.local"),
        "device" => Tab.AppRef.DeviceName(Endpoint.Id),
        _ => Tab.AppRef.SshLabel(Endpoint.Id),
    };

    public bool IsLocal => Endpoint.IsLocal;
    public string NameOf(FileItem i) => IsLocal ? LocalFileSource.RealName(i) : i.Name;
}

/// <summary>文件管理标签：本机 ↔ 远端、远端 ↔ 远端，传输进传输队列</summary>
internal sealed class SftpTab : TabModel
{
    public readonly FilePane LeftPane, RightPane;
    public bool QueueOpen = true;
    public AppModel AppRef => App;

    public SftpTab(AppModel app) : base(app, TabKind.Sftp, L.T("nav.sftp"))
    {
        LeftPane = new FilePane(this, FileEndpoint.Local);
        RightPane = new FilePane(this, FileEndpoint.Local);
    }

    public void Start(FileEndpoint? remote)
    {
        Ui.Run(() => LeftPane.Connect(FileEndpoint.Local));
        Ui.Run(async () =>
        {
            await RightPane.Connect(remote ?? FileEndpoint.Local);
            if (remote != null) Live = RightPane.Error == null;
        });
    }

    public FilePane Other(FilePane p) => ReferenceEquals(p, LeftPane) ? RightPane : LeftPane;

    /// <summary>把选中的条目传到另一栏的当前目录</summary>
    public void Transfer(List<FileItem> items, FilePane from)
    {
        var to = Other(from);
        if (from.Source == null || to.Source == null) return;
        foreach (var item in items)
        {
            var name = from.NameOf(item);
            var target = to.Source.Join(to.Path, name);
            switch (from.Source, to.Source)
            {
                case (LocalFileSource, RemoteFileSource r):
                    if (item.IsDir) UploadDir(item.Path, target, r, to);
                    else App.Transfers.Start(name, true, r.Client, item.Path, target, to.EndpointName, to.Refresh);
                    break;
                case (RemoteFileSource r, LocalFileSource):
                    if (item.IsDir) DownloadDir(item.Path, target, r, to, from.EndpointName);
                    else App.Transfers.Start(name, false, r.Client, target, item.Path, from.EndpointName, to.Refresh);
                    break;
                case (RemoteFileSource a, RemoteFileSource b):
                    // 远端到远端：先下到本机临时目录，再上传到另一端
                    if (item.IsDir) { App.ShowToast(L.T("sftp.remoteDirUnsupported"), true); continue; }
                    var tmp = IOPath.Combine(IOPath.GetTempPath(), "LemonGalaxy-" + Guid.NewGuid().ToString("N") + "-" + name);
                    App.Transfers.Start(name, false, a.Client, tmp, item.Path, from.EndpointName, () =>
                        App.Transfers.Start(name, true, b.Client, tmp, target, to.EndpointName, () =>
                        {
                            try { File.Delete(tmp); } catch { }
                            to.Refresh();
                        }));
                    break;
                default:
                    // 本机到本机：直接复制
                    try
                    {
                        if (item.IsDir) CopyDir(item.Path, target);
                        else File.Copy(item.Path, target, false);
                        to.Refresh();
                    }
                    catch (Exception e) { App.ShowError(e); }
                    break;
            }
        }
        QueueOpen = true;
        Notify();
    }

    private static void CopyDir(string src, string dst)
    {
        Directory.CreateDirectory(dst);
        foreach (var f in Directory.GetFiles(src)) File.Copy(f, IOPath.Combine(dst, IOPath.GetFileName(f)), false);
        foreach (var d in Directory.GetDirectories(src)) CopyDir(d, IOPath.Combine(dst, IOPath.GetFileName(d)));
    }

    private void UploadDir(string local, string remote, RemoteFileSource r, FilePane to)
    {
        Ui.Run(async () =>
        {
            try { await r.Mkdir(remote); } catch { }
            foreach (var path in Directory.EnumerateFileSystemEntries(local, "*", SearchOption.AllDirectories))
            {
                var rel = path.Substring(local.Length).TrimStart('\\').Replace('\\', '/');
                var dest = remote + "/" + rel;
                if (Directory.Exists(path)) { try { await r.Mkdir(dest); } catch { } }
                else App.Transfers.Start(rel, true, r.Client, path, dest, to.EndpointName, to.Refresh);
            }
            to.Refresh();
        });
    }

    private void DownloadDir(string remote, string local, RemoteFileSource r, FilePane to, string endpointName)
    {
        Ui.Run(async () =>
        {
            async Task Walk(string rdir, string ldir)
            {
                Directory.CreateDirectory(ldir);
                List<FileItem> list;
                try { list = await r.List(rdir); } catch { return; }
                foreach (var i in list)
                {
                    var lpath = IOPath.Combine(ldir, i.Name);
                    if (i.Kind == FileKind.Dir) await Walk(i.Path, lpath);
                    else App.Transfers.Start(i.Name, false, r.Client, lpath, i.Path, endpointName, to.Refresh);
                }
            }
            await Walk(remote, local);
            to.Refresh();
        });
    }

    /// <summary>从资源管理器拖进来的本机文件</summary>
    public void UploadFiles(string[] paths, FilePane pane)
    {
        if (pane.Source is LocalFileSource dst)
        {
            foreach (var p in paths)
            {
                try
                {
                    var target = dst.Join(pane.Path, IOPath.GetFileName(p));
                    if (Directory.Exists(p)) CopyDir(p, target);
                    else File.Copy(p, target, false);
                }
                catch (Exception e) { App.ShowError(e); }
            }
            pane.Refresh();
            return;
        }
        if (pane.Source is not RemoteFileSource r) return;
        foreach (var p in paths)
        {
            var name = IOPath.GetFileName(p.TrimEnd('\\'));
            var target = r.Join(pane.Path, name);
            if (Directory.Exists(p)) UploadDir(p, target, r, pane);
            else App.Transfers.Start(name, true, r.Client, p, target, pane.EndpointName, pane.Refresh);
        }
        QueueOpen = true;
        Notify();
    }

    protected override FrameworkElement CreateView() => SftpViews.TabView(App, this);
}
