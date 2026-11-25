using System.IO;
using System.IO.Pipes;
using System.Security.AccessControl;
using System.Security.Principal;
using System.Text;

namespace LemonGalaxy.Ipc;

/// <summary>
/// 连被控服务的本机控制接口（命名管道，docs/spec.md 3.0.2）。后台线程负责连接与读，断了每 2 秒重连；
/// 回复交给等它的请求，事件与连上 / 断开交回界面线程。
/// 连上后先核对管道的属主：系统服务的管道必须属于 SYSTEM 或管理员组，防止别的程序抢先建同名管道冒充服务骗取密码。
/// </summary>
internal sealed class ControlClient : IDisposable
{
    public const string SystemPipe = "LemonGalaxyWorker.control";
    private const string SystemSid = "S-1-5-18";
    private const string AdminsSid = "S-1-5-32-544";

    /// <summary>界面线程上调用</summary>
    public event Action? Connected;
    public event Action? Disconnected;
    public event Action<J>? EventReceived;

    private readonly object sync = new();
    private readonly Dictionary<long, TaskCompletionSource<Response>> pending = new();
    private NamedPipeClientStream? pipe;
    private long next = 1;
    private volatile bool disposed;
    private Thread? thread;
    private readonly Action<Action> post;

    public string? PipeName { get; private set; }
    public bool IsConnected { get { lock (sync) return pipe != null; } }

    /// <param name="post">把回调投到界面线程（测试时可以直接执行）</param>
    public ControlClient(Action<Action>? post = null)
    {
        this.post = post ?? (a => UI.Ui.Post(a));
    }

    public void Start()
    {
        thread = new Thread(Loop) { IsBackground = true, Name = "control" };
        thread.Start();
    }

    /// <summary>按顺序找服务：环境变量 LEMON_GALAXY_CONTROL_SOCKET（\\.\pipe\名字）&gt; 系统服务 &gt; 本用户前台运行的服务</summary>
    public static List<(string name, bool system)> Candidates()
    {
        var v = new List<(string, bool)>();
        var env = Environment.GetEnvironmentVariable("LEMON_GALAXY_CONTROL_SOCKET");
        if (!string.IsNullOrEmpty(env))
        {
            v.Add((StripPrefix(env!), false));
            return v;
        }
        v.Add((SystemPipe, true));
        var sid = WindowsIdentity.GetCurrent().User?.Value;
        if (sid != null) v.Add((SystemPipe + "." + sid, false));
        return v;
    }

    public static string StripPrefix(string name)
    {
        const string prefix = @"\\.\pipe\";
        return name.StartsWith(prefix, StringComparison.OrdinalIgnoreCase) ? name.Substring(prefix.Length) : name;
    }

    /// <summary>打开管道并核对属主。失败返回 null（没有这个服务、属主不对）。</summary>
    public static NamedPipeClientStream? Open(string name, bool system, int timeoutMs, out string? problem)
    {
        problem = null;
        var p = new NamedPipeClientStream(".", name, PipeDirection.InOut, PipeOptions.Asynchronous, TokenImpersonationLevel.Identification);
        try
        {
            p.Connect(timeoutMs);
        }
        catch
        {
            p.Dispose();
            return null;
        }
        try
        {
            var owner = p.GetAccessControl().GetOwner(typeof(SecurityIdentifier)) as SecurityIdentifier;
            var me = WindowsIdentity.GetCurrent().User;
            var ok = owner != null && (owner.Value == SystemSid || owner.Value == AdminsSid || (!system && me != null && owner.Equals(me)));
            if (!ok)
            {
                problem = $"管道 {name} 的属主不对（{owner?.Value ?? "?"}），不是 LemonGalaxy 被控服务";
                p.Dispose();
                return null;
            }
        }
        catch (Exception e)
        {
            problem = "读不到管道的属主：" + e.Message;
            p.Dispose();
            return null;
        }
        return p;
    }

    private void Loop()
    {
        while (!disposed)
        {
            foreach (var (name, system) in Candidates())
            {
                if (disposed) return;
                var p = Open(name, system, 400, out var problem);
                if (p == null)
                {
                    if (problem != null) Log.Error(problem);
                    continue;
                }
                Serve(p, name);
                break;
            }
            for (var i = 0; i < 20 && !disposed; i++) Thread.Sleep(100);
        }
    }

    private void Serve(NamedPipeClientStream p, string name)
    {
        lock (sync)
        {
            pipe = p;
            PipeName = name;
        }
        post(() => Connected?.Invoke());
        try
        {
            var reader = new StreamReader(p, new UTF8Encoding(false), false, 64 * 1024);
            string? line;
            while (!disposed && (line = reader.ReadLine()) != null)
            {
                if (line.Length == 0) continue;
                var j = J.Parse(line);
                if (j == null)
                {
                    Log.Error("控制接口：看不懂的一行：" + line);
                    continue;
                }
                if (j.O("resp") is { } resp)
                {
                    var id = j.L("id");
                    TaskCompletionSource<Response>? tcs;
                    lock (sync)
                    {
                        if (pending.TryGetValue(id, out tcs)) pending.Remove(id);
                    }
                    tcs?.TrySetResult(new Response(resp));
                }
                else if (j.O("event") is { } ev)
                {
                    post(() => EventReceived?.Invoke(ev));
                }
            }
        }
        catch (Exception e) when (e is IOException or ObjectDisposedException or InvalidOperationException) { }
        List<TaskCompletionSource<Response>> fail;
        lock (sync)
        {
            pipe = null;
            fail = pending.Values.ToList();
            pending.Clear();
        }
        try { p.Dispose(); } catch { }
        foreach (var t in fail) t.TrySetResult(Gone());
        if (!disposed) post(() => Disconnected?.Invoke());
    }

    public static Response Gone() => Response.Error(ErrorCode.Failed, L.T("svc.lost"));

    /// <summary>发请求，等回复。没连上服务时马上返回错误。</summary>
    public Task<Response> Call(Dictionary<string, object?> req)
    {
        var tcs = Bg.Tcs<Response>();
        NamedPipeClientStream? p;
        long id;
        lock (sync)
        {
            p = pipe;
            if (p == null) return Bg.Done(Gone());
            id = next++;
            pending[id] = tcs;
        }
        try
        {
            var bytes = new UTF8Encoding(false).GetBytes(Req.Line(id, req));
            lock (p) p.Write(bytes, 0, bytes.Length);
        }
        catch
        {
            lock (sync) pending.Remove(id);
            return Bg.Done(Gone());
        }
        return tcs.Task;
    }

    public void Dispose()
    {
        disposed = true;
        NamedPipeClientStream? p;
        lock (sync) p = pipe;
        try { p?.Dispose(); } catch { }
    }

    /// <summary>
    /// 一次性的同步调用（授权程序、命令行自检用）：连上、按顺序发几条请求、收齐回复、断开。
    /// </summary>
    public static List<Response> Once(string name, bool system, IEnumerable<Dictionary<string, object?>> reqs, int timeoutMs = 15000)
    {
        var p = Open(name, system, 3000, out var problem) ?? throw new IOException(problem ?? L.T("svc.notRunning"));
        using (p)
        {
            var enc = new UTF8Encoding(false);
            var reader = new StreamReader(p, enc, false, 64 * 1024);
            var outList = new List<Response>();
            long id = 0;
            var deadline = DateTime.UtcNow.AddMilliseconds(timeoutMs);
            foreach (var r in reqs)
            {
                id++;
                var bytes = enc.GetBytes(Req.Line(id, r));
                p.Write(bytes, 0, bytes.Length);
                while (true)
                {
                    if (DateTime.UtcNow > deadline) throw new TimeoutException();
                    var readTask = Bg.Run(() => reader.ReadLine());
                    if (!readTask.Wait(Math.Max(1, (int)(deadline - DateTime.UtcNow).TotalMilliseconds))) throw new TimeoutException();
                    var line = readTask.Result ?? throw new IOException(L.T("svc.lost"));
                    var j = J.Parse(line);
                    if (j?.O("resp") is { } resp && j.L("id") == id)
                    {
                        outList.Add(new Response(resp));
                        break;
                    }
                }
            }
            return outList;
        }
    }
}
