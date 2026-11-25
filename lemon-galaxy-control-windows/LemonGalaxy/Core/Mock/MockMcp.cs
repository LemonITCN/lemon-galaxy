namespace LemonGalaxy.Core.Mock;

/// <summary>
/// 模拟本机 MCP 服务（docs/mcp.md）：不监听端口。第一次开启后有一个已连接的客户端 claude-code，带几条操作记录
/// （成功、失败、已拒绝、调用方已取消）和一条等批准的危险命令。批准、拒绝、输入密码、停止客户端按真内核的事件顺序回调
/// （ToolProposed → ToolStarted → ToolOutput → ToolFinished，客户端与待批准数的变化走 OnStatus）。
/// 端口小于 1024 时模拟「端口被占用」。selftest 用 Simulate* 造出连接确认、操作应用确认、要密码、调用方取消、大量记录。
/// </summary>
internal sealed class MockMcp
{
    public const ushort DefaultPort = 47821;
    /// <summary>与内核 galaxy-agent cancel_external 给的结果说明一致</summary>
    public const string CallerCancelled = "调用方已取消";
    private const string FirstToken = "lgm_bW9jay1MZW1vbkdhbGF4eS1NQ1AtdG9rZW4tZm9yLXNlbGZ0ZXN0";
    private const string SecondToken = "lgm_cmVnZW5lcmF0ZWQtTGVtb25HYWxheHktTUNQLXRva2VuLTJuZA";

    private readonly MockClient client;
    private readonly object lk = new();
    private readonly SerialQueue queue = new();
    private readonly List<Client> clients = new();
    private McpListener? listener;
    private bool running, seeded;
    private ushort port = DefaultPort;
    private AiMode mode = AiMode.ConfirmDangerous;
    private uint? trust;
    private string token = "";
    private string? error;
    private int ids;

    /// <summary>selftest 核对：密码送到了内核（不进操作记录）</summary>
    public string? LastSecret;
    /// <summary>selftest 核对：档位调整传到了内核</summary>
    public AiMode? LastMode;
    public uint? LastTrust;

    private sealed class Client
    {
        public string Key = "", Name = "", Version = "";
        public long LastActive;
        public bool Running;
        public readonly List<AgentMessage.Tool> History = new();
        public readonly Dictionary<string, ToolCallView> Approvals = new();
        /// <summary>request_id → 发起它的 request_secret 调用</summary>
        public readonly Dictionary<string, string> Secrets = new();
    }

    public MockMcp(MockClient client) { this.client = client; }

    private string Z(string zh, string en) => client.Zh ? zh : en;
    private string NewId() => "mcp-" + Interlocked.Increment(ref ids).ToString("D4", System.Globalization.CultureInfo.InvariantCulture);

    public McpStatus Status() { lock (lk) return StatusLocked(); }

    private McpStatus StatusLocked() => new(true, running, running ? $"http://127.0.0.1:{port}/mcp" : "", port, token,
        IOPath.Combine(AppInfo.DataDir, "mcp-endpoint.json"), running ? mode : null, error,
        clients.OrderByDescending(c => c.LastActive).Select(c => new McpClientInfo(c.Key, c.Name, c.Version, c.LastActive, c.Running, (uint)c.Approvals.Count)).ToArray());

    private void Emit(string key, AgentEvent e)
    {
        var l = listener;
        if (l != null) queue.Post(() => l.OnEvent(key, e));
    }

    private void EmitStatus()
    {
        McpStatus s;
        McpListener? l;
        lock (lk)
        {
            s = StatusLocked();
            l = listener;
        }
        if (l != null) queue.Post(() => l.OnStatus(s));
    }

    // ---------- 服务 ----------

    public async Task<McpStatus> Start(McpConfig config, McpListener l)
    {
        await client.Delay(200);
        var p = config.Port == 0 ? DefaultPort : config.Port;
        string? fail = null;
        lock (lk)
        {
            listener = l;
            // 重启：和真内核一样，旧的客户端智能体作废
            clients.Clear();
            port = p;
            if (p < 1024)
            {
                running = false;
                fail = Z($"地址已被占用（端口 {p} 可能被别的程序占用，可以换一个端口）", $"Address already in use (port {p} may be taken by another program; try another port)");
                error = fail;
            }
            else
            {
                running = true;
                mode = config.Mode;
                trust = config.TrustMinutes;
                error = null;
                if (token.Length == 0) token = FirstToken;
            }
        }
        EmitStatus();
        if (fail != null) throw new GalaxyException.Other(fail);
        bool seed;
        lock (lk)
        {
            seed = !seeded;
            seeded = true;
        }
        // 和真内核一样：外部客户端是开启之后才连上来的
        if (seed) queue.Post(Seed);
        return Status();
    }

    public void Stop()
    {
        lock (lk)
        {
            running = false;
            error = null;
            clients.Clear();
        }
        EmitStatus();
    }

    /// <summary>退出登录：外部客户端的智能体全部作废，服务继续开着（与内核 reset_clients 一致）</summary>
    public void ResetClients()
    {
        lock (lk) clients.Clear();
        EmitStatus();
    }

    public async Task<McpStatus> Regenerate()
    {
        await client.Delay(150);
        McpListener? l;
        bool wasRunning;
        McpConfig cfg;
        lock (lk)
        {
            token = token == FirstToken ? SecondToken : FirstToken;
            wasRunning = running;
            l = listener;
            cfg = new McpConfig(port, mode, trust);
        }
        if (wasRunning && l != null) return await Start(cfg, l);
        return Status();
    }

    public void SetMode(AiMode m, uint? minutes)
    {
        List<string> keys;
        lock (lk)
        {
            LastMode = m;
            LastTrust = minutes;
            if (!running) return;
            mode = m;
            trust = minutes;
            keys = clients.Select(c => c.Key).ToList();
        }
        long? until = m == AiMode.FullTrust && minutes is { } t ? MockData.Now + t * 60_000L : null;
        foreach (var k in keys) Emit(k, new AgentEvent.ModeChanged(m, m, until));
        EmitStatus();
    }

    public AgentMessage[] History(string key)
    {
        lock (lk) return clients.FirstOrDefault(c => c.Key == key)?.History.Cast<AgentMessage>().ToArray() ?? Array.Empty<AgentMessage>();
    }

    // ---------- 批准 / 密码 / 停止 ----------

    public void Decide(string key, string callId, ApprovalDecision d)
    {
        Client? c;
        ToolCallView? call = null;
        lock (lk)
        {
            c = clients.FirstOrDefault(x => x.Key == key);
            if (c == null || !c.Approvals.TryGetValue(callId, out call)) return;
            c.Approvals.Remove(callId);
            c.LastActive = MockData.Now;
        }
        if (call == null) return;
        if (d is ApprovalDecision.Deny deny)
            Finish(c, callId, new ToolResultView(false, Z("用户拒绝", "Denied by user"), deny.Reason ?? "", null, true));
        else if (call.Tool == "use_app")
            Finish(c, callId, Ok(Z("已允许", "Allowed"), ""));
        else
        {
            Emit(c.Key, new AgentEvent.ToolStarted(callId));
            EmitStatus();
            var output = call.Tool == "use_target" ? "" : Z("已删除 23 个文件\n", "23 files removed\n");
            if (output.Length > 0) Emit(c.Key, new AgentEvent.ToolOutput(callId, output));
            Finish(c, callId, Ok(call.Tool == "use_target" ? Z("已连接", "Connected") : Z("退出码 0", "Exit code 0"), output));
        }
        Idle(c);
    }

    public void ProvideSecret(string key, string requestId, string secret)
    {
        Client? c;
        string? callId = null;
        lock (lk)
        {
            c = clients.FirstOrDefault(x => x.Key == key);
            if (c == null || !c.Secrets.TryGetValue(requestId, out callId)) return;
            c.Secrets.Remove(requestId);
            LastSecret = secret;
        }
        if (callId == null) return;
        Finish(c, callId, Ok(Z("已输入", "Entered"), ""));
        Idle(c);
    }

    public void StopClient(string key)
    {
        Client? c;
        List<string> approvals, secrets;
        lock (lk)
        {
            c = clients.FirstOrDefault(x => x.Key == key);
            if (c == null) return;
            approvals = c.Approvals.Keys.ToList();
            secrets = c.Secrets.Values.ToList();
            c.Approvals.Clear();
            c.Secrets.Clear();
        }
        foreach (var id in approvals) Finish(c, id, new ToolResultView(false, Z("用户拒绝", "Denied by user"), Z("用户停止了任务", "Stopped by user"), null, true));
        foreach (var id in secrets) Finish(c, id, new ToolResultView(false, Z("用户没有输入", "Nothing was entered"), "", null, false));
        Idle(c);
    }

    // ---------- 事件 ----------

    private static ToolResultView Ok(string summary, string output) => new(true, summary, output, null, false);

    private ToolCallView Call(string tool, (string id, string name) target, string summary, string detail, RiskLevel risk, string[] reasons, string why)
        => new(NewId(), tool, target.id, target.name, summary, detail, risk, reasons, why, null);

    private Client ClientOrAdd(string key, string name, string version)
    {
        Client c;
        lock (lk)
        {
            var hit = clients.FirstOrDefault(x => x.Key == key);
            if (hit != null) return hit;
            c = new Client { Key = key, Name = name, Version = version, LastActive = MockData.Now };
            clients.Add(c);
        }
        EmitStatus();
        return c;
    }

    /// <summary>直接执行完的调用：提出 → 开始 → 输出 → 结束</summary>
    private void Done(Client c, ToolCallView call, ToolResultView result, bool started = true)
    {
        lock (lk)
        {
            c.History.Add(new AgentMessage.Tool(call, result));
            c.LastActive = MockData.Now;
        }
        Emit(c.Key, new AgentEvent.ToolProposed(call, false));
        if (started) Emit(c.Key, new AgentEvent.ToolStarted(call.CallId));
        if (started && result.Output.Length > 0) Emit(c.Key, new AgentEvent.ToolOutput(call.CallId, result.Output));
        Emit(c.Key, new AgentEvent.ToolFinished(call.CallId, result));
    }

    /// <summary>需要批准：挂着直到界面作答</summary>
    private string Ask(Client c, ToolCallView call)
    {
        lock (lk)
        {
            c.History.Add(new AgentMessage.Tool(call, null));
            c.Approvals[call.CallId] = call;
            c.LastActive = MockData.Now;
            c.Running = true;
        }
        Emit(c.Key, new AgentEvent.Status(true, 0));
        Emit(c.Key, new AgentEvent.ToolProposed(call, true));
        EmitStatus();
        return call.CallId;
    }

    private void Finish(Client c, string callId, ToolResultView r)
    {
        lock (lk)
        {
            var i = c.History.FindIndex(m => m.Call.CallId == callId);
            if (i >= 0) c.History[i] = new AgentMessage.Tool(c.History[i].Call, r);
        }
        Emit(c.Key, new AgentEvent.ToolFinished(callId, r));
    }

    /// <summary>没有挂着的步骤了：客户端回到空闲（与内核外部调用全部结束时一样发 Status 不在运行）</summary>
    private void Idle(Client c)
    {
        bool idle;
        lock (lk)
        {
            idle = c.Approvals.Count == 0 && c.Secrets.Count == 0;
            if (idle) c.Running = false;
        }
        if (idle) Emit(c.Key, new AgentEvent.Status(false, 0));
        EmitStatus();
    }

    private (string, string) Build => ("dev-build", "build-server-01");
    private (string, string) Office => ("dev-office", "OFFICE-PC-07");

    /// <summary>第一次开启：claude-code 的几条记录和一条等批准的危险命令</summary>
    private void Seed()
    {
        var c = ClientOrAdd("claude-code", "claude-code", "2.1.30");
        var none = Array.Empty<string>();
        var readOnly = new[] { Z("只读命令", "Read-only command") };
        Done(c, Call("list_devices", ("", ""), Z("查看设备与主机", "List devices and hosts"), "", RiskLevel.Read, none, ""),
            Ok(Z("6 台设备", "6 devices"), "build-server-01  linux    online\ngpu-node-02      linux    online\nOFFICE-PC-07     windows  online\nDesign-iMac      macos    online\nnas              linux    offline\nRECEPTION        windows  offline\n"));
        Done(c, Call("use_target", Build, Z("连接 build-server-01", "Connect to build-server-01"), "Ubuntu 24.04.1 LTS", RiskLevel.Read, none, ""), Ok(Z("已连接", "Connected"), ""));
        Done(c, Call("run_command", Build, "df -h", "df -h", RiskLevel.Read, readOnly, Z("查看各分区使用率", "Check usage of each partition")),
            Ok(Z("退出码 0", "Exit code 0"), "Filesystem      Size  Used Avail Use% Mounted on\n/dev/sda2       457G  201G  233G  47% /\n/dev/sdb1       3.6T  3.3T  290G  92% /data\n"));
        Done(c, Call("read_file", Build, "/var/log/nginx/error.log", "/var/log/nginx/error.log", RiskLevel.Read, none, Z("看看最近的报错", "Look at recent errors")),
            Ok(Z("读取 4.2 KB", "Read 4.2 KB"), "2026/11/21 09:31:02 [error] 2817#2817: *1043 upstream timed out (110: Connection timed out) while reading response header from upstream\n"));
        Done(c, Call("run_command", Build, "systemctl restart nginx", "systemctl restart nginx", RiskLevel.Dangerous, new[] { Z("重启系统服务", "Restarts a system service") },
            Z("让 nginx 重新加载配置", "Make nginx reload its configuration")), new ToolResultView(false, Z("用户拒绝", "Denied by user"), Z("业务高峰，先别重启", "Peak hours, do not restart now"), null, true), started: false);
        Done(c, Call("download_file", Build, "/data/exports/report-2026Q3.csv → C:\\Users\\alice\\Downloads\\report-2026Q3.csv", "", RiskLevel.Read, none, ""),
            new ToolResultView(false, Z("远端文件不存在", "Remote file not found"), "", null, false));
        Done(c, Call("screenshot", Office, Z("截取屏幕 1", "Capture display 1"), "", RiskLevel.Read, none, Z("看看报错窗口", "Look at the error dialog")),
            new ToolResultView(false, CallerCancelled, "", null, true), started: false);
        Ask(c, Call("run_command", Build, "find /data/backups -name '*.tar.gz' -mtime +30 -delete", "find /data/backups -name '*.tar.gz' -mtime +30 -delete", RiskLevel.Dangerous,
            new[] { Z("find -delete 会删除文件", "find -delete removes files"), Z("作用于 /data 下的大量文件", "Affects many files under /data") },
            Z("清理 30 天前的旧备份，释放 /data 空间", "Remove backups older than 30 days to free space on /data")));
    }

    // ---------- selftest ----------

    /// <summary>claude-code 第一次用 OFFICE-PC-07：连接确认</summary>
    public string SimulateUseTarget()
    {
        var c = ClientOrAdd("claude-code", "claude-code", "2.1.30");
        return Ask(c, Call("use_target", Office, Z("连接 OFFICE-PC-07", "Connect to OFFICE-PC-07"), "Windows 11 23H2", RiskLevel.Read,
            new[] { Z("对话里没有引用这台设备", "This device was not referenced in the conversation") }, Z("需要在这台电脑上截图查看报错窗口", "Need a screenshot of the error dialog on this PC")));
    }

    /// <summary>第二个客户端 codex 第一次操作 Design-iMac 上的 Safari：操作应用确认</summary>
    public string SimulateUseApp()
    {
        var c = ClientOrAdd("codex-mcp-client", "codex-mcp-client", "0.63.0");
        return Ask(c, Call("use_app", ("dev-imac", "Design-iMac"), Z("AI 想操作「Safari」", "AI wants to use \"Safari\""),
            Z("允许后，这个客户端可以读取「Safari」的界面并在里面点击、输入（不会占用你的鼠标键盘）", "Once allowed, this client can read the \"Safari\" window and click or type in it (without taking over your mouse and keyboard)"),
            RiskLevel.Normal, new[] { Z("第一次操作这个应用", "First time using this app") }, Z("在网页后台里核对发布状态", "Check the release status in the web console")));
    }

    /// <summary>claude-code 要 sudo 密码：返回 request_id</summary>
    public string SimulateSecret()
    {
        var c = ClientOrAdd("claude-code", "claude-code", "2.1.30");
        var call = Call("request_secret", Build, Z("输入 sudo 密码", "Enter sudo password"), "", RiskLevel.Normal, Array.Empty<string>(), Z("安装依赖需要 sudo", "Installing packages needs sudo"));
        var rid = "secret-" + NewId();
        lock (lk)
        {
            c.History.Add(new AgentMessage.Tool(call, null));
            c.Secrets[rid] = call.CallId;
            c.Running = true;
            c.LastActive = MockData.Now;
        }
        Emit(c.Key, new AgentEvent.Status(true, 0));
        Emit(c.Key, new AgentEvent.ToolProposed(call, false));
        Emit(c.Key, new AgentEvent.ToolStarted(call.CallId));
        Emit(c.Key, new AgentEvent.SecretRequested(rid, Z("deploy 的 sudo 密码", "sudo password for deploy"), "dev-build"));
        EmitStatus();
        return rid;
    }

    /// <summary>claude-code 一条要批准的命令（之后用 SimulateCallerCancel 模拟客户端断开）</summary>
    public string SimulatePending(string command)
    {
        var c = ClientOrAdd("claude-code", "claude-code", "2.1.30");
        return Ask(c, Call("run_command", Build, command, command, RiskLevel.Unknown, new[] { Z("看不懂的命令", "Unrecognized command") }, ""));
    }

    /// <summary>客户端断开（新版规范的取消方式）：撤销挂着的确认，卡片标成调用方已取消</summary>
    public void SimulateCallerCancel(string callId)
    {
        Client? c;
        lock (lk)
        {
            c = clients.FirstOrDefault(x => x.Approvals.ContainsKey(callId));
            c?.Approvals.Remove(callId);
        }
        if (c == null) return;
        Finish(c, callId, new ToolResultView(false, CallerCancelled, "", null, true));
        Idle(c);
    }

    /// <summary>大量直接执行完的只读调用（核对操作记录的上限）</summary>
    public void SimulateMany(int n)
    {
        var c = ClientOrAdd("claude-code", "claude-code", "2.1.30");
        for (var i = 0; i < n; i++)
            Done(c, Call("list_dir", Build, "/var/log/app-" + i, "", RiskLevel.Read, Array.Empty<string>(), ""), Ok(Z("12 项", "12 entries"), ""));
        EmitStatus();
    }
}
