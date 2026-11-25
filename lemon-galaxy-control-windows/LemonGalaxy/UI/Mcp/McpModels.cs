using System.Diagnostics;
using System.Globalization;
using System.Text;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

internal enum McpPhase { Auto, Pending, Running, Finished }
internal enum McpOutcome { None, Success, Failed, Denied, Cancelled }

/// <summary>一次工具调用：操作记录里的一行；需要批准时同时是一张待批准卡片</summary>
internal sealed class McpCall : Observable
{
    public readonly string ClientKey;
    public ToolCallView Call;
    /// <summary>界面收到的时间；从 mcp_history 补齐的没有时间</summary>
    public long? Time;
    public long Seq;
    public McpPhase Phase;
    public ToolResultView? Result;
    public string Output = "";
    public bool Expanded, OutputExpanded;
    /// <summary>待批准卡片：正在填拒绝理由</summary>
    public bool Denying;
    public string DenyReason = "";

    public McpCall(string clientKey, ToolCallView call)
    {
        ClientKey = clientKey;
        Call = call;
    }

    public string Id => McpHub.Key(ClientKey, Call.CallId);
    public bool IsUseTarget => Call.Tool == "use_target";
    public bool IsUseApp => Call.Tool == "use_app";
    /// <summary>第一次用某台设备 / 某个应用：只有允许与拒绝</summary>
    public bool IsSimpleApproval => IsUseTarget || IsUseApp;

    public McpOutcome Outcome => Result switch
    {
        null => McpOutcome.None,
        { Ok: true } => McpOutcome.Success,
        { Denied: true } r when r.Summary == McpHub.CallerCancelled => McpOutcome.Cancelled,
        { Denied: true } => McpOutcome.Denied,
        _ => McpOutcome.Failed,
    };
}

/// <summary>request_secret：界面上一个安全输入框，内容走 mcp_provide_secret，不经过外部智能体</summary>
internal sealed class McpSecret : Observable
{
    public readonly string ClientKey, RequestId, Prompt, TargetId;
    /// <summary>发起它的 request_secret 调用（那一步结束时收起）</summary>
    public readonly string? CallId;
    public long Seq;

    public McpSecret(string clientKey, string requestId, string prompt, string targetId, string? callId)
    {
        ClientKey = clientKey;
        RequestId = requestId;
        Prompt = prompt;
        TargetId = targetId;
        CallId = callId;
    }
}

/// <summary>新的待批准（主窗口据此闪任务栏、发通知）</summary>
internal sealed record McpAttention(string ClientName, string What, int Pending);

/// <summary>
/// 本机 MCP 服务（docs/mcp.md）：开关与重启、状态与客户端、待批准卡片、操作记录、档位。
/// 内核回调在任意线程，这里统一切回界面线程处理。设置（开关、端口、档位、时限）存在 AppSettings；登录后若开着就自动开启。
/// 退出登录时内核让所有外部客户端的智能体作废，服务继续开着（新调用会提示未登录）。
/// </summary>
internal sealed class McpHub
{
    /// <summary>操作记录最多留多少条</summary>
    public const int MaxLog = 200;
    /// <summary>一条记录的输出最多留多少字符（内核回给外部智能体的单次输出最多 64 KB）</summary>
    private const int MaxOutput = 64 * 1024;
    /// <summary>与内核 galaxy-agent cancel_external 给的结果说明一致：外部客户端断开（取消）时挂着的步骤标成这个</summary>
    public const string CallerCancelled = "调用方已取消";

    private readonly AppModel app;
    /// <summary>服务状态、客户端、待批准、档位变化</summary>
    public readonly Observable Changed = new();
    /// <summary>操作记录有增删（单条的变化走那一条自己的 Notify）</summary>
    public readonly Observable LogChanged = new();
    public readonly McpListener Listener;

    public McpStatus Status;
    /// <summary>正在开启、重启或换令牌</summary>
    public bool Busy;
    /// <summary>最近一次开启失败的原因（端口被占用等）</summary>
    public string? Error;
    public readonly List<McpCall> Approvals = new();
    public readonly List<McpSecret> Secrets = new();
    /// <summary>操作记录，最新的在前</summary>
    public readonly List<McpCall> Records = new();
    private readonly Dictionary<string, McpCall> calls = new();
    /// <summary>见过的调用（含已挤出记录的），补齐历史时不重复加回来</summary>
    private readonly HashSet<string> seen = new();
    private readonly Queue<string> seenOrder = new();
    private readonly Dictionary<string, string> names = new();
    private readonly ushort defaultPort;
    private long seq;
    private int gen;

    public bool TokenVisible;
    /// <summary>端口输入框里的文字（区域重建时保留）</summary>
    public string PortDraft;
    /// <summary>完全信任的到期时间（内核 ModeChanged 给出）</summary>
    public long? TrustUntilMs;
    public ImageData? ZoomImage;
    /// <summary>最近复制的内容（selftest 核对；selftest 不碰系统剪贴板）</summary>
    public string? LastCopied;
    public event Action<McpAttention>? Attention;

    public McpHub(AppModel app)
    {
        this.app = app;
        Listener = new ListenerAdapter(this);
        try { defaultPort = app.Client.McpDefaultPort(); }
        catch { defaultPort = 47821; }
        Status = Query();
        PortDraft = EffectivePort.ToString(CultureInfo.InvariantCulture);
    }

    public static string Key(string clientKey, string callId) => clientKey + "\n" + callId;

    private McpStatus Query()
    {
        try { return app.Client.McpStatus(); }
        catch (Exception e)
        {
            Log.Error("查询 MCP 服务状态失败：" + e.Message);
            return new McpStatus(true, false, "", EffectivePort, "", "", null, null, Array.Empty<McpClientInfo>());
        }
    }

    // ---------- 状态 ----------

    private AppSettings S => app.Settings;
    public bool Enabled => S.McpEnabled;
    public bool Running => Status.Running;
    public ushort DefaultPort => defaultPort;
    public ushort EffectivePort => S.McpPort is > 0 and <= 65535 ? (ushort)S.McpPort : defaultPort;
    public string Url => Status.Running && Status.Url.Length > 0 ? Status.Url : $"http://127.0.0.1:{EffectivePort}/mcp";
    public string Token => Status.Token;
    public AiMode Mode => S.McpMode;
    public int TrustMinutes => S.McpTrustMinutes;
    private uint? TrustArg => Mode == AiMode.FullTrust && TrustMinutes > 0 ? (uint)TrustMinutes : null;
    public int PendingCount => Approvals.Count + Secrets.Count;
    /// <summary>界面显示的错误：开着的时候不显示上一次的失败</summary>
    public string? ShownError => Running || Busy ? null : Error ?? Status.Error;
    public McpClientInfo[] Clients => Status.Running ? Status.Clients : Array.Empty<McpClientInfo>();

    public string ClientName(string key)
        => Status.Clients.FirstOrDefault(x => x.Key == key) is { Name.Length: > 0 } info ? info.Name : names.TryGetValue(key, out var n) ? n : key;

    /// <summary>待批准卡片与密码请求，按到达顺序</summary>
    public List<object> PendingItems()
        => Approvals.Select(a => (seq: a.Seq, item: (object)a)).Concat(Secrets.Select(s => (seq: s.Seq, item: (object)s))).OrderBy(x => x.seq).Select(x => x.item).ToList();

    // ---------- 开关、端口、档位、令牌 ----------

    /// <summary>登录后：开关开着就自动开启（服务已在运行时不动，退出登录不关服务）</summary>
    public void OnLogin()
    {
        if (Enabled && !Running && !Busy) Start();
    }

    /// <summary>退出登录：内核已让外部客户端的智能体作废（服务继续开着）；上一个账号的记录与卡片不留给下一个账号</summary>
    public void OnLogout()
    {
        Approvals.Clear();
        Secrets.Clear();
        Records.Clear();
        calls.Clear();
        TrustUntilMs = null;
        ZoomImage = null;
        Changed.Notify();
        LogChanged.Notify();
    }

    public void SetEnabled(bool on)
    {
        S.McpEnabled = on;
        if (on)
        {
            if (!Running) Start();
            return;
        }
        gen++;
        Busy = false;
        Error = null;
        try { app.Client.McpStop(); }
        catch (Exception e) { Log.Error("关闭 MCP 服务失败：" + e.Message); }
        Apply(Query());
    }

    /// <summary>按设置开启（或重启）。没登录时只记着，登录后再开</summary>
    public void Start()
    {
        if (!app.LoggedIn) { Changed.Notify(); return; }
        var my = ++gen;
        Busy = true;
        Error = null;
        Changed.Notify();
        var config = new McpConfig((ushort)(S.McpPort is > 0 and <= 65535 ? S.McpPort : 0), Mode, TrustArg);
        Ui.Run(async () =>
        {
            McpStatus? st = null;
            string? err = null;
            try { st = await app.Client.McpStart(config, Listener); }
            catch (Exception e) { err = Fmt.Error(e); }
            if (my != gen)
            {
                // 等待期间关掉了：开起来的这个也关掉
                if (st != null && !Enabled) { try { app.Client.McpStop(); } catch { } }
                return;
            }
            Busy = false;
            Error = err;
            // 以此刻的状态为准：等待期间已经可能有客户端连上来（回调先到）
            Apply(Query());
            if (st != null) SyncHistory();
        });
    }

    public static ushort? ParsePort(string s)
        => ushort.TryParse(s.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var p) && p > 0 ? p : null;

    public bool PortDraftValid => ParsePort(PortDraft) != null;
    public bool PortDraftChanged => ParsePort(PortDraft) is { } p && p != EffectivePort;

    /// <summary>端口改了点「应用」：开着就按新端口重启</summary>
    public void ApplyPort()
    {
        if (ParsePort(PortDraft) is not { } p) return;
        S.McpPort = p == defaultPort ? 0 : p;
        PortDraft = p.ToString(CultureInfo.InvariantCulture);
        if (Enabled && app.LoggedIn) Start();
        else Changed.Notify();
    }

    public void ResetPort()
    {
        PortDraft = defaultPort.ToString(CultureInfo.InvariantCulture);
        ApplyPort();
    }

    /// <summary>档位立即作用于所有外部客户端；完全信任可限时（0 为不限时）</summary>
    public void SetMode(AiMode mode, int trustMinutes)
    {
        if (mode == AiMode.Off) return;
        S.McpMode = mode;
        S.McpTrustMinutes = trustMinutes;
        if (mode != AiMode.FullTrust || trustMinutes == 0) TrustUntilMs = null;
        try { app.Client.McpSetMode(mode, TrustArg); }
        catch (Exception e) { app.ShowError(e); }
        Changed.Notify();
    }

    public void AskRegenerateToken()
        => app.Confirm(L.T("mcp.token.regenerate.title"), L.T("mcp.token.regenerate.message"), L.T("mcp.token.regenerate"), RegenerateToken);

    public void RegenerateToken()
    {
        var my = ++gen;
        Busy = true;
        Changed.Notify();
        Ui.Run(async () =>
        {
            McpStatus? st = null;
            try { st = await app.Client.McpRegenerateToken(); }
            catch (Exception e) { app.ShowError(e); }
            if (my != gen) return;
            Busy = false;
            Apply(Query());
            if (st != null) app.ShowToast(L.T("mcp.token.regenerated"));
        });
    }

    public void Copy(string text)
    {
        LastCopied = text;
        if (!app.SelfTest) SysClipboard.SetText(text);
        app.ShowToast(L.T("common.copied"));
    }

    // ---------- 批准、密码、停止 ----------

    public void Decide(McpCall c, ApprovalDecision d)
    {
        try { app.Client.McpDecide(c.ClientKey, c.Call.CallId, d); }
        catch (Exception e) { app.ShowError(e); return; }
        c.Denying = false;
        if (c.Phase == McpPhase.Pending && c.Result == null) c.Phase = McpPhase.Running;
        Approvals.Remove(c);
        c.Notify();
        Changed.Notify();
    }

    public void ProvideSecret(McpSecret s, string secret)
    {
        // 密码只交给内核送到被控端，不留在界面状态里，也不进操作记录
        try { app.Client.McpProvideSecret(s.ClientKey, s.RequestId, secret); }
        catch (Exception e) { app.ShowError(e); return; }
        Secrets.Remove(s);
        Changed.Notify();
    }

    /// <summary>
    /// 不输入：回空内容，内核按「用户取消」处理（什么都不发给设备，回给外部智能体「用户取消了输入」），
    /// 只影响这一步，不牵连这个客户端其他等待中的步骤
    /// </summary>
    public void CancelSecret(McpSecret s) => ProvideSecret(s, "");

    /// <summary>停下某个客户端：拒绝它等批准的步骤、停止正在跑的命令（之后的新调用照常）</summary>
    public void StopClient(string key)
    {
        try { app.Client.McpStopClient(key); }
        catch (Exception e) { app.ShowError(e); return; }
        foreach (var a in Approvals.Where(x => x.ClientKey == key).ToList())
        {
            Approvals.Remove(a);
            if (a.Result == null) a.Phase = McpPhase.Running;
            a.Notify();
        }
        Secrets.RemoveAll(x => x.ClientKey == key);
        Changed.Notify();
    }

    // ---------- 操作记录 ----------

    private McpCall? Find(string key, string callId) => calls.TryGetValue(Key(key, callId), out var c) ? c : null;

    private void Remember(string id)
    {
        if (!seen.Add(id)) return;
        seenOrder.Enqueue(id);
        while (seenOrder.Count > 5000) seen.Remove(seenOrder.Dequeue());
    }

    /// <summary>超过上限：挤掉最旧的（还没结束的继续留在索引里，结果到了照样能更新卡片）</summary>
    private void Trim()
    {
        while (Records.Count > MaxLog)
        {
            var last = Records[Records.Count - 1];
            Records.RemoveAt(Records.Count - 1);
            if (last.Result != null) calls.Remove(last.Id);
        }
    }

    /// <summary>重新进页面时用 mcp_history 补齐（界面一直在收事件，正常不会漏；漏掉的没有时间，排在后面）</summary>
    public void SyncHistory()
    {
        if (!Running) return;
        var added = false;
        foreach (var info in Status.Clients)
        {
            AgentMessage[] msgs;
            try { msgs = app.Client.McpHistory(info.Key); }
            catch (Exception e)
            {
                Log.Error("读取 MCP 操作记录失败：" + e.Message);
                continue;
            }
            var missing = new List<McpCall>();
            foreach (var m in msgs.OfType<AgentMessage.Tool>())
            {
                var id = Key(info.Key, m.Call.CallId);
                if (calls.TryGetValue(id, out var known))
                {
                    if (known.Result == null && m.Result != null)
                    {
                        known.Result = m.Result;
                        known.Phase = McpPhase.Finished;
                        if (m.Result.Output.Length > known.Output.Length) known.Output = m.Result.Output;
                        known.Notify();
                    }
                    continue;
                }
                if (seen.Contains(id)) continue;
                Remember(id);
                missing.Add(new McpCall(info.Key, m.Call)
                {
                    Result = m.Result, Output = m.Result?.Output ?? "", Phase = m.Result == null ? McpPhase.Running : McpPhase.Finished,
                });
            }
            // 内核给的是从旧到新：倒过来接在已有记录后面
            for (var i = missing.Count - 1; i >= 0; i--)
            {
                var c = missing[i];
                c.Seq = ++seq;
                calls[c.Id] = c;
                Records.Add(c);
                added = true;
            }
        }
        if (!added) return;
        Trim();
        LogChanged.Notify();
    }

    /// <summary>selftest：模拟界面漏收了全部事件（之后 SyncHistory 应从内核补齐）</summary>
    internal void ForgetRecordsForSelfTest()
    {
        Records.Clear();
        calls.Clear();
        seen.Clear();
        seenOrder.Clear();
        LogChanged.Notify();
    }

    // ---------- 内核回调（界面线程） ----------

    private void Apply(McpStatus s)
    {
        Status = s;
        foreach (var c in s.Clients)
            if (c.Name.Length > 0) names[c.Key] = c.Name;
        Reconcile();
        Changed.Notify();
    }

    /// <summary>服务关了、客户端被回收（重启、换令牌、退出登录、空闲 2 小时）：它们挂着的步骤不会再有结果，卡片收起、记录标成已取消</summary>
    private void Reconcile()
    {
        var alive = new HashSet<string>(Clients.Select(c => c.Key));
        foreach (var a in Approvals.Where(x => !alive.Contains(x.ClientKey)).ToList())
        {
            Approvals.Remove(a);
            Orphan(a);
        }
        Secrets.RemoveAll(x => !alive.Contains(x.ClientKey));
        foreach (var r in Records.Where(x => x.Result == null && !alive.Contains(x.ClientKey))) Orphan(r);
    }

    private static void Orphan(McpCall c)
    {
        if (c.Result != null) return;
        c.Phase = McpPhase.Finished;
        c.Result = new ToolResultView(false, CallerCancelled, "", null, true);
        c.Notify();
    }

    private void Raise(string key, string what) => Attention?.Invoke(new McpAttention(ClientName(key), what, PendingCount));

    /// <summary>通知里的一句话：工具名 + 摘要（连接设备、操作应用用内核给的摘要）</summary>
    public static string Describe(McpCall c)
    {
        var call = c.Call;
        if (c.IsUseTarget) return L.T("ai.useTarget.row", call.TargetName);
        var summary = call.Summary.Replace('\n', ' ').Trim();
        if (summary.Length > 80) summary = summary.Substring(0, 80) + "…";
        if (c.IsUseApp) return summary;
        var label = AiViews.ToolLabel(call.Tool);
        return summary.Length == 0 ? label : label + (L.IsChinese ? "：" : ": ") + summary;
    }

    internal void HandleStatus(McpStatus s)
    {
        Apply(s);
        // 别处（重启失败）带来的错误原因
        if (!s.Running && s.Error != null && !Busy) Error = s.Error;
    }

    internal void HandleEvent(string key, AgentEvent e)
    {
        switch (e)
        {
            case AgentEvent.ToolProposed p:
            {
                var c = Find(key, p.Call.CallId);
                if (c == null)
                {
                    c = new McpCall(key, p.Call) { Time = Clock.NowMs, Seq = ++seq };
                    calls[c.Id] = c;
                    Remember(c.Id);
                    Records.Insert(0, c);
                    Trim();
                    LogChanged.Notify();
                }
                else
                {
                    // 补齐历史时先拿到的：事件到了说明是刚发生的
                    c.Call = p.Call;
                    c.Time ??= Clock.NowMs;
                }
                if (p.NeedsApproval && c.Result == null)
                {
                    c.Phase = McpPhase.Pending;
                    c.Seq = c.Seq == 0 ? ++seq : c.Seq;
                    if (!Approvals.Contains(c)) Approvals.Add(c);
                    c.Notify();
                    Changed.Notify();
                    Raise(key, Describe(c));
                }
                else
                {
                    if (c.Result == null) c.Phase = McpPhase.Auto;
                    c.Notify();
                }
                break;
            }
            case AgentEvent.ToolStarted s:
                if (Find(key, s.CallId) is { } started)
                {
                    if (started.Result == null) started.Phase = McpPhase.Running;
                    if (Approvals.Remove(started)) Changed.Notify();
                    started.Notify();
                }
                break;
            case AgentEvent.ToolOutput o:
                if (Find(key, o.CallId) is { } output)
                {
                    output.Output += o.Text;
                    if (output.Output.Length > MaxOutput) output.Output = output.Output.Substring(output.Output.Length - MaxOutput);
                    Ui.Coalesce(output, output.Notify);
                }
                break;
            case AgentEvent.ToolFinished f:
                if (Find(key, f.CallId) is { } done)
                {
                    done.Result = f.Result;
                    done.Phase = McpPhase.Finished;
                    if (f.Result.Output.Length > done.Output.Length) done.Output = f.Result.Output;
                    var changed = Approvals.Remove(done);
                    if (done.Call.Tool == "request_secret")
                        changed |= Secrets.RemoveAll(x => x.ClientKey == key && (x.CallId == null || x.CallId == done.Call.CallId)) > 0;
                    if (!Records.Contains(done)) calls.Remove(done.Id);
                    done.Notify();
                    if (changed) Changed.Notify();
                }
                break;
            case AgentEvent.SecretRequested r:
            {
                // 发起它的是这个客户端最近一个还没结束的 request_secret
                var from = Records.FirstOrDefault(x => x.ClientKey == key && x.Call.Tool == "request_secret" && x.Result == null);
                Secrets.Add(new McpSecret(key, r.RequestId, r.Prompt, r.TargetId, from?.Call.CallId) { Seq = ++seq });
                Changed.Notify();
                Raise(key, r.Prompt);
                break;
            }
            case AgentEvent.Status { Running: false }:
            {
                // 这个客户端的调用全部结束：内核同时清掉了它的待批准
                var stale = Approvals.Where(x => x.ClientKey == key).ToList();
                foreach (var a in stale)
                {
                    Approvals.Remove(a);
                    Orphan(a);
                }
                if (stale.Count + Secrets.RemoveAll(x => x.ClientKey == key) > 0) Changed.Notify();
                break;
            }
            case AgentEvent.ModeChanged mc:
                TrustUntilMs = mc.Mode == AiMode.FullTrust ? mc.UntilMs : null;
                if (mc.Mode != AiMode.FullTrust && mc.Mode != AiMode.Off && Mode == AiMode.FullTrust && TrustMinutes > 0)
                {
                    // 完全信任到时：内核已回到之前的档位，设置跟着回去，之后新连的客户端也不再完全信任
                    S.McpMode = mc.Mode;
                    try { app.Client.McpSetMode(mc.Mode, null); }
                    catch (Exception ex) { Log.Error("调整 MCP 档位失败：" + ex.Message); }
                }
                Changed.Notify();
                break;
        }
    }

    /// <summary>内核回调（任意线程）→ 界面线程</summary>
    private sealed class ListenerAdapter : McpListener
    {
        private readonly McpHub hub;
        public ListenerAdapter(McpHub hub) { this.hub = hub; }
        public void OnStatus(McpStatus status) => Ui.Post(() => hub.HandleStatus(status));
        public void OnEvent(string clientKey, AgentEvent e) => Ui.Post(() => hub.HandleEvent(clientKey, e));
    }
}

/// <summary>外部智能体的配置片段（docs/mcp.md 第 5 节）</summary>
internal static class McpConfigs
{
    public const string ServerName = "lemongalaxy";
    /// <summary>selftest 固定可执行文件路径，截图稳定</summary>
    public static string? ExePathOverride;

    /// <summary>当前进程 exe 的完整路径（stdio 桥接由外部客户端用 --mcp-stdio 启动它）</summary>
    public static string ExePath
    {
        get
        {
            if (ExePathOverride != null) return ExePathOverride;
            try
            {
                using var p = Process.GetCurrentProcess();
                if (p.MainModule?.FileName is { Length: > 0 } f) return f;
            }
            catch { }
            return IOPath.Combine(AppInfo.BaseDir, "LemonGalaxy.exe");
        }
    }

    public static string ClaudeCode(string url, string token)
        => $"claude mcp add --transport http --scope user {ServerName} {url} --header \"Authorization: Bearer {token}\"";

    public static string Codex(string url, string token)
        => $"[mcp_servers.{ServerName}]\r\nurl = {Str(url)}\r\nhttp_headers = {{ \"Authorization\" = {Str("Bearer " + token)} }}";

    public static string Cursor(string url, string token)
        => "{\"mcpServers\":{\"" + ServerName + "\":{\"url\":" + Str(url) + ",\"headers\":{\"Authorization\":" + Str("Bearer " + token) + "}}}}";

    public static string Stdio(string exe)
        => "{\"mcpServers\":{\"" + ServerName + "\":{\"command\":" + Str(exe) + ",\"args\":[\"--mcp-stdio\"]}}}";

    /// <summary>JSON 字符串（Windows 路径里的反斜杠要转义）；TOML 的基本字符串转义规则相同</summary>
    public static string Str(string s)
    {
        var sb = new StringBuilder("\"");
        foreach (var ch in s)
        {
            switch (ch)
            {
                case '"': sb.Append("\\\""); break;
                case '\\': sb.Append("\\\\"); break;
                case '\n': sb.Append("\\n"); break;
                case '\r': sb.Append("\\r"); break;
                case '\t': sb.Append("\\t"); break;
                default:
                    if (ch < 0x20) sb.Append("\\u").Append(((int)ch).ToString("x4", CultureInfo.InvariantCulture));
                    else sb.Append(ch);
                    break;
            }
        }
        return sb.Append('"').ToString();
    }

    /// <summary>界面上遮住的令牌：只露出前缀</summary>
    public static string Mask(string token) => token.Length == 0 ? "" : (token.StartsWith("lgm_", StringComparison.Ordinal) ? "lgm_" : "") + new string('•', 24);
}
