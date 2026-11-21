using System.IO;
using System.Text;

namespace LemonGalaxy.Core.Mock;

/// <summary>模拟远端文件系统：内存里的一棵树。传输按定时器推进进度，文件名含 fail 的传到 60% 失败，用来演示重试。</summary>
internal sealed class MockFileClient : ICoreFiles
{
    private sealed class Node
    {
        public string Name;
        public FileKind Kind;
        public ulong Size;
        public uint Mode;
        public long Mtime;
        public string? Text;
        public string? LinkTarget;
        public readonly Dictionary<string, Node> Children = new();

        public Node(string name, FileKind kind, ulong size = 0, uint mode = 0x1A4, long mtime = 0, string? text = null)
        {
            Name = name; Kind = kind; Size = size; Mode = mode; Mtime = mtime == 0 ? MockData.Now : mtime; Text = text;
        }
    }

    private readonly MockClient client;
    private readonly Platform platform;
    private readonly string user;
    private readonly Node root = new("/", FileKind.Dir, mode: 0x1ED);
    private readonly object lk = new();
    public readonly string HomePath;

    public MockFileClient(MockClient client, Platform platform, string username)
    {
        this.client = client;
        this.platform = platform;
        user = username;
        HomePath = platform switch
        {
            Platform.Windows => $"/C:/Users/{username}",
            Platform.Macos => $"/Users/{username}",
            _ => username == "root" ? "/root" : $"/home/{username}",
        };
        Seed();
    }

    private void Seed()
    {
        var t = MockData.Now;
        const long day = 86_400_000;
        var home = Mkdirs(HomePath);
        Node Dir(Node parent, string name, long mtime, uint mode = 0x1ED)
        {
            var n = new Node(name, FileKind.Dir, 4096, mode, mtime);
            parent.Children[name] = n;
            return n;
        }
        void File(Node parent, string name, ulong size, long mtime, uint mode = 0x1A4, string? text = null)
            => parent.Children[name] = new Node(name, FileKind.File, text != null ? (ulong)Encoding.UTF8.GetByteCount(text) : size, mode, mtime, text);

        if (platform == Platform.Windows)
        {
            Dir(home, "Desktop", t - day);
            var docs = Dir(home, "Documents", t - day * 2);
            File(docs, "report-2026Q3.xlsx", 182_440, t - day * 3);
            File(docs, "meeting-notes.txt", 0, t - day, text: "2026-11-20 周会\n- 发布 0.1.0\n- 远程桌面测试\n");
            var dl = Dir(home, "Downloads", t - 3_600_000);
            File(dl, "LemonGalaxy-0.1.0.exe", 18_552_112, t - 3_600_000 * 2);
            File(dl, "drivers-fail.zip", 92_331_008, t - day * 9);
            File(home, "todo.txt", 0, t - 600_000, text: "1. 更新显卡驱动\n2. 备份 D 盘\n");
            Mkdirs("/D:/Backup");
            return;
        }
        var ssh = Dir(home, ".ssh", t - day * 50, 0x1C0);
        File(ssh, "authorized_keys", 0, t - day * 50, 0x180, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoKeyOnly alice@laptop\n");
        var app = Dir(home, "app", t - day);
        File(app, "config.yaml", 0, t - day, text: "server:\n  listen: 0.0.0.0:8080\n  workers: 8\nlog:\n  level: info\n  dir: /var/log/app\n");
        File(app, "app.bin", 48_221_184, t - day, 0x1ED);
        var st = Dir(app, "static", t - day * 4);
        File(st, "index.html", 0, t - day * 4, text: "<!doctype html>\n<title>Demo</title>\n<h1>It works</h1>\n");
        File(st, "logo.png", 24_518, t - day * 4);
        var logs = Dir(home, "logs", t - 600_000);
        File(logs, "app.log", 0, t - 120_000, text: string.Join("\n", MockShell.SyslogLines) + "\n");
        File(logs, "app.log.1.gz", 3_481_993, t - day);
        File(logs, "app.log.2.gz", 3_512_004, t - day * 2);
        var proj = Dir(home, "projects", t - day * 3);
        var names = new[] { "galaxy-web", "infra", "ml-pipeline", "scripts" };
        for (var i = 0; i < names.Length; i++) Dir(proj, names[i], t - day * (i + 2));
        File(home, "docker-compose.yml", 0, t - day * 2, text: "services:\n  nginx:\n    image: nginx:1.27\n    ports: [\"80:80\", \"443:443\"]\n  redis:\n    image: redis:7\n");
        File(home, "deploy.sh", 0, t - day * 9, 0x1ED, "#!/bin/sh\nset -e\ncd ~/app\n./app.bin --check\nsystemctl --user restart app\n");
        File(home, "README.md", 0, t - day * 50, text: "# build-server-01\n\nCI runner and artifact cache.\n");
        File(home, "backup-fail.tar.gz", 734_003_200, t - day * 6);
        home.Children["current"] = new Node("current", FileKind.Symlink, 18, 0x1FF, t - day) { LinkTarget = HomePath + "/app" };
        var etc = Mkdirs("/etc");
        File(etc, "hostname", 0, t - day * 100, text: "build-server-01\n");
        File(etc, "hosts", 0, t - day * 100, text: "127.0.0.1 localhost\n10.0.2.10 db-primary\n");
        Mkdirs("/var/log");
        Mkdirs("/tmp");
    }

    private Node Mkdirs(string path)
    {
        var n = root;
        foreach (var comp in path.Split(new[] { '/' }, StringSplitOptions.RemoveEmptyEntries))
        {
            if (!n.Children.TryGetValue(comp, out var c))
            {
                c = new Node(comp, FileKind.Dir, 4096, 0x1ED);
                n.Children[comp] = c;
            }
            n = c;
        }
        return n;
    }

    private Node? Find(string path)
    {
        var n = root;
        foreach (var comp in path.Split(new[] { '/' }, StringSplitOptions.RemoveEmptyEntries))
        {
            if (!n.Children.TryGetValue(comp, out var c)) return null;
            n = c;
        }
        return n;
    }

    private static (string parent, string name) Split(string path)
    {
        var p = path.Length > 1 ? path.TrimEnd('/') : path;
        var i = p.LastIndexOf('/');
        if (i < 0) return ("/", p);
        var parent = p.Substring(0, i);
        return (parent.Length == 0 ? "/" : parent, p.Substring(i + 1));
    }

    private FileEntry Entry(Node n, string path)
    {
        var mine = path.StartsWith(HomePath, StringComparison.Ordinal);
        return new FileEntry(n.Name, path, n.Kind, n.Size, n.Mode, n.Mtime, mine ? user : "root", mine ? user : "root", n.LinkTarget);
    }

    private GalaxyException NotFound(string p) => new GalaxyException.NotFound(client.Zh ? $"找不到 {p}" : $"No such file or directory: {p}");

    public async Task<string> Home() { await client.Delay(60); return HomePath; }

    public async Task<FileEntry[]> List(string path)
    {
        await client.Delay(180);
        lock (lk)
        {
            var n = Find(path);
            if (n == null || n.Kind != FileKind.Dir) throw NotFound(path);
            var b = path == "/" ? "" : path.TrimEnd('/');
            return n.Children.Values.Select(c => Entry(c, b + "/" + c.Name)).ToArray();
        }
    }

    public async Task<FileEntry> Stat(string path)
    {
        await client.Delay(40);
        lock (lk) return Entry(Find(path) ?? throw NotFound(path), path);
    }

    public async Task<string> Realpath(string path)
    {
        await client.Delay(40);
        lock (lk) return (Find(path) ?? throw NotFound(path)).LinkTarget ?? path;
    }

    public async Task<FileEntry[]> Search(string rootPath, string pattern, bool content)
    {
        await client.Delay(200);
        lock (lk)
        {
            var outList = new List<FileEntry>();
            void Walk(Node n, string p)
            {
                foreach (var c in n.Children.Values)
                {
                    var cp = (p == "/" ? "" : p) + "/" + c.Name;
                    if (c.Name.IndexOf(pattern, StringComparison.OrdinalIgnoreCase) >= 0 || content && (c.Text?.Contains(pattern) ?? false)) outList.Add(Entry(c, cp));
                    if (c.Kind == FileKind.Dir && outList.Count < 200) Walk(c, cp);
                }
            }
            var r = Find(rootPath);
            if (r != null) Walk(r, rootPath);
            return outList.ToArray();
        }
    }

    public async Task Mkdir(string path)
    {
        await client.Delay(100);
        lock (lk)
        {
            var (p, name) = Split(path);
            var parent = Find(p) ?? throw NotFound(p);
            if (parent.Children.ContainsKey(name)) throw new GalaxyException.Other(client.Zh ? "已存在同名文件或文件夹" : "A file or folder with that name already exists");
            parent.Children[name] = new Node(name, FileKind.Dir, 4096, 0x1ED);
        }
    }

    public async Task Rename(string from, string to)
    {
        await client.Delay(100);
        lock (lk)
        {
            var (fp, fname) = Split(from);
            var (tp, tname) = Split(to);
            var src = Find(fp);
            if (src == null || !src.Children.TryGetValue(fname, out var n)) throw NotFound(from);
            var dst = Find(tp) ?? throw NotFound(tp);
            src.Children.Remove(fname);
            n.Name = tname;
            n.Mtime = MockData.Now;
            dst.Children[tname] = n;
        }
    }

    public async Task Remove(string path, bool recursive)
    {
        await client.Delay(120);
        lock (lk)
        {
            var (p, name) = Split(path);
            var parent = Find(p);
            if (parent == null || !parent.Children.TryGetValue(name, out var n)) throw NotFound(path);
            if (n.Kind == FileKind.Dir && n.Children.Count > 0 && !recursive) throw new GalaxyException.Other(client.Zh ? "文件夹不是空的" : "The folder is not empty");
            parent.Children.Remove(name);
        }
    }

    public async Task Chmod(string path, uint mode)
    {
        await client.Delay(80);
        lock (lk) (Find(path) ?? throw NotFound(path)).Mode = mode & 0xFFF;
    }

    public async Task<string> ReadText(string path, ulong maxBytes)
    {
        await client.Delay(150);
        lock (lk)
        {
            var n = Find(path) ?? throw NotFound(path);
            if (n.Text == null) throw new GalaxyException.Other(client.Zh ? "这不是文本文件" : "This is not a text file");
            return n.Text.Length > (int)Math.Min(maxBytes, int.MaxValue) ? n.Text.Substring(0, (int)maxBytes) : n.Text;
        }
    }

    public async Task WriteText(string path, string text)
    {
        await client.Delay(150);
        lock (lk)
        {
            var (p, name) = Split(path);
            var parent = Find(p) ?? throw NotFound(p);
            if (!parent.Children.TryGetValue(name, out var n)) parent.Children[name] = n = new Node(name, FileKind.File);
            n.Text = text;
            n.Size = (ulong)Encoding.UTF8.GetByteCount(text);
            n.Mtime = MockData.Now;
        }
    }

    public ICoreTransfer Download(string remote, string local, TransferListener listener)
    {
        Node? n;
        lock (lk) n = Find(remote);
        var total = n?.Size ?? 0;
        var content = n?.Text;
        var fail = remote.IndexOf("fail", StringComparison.OrdinalIgnoreCase) >= 0;
        return new MockTransfer(Math.Max(total, 1), fail, client.Zh, listener, client.Options.LatencyScale, ok =>
        {
            if (!ok) return;
            // 文本文件写真实内容；二进制写占位字节（大文件只写 1 MB，避免占用磁盘）
            try
            {
                var dir = IOPath.GetDirectoryName(local);
                if (!string.IsNullOrEmpty(dir)) Directory.CreateDirectory(dir);
                File.WriteAllBytes(local, content != null ? Encoding.UTF8.GetBytes(content) : new byte[Math.Min(total, 1_048_576UL)]);
            }
            catch { }
        });
    }

    public ICoreTransfer Upload(string local, string remote, TransferListener listener)
    {
        ulong size = 0;
        string? text = null;
        try
        {
            var fi = new FileInfo(local);
            size = fi.Exists ? (ulong)fi.Length : 0;
            if (fi.Exists && size < 256 * 1024) text = File.ReadAllText(local, Encoding.UTF8);
        }
        catch { }
        var fail = local.IndexOf("fail", StringComparison.OrdinalIgnoreCase) >= 0;
        return new MockTransfer(Math.Max(size, 1), fail, client.Zh, listener, client.Options.LatencyScale, ok =>
        {
            if (!ok) return;
            lock (lk)
            {
                var (p, name) = Split(remote);
                var parent = Find(p);
                if (parent != null) parent.Children[name] = new Node(name, FileKind.File, size, 0x1A4, MockData.Now, text);
            }
        });
    }
}

/// <summary>模拟传输：约 40 MB/s 推进进度，支持暂停、继续、取消。</summary>
internal sealed class MockTransfer : ICoreTransfer
{
    private readonly ulong total;
    private ulong done;
    private bool paused, cancelled, finished;
    private readonly bool fail, zh;
    private readonly TransferListener listener;
    private readonly Action<bool> completion;
    private readonly Timer timer;
    private readonly object lk = new();
    private readonly ulong step;

    public MockTransfer(ulong total, bool fail, bool zh, TransferListener listener, double speedScale, Action<bool> completion)
    {
        this.total = total;
        this.fail = fail;
        this.zh = zh;
        this.listener = listener;
        this.completion = completion;
        // 每 100 毫秒推进一次；小文件至少走 3 步，好看到进度
        step = speedScale == 0 ? total / 3 + 1 : Math.Max(total / 3, 4_000_000UL);
        var period = speedScale == 0 ? 2 : 100;
        timer = new Timer(_ => Tick(), null, period, period);
    }

    private void Tick()
    {
        lock (lk)
        {
            if (paused || cancelled || finished) return;
            done = Math.Min(total, done + step);
            listener.OnProgress(done, total);
            if (fail && done * 10 >= total * 6) Finish(zh ? "连接中断（模拟失败）" : "Connection lost (simulated failure)");
            else if (done >= total)
            {
                completion(true);
                Finish(null);
            }
        }
    }

    private void Finish(string? error)
    {
        finished = true;
        timer.Dispose();
        listener.OnFinished(error);
    }

    public void Cancel()
    {
        lock (lk)
        {
            if (finished) return;
            cancelled = true;
            Finish(zh ? "已取消" : "Cancelled");
        }
    }

    public void Pause() { lock (lk) paused = true; }
    public void Resume() { lock (lk) paused = false; }
}
