namespace LemonGalaxy.Core.Mock;

/// <summary>
/// 模拟 AI 智能体：按脚本流式吐文字、提出工具调用、按权限档位决定是否要确认、等用户回应确认卡片 / 提问 / 密码。
/// 事件全部走 AgentListener.OnEvent，和真内核一致（与 Mac 端同一套脚本）。
/// </summary>
internal sealed class MockConversation : ICoreConversation
{
    private readonly MockClient client;
    private ConversationConfig config;
    public AgentListener Listener;
    private readonly string convId = "conv-" + Guid.NewGuid().ToString("N").Substring(0, 8);
    private readonly object lk = new();
    private readonly List<AgentMessage> history = new();
    private AiMode mode;
    private DateTime? trustUntil;
    private CancellationTokenSource? cts;
    private readonly Dictionary<string, TaskCompletionSource<ApprovalDecision>> approvals = new();
    private readonly Dictionary<string, TaskCompletionSource<string>> answers = new();
    private readonly Dictionary<string, TaskCompletionSource<string?>> secrets = new();
    private readonly HashSet<string> allowSimilar = new();
    private ulong promptTokens, completionTokens;
    private string titleText;
    private long updatedAt = MockData.Now;
    private uint step;
    private readonly SerialQueue events = new();
    /// <summary>本次任务操作的目标；对话里引用过（或用户同意连接过）的设备与主机</summary>
    private AgentTarget? current;
    private readonly HashSet<string> authorized = new();

    public MockConversation(MockClient client, ConversationConfig config, AgentListener listener)
    {
        this.client = client;
        this.config = config;
        Listener = listener;
        mode = config.Mode;
        trustUntil = config.TrustMinutes is { } m ? DateTime.Now.AddMinutes(m) : null;
        titleText = config.Title ?? "";
        foreach (var t in config.Targets) authorized.Add(t.Id);
    }

    private bool Zh => client.Zh;
    private string Z(string zh, string en) => Zh ? zh : en;

    public string Id() => convId;
    public string Title() { lock (lk) return titleText; }
    public void SetForeground(bool foreground) { if (!foreground) Stop(); }
    public AgentMessage[] Messages() { lock (lk) return history.ToArray(); }

    public ConversationSummary Summary()
    {
        lock (lk)
            return new ConversationSummary(convId, titleText.Length == 0 ? Z("新对话", "New conversation") : titleText, config.ModelId,
                authorized.Select(id => client.TargetName(new AgentTarget(client.Device(id) != null ? AgentTargetKind.Device : AgentTargetKind.SshHost, id))).ToArray(), updatedAt,
                config with { Mode = mode });
    }

    private void Emit(AgentEvent e)
    {
        var l = Listener;
        events.Post(() => l.OnEvent(e));
    }

    private void Record(AgentMessage m)
    {
        lock (lk)
        {
            history.Add(m);
            updatedAt = MockData.Now;
        }
    }

    private Task Pause(int ms) => client.Delay(ms);

    public void Send(string text) => SendWithRefs(text, Array.Empty<ContextRef>());

    public void SendWithRefs(string text, ContextRef[] refs)
    {
        lock (lk)
            if (titleText.Length == 0)
            {
                var t = string.Join(" ", text.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));
                titleText = t.Length == 0 ? refs.FirstOrDefault()?.Label ?? "" : t.Length > 40 ? t.Substring(0, 40) : t;
            }
        Record(new AgentMessage.User(text, refs));
        cts?.Cancel();
        var c = new CancellationTokenSource();
        cts = c;
        Task.Run(async () =>
        {
            try { await RunScript(text, refs, c.Token); }
            catch (OperationCanceledException) { }
            catch (Exception e) { Log.Error("模拟 AI 脚本出错：" + e); }
        });
    }

    public void SetModel(ulong providerId, string modelId)
    {
        lock (lk) config = config with { ProviderId = providerId, ModelId = modelId };
    }

    public ConversationModel Model() { lock (lk) return new ConversationModel(config.ProviderId, config.ModelId); }

    public void Decide(string callId, ApprovalDecision decision)
    {
        TaskCompletionSource<ApprovalDecision>? t;
        lock (lk) { approvals.TryGetValue(callId, out t); approvals.Remove(callId); }
        t?.TrySetResult(decision);
    }

    public void Answer(string questionId, string answer)
    {
        TaskCompletionSource<string>? t;
        lock (lk) { answers.TryGetValue(questionId, out t); answers.Remove(questionId); }
        t?.TrySetResult(answer);
    }

    public void ProvideSecret(string requestId, string secret)
    {
        TaskCompletionSource<string?>? t;
        lock (lk) { secrets.TryGetValue(requestId, out t); secrets.Remove(requestId); }
        // 密码只交给被控端，不进模型上下文、不进历史；空字符串表示用户取消
        t?.TrySetResult(secret.Length == 0 ? null : secret);
    }

    public void SetMode(AiMode m, uint? trustMinutes)
    {
        DateTime? until = trustMinutes is { } t ? DateTime.Now.AddMinutes(t) : null;
        lock (lk)
        {
            mode = m;
            trustUntil = until;
        }
        // 实际档位 = min(界面选择, 管理员上限, 被控端上限)；模拟内核的上限都是完全信任
        Emit(new AgentEvent.ModeChanged(m, m, until.HasValue ? new DateTimeOffset(until.Value).ToUnixTimeMilliseconds() : null));
    }

    public void Stop()
    {
        cts?.Cancel();
        List<TaskCompletionSource<ApprovalDecision>> a;
        List<TaskCompletionSource<string>> q;
        List<TaskCompletionSource<string?>> s;
        lock (lk)
        {
            a = approvals.Values.ToList(); approvals.Clear();
            q = answers.Values.ToList(); answers.Clear();
            s = secrets.Values.ToList(); secrets.Clear();
        }
        a.ForEach(x => x.TrySetResult(new ApprovalDecision.Deny(null)));
        q.ForEach(x => x.TrySetResult(""));
        s.ForEach(x => x.TrySetResult(null));
        Emit(new AgentEvent.Status(false, step));
    }

    // ---------- 脚本 ----------

    private AgentTarget Target => current ?? config.Targets.FirstOrDefault() ?? new AgentTarget(AgentTargetKind.Device, "dev-build");
    private string TargetName => client.TargetName(Target);
    private bool IsGui => client.TargetPlatform(Target) is Platform.Windows or Platform.Macos;
    private static string ShortId() => Guid.NewGuid().ToString("N").Substring(0, 6);

    private async Task Say(string text, CancellationToken ct)
    {
        var id = "a-" + ShortId();
        var acc = "";
        var chunk = "";
        foreach (var ch in text)
        {
            chunk += ch;
            if (chunk.Length < 4) continue;
            Emit(new AgentEvent.AssistantDelta(id, chunk));
            acc += chunk;
            chunk = "";
            await Pause(30);
            ct.ThrowIfCancellationRequested();
        }
        if (chunk.Length > 0) { Emit(new AgentEvent.AssistantDelta(id, chunk)); acc += chunk; }
        Emit(new AgentEvent.AssistantDone(id));
        Record(new AgentMessage.Assistant(id, acc, ""));
        completionTokens += (ulong)acc.Length * 2;
        promptTokens += 1800;
        Emit(new AgentEvent.Usage(promptTokens, completionTokens));
    }

    /// <summary>null 表示被拒绝（档位不允许）；true 要确认；false 自动执行</summary>
    private bool? NeedsApproval(RiskLevel risk, string tool)
    {
        AiMode m;
        DateTime? until;
        bool similar;
        lock (lk) { m = mode; until = trustUntil; similar = allowSimilar.Contains(tool + risk); }
        if (risk == RiskLevel.Forbidden) return null;
        if (similar) return false;
        var effective = m;
        if (effective == AiMode.FullTrust && until is { } u && u < DateTime.Now) effective = AiMode.ConfirmDangerous;
        return effective switch
        {
            AiMode.Off => null,
            AiMode.ReadOnly => risk == RiskLevel.Read ? false : null,
            AiMode.ConfirmAll => risk != RiskLevel.Read,
            AiMode.ConfirmDangerous => risk is RiskLevel.Dangerous or RiskLevel.Unknown,
            _ => false,
        };
    }

    private async Task<string?> Tool(CancellationToken ct, string tool, string summary, RiskLevel risk, string[] reasons, string why, string output, string resultSummary,
        ImageData? image = null, ImageData? resultImage = null, bool account = false, string? detail = null)
    {
        step++;
        Emit(new AgentEvent.Status(true, step));
        var callId = "call-" + ShortId();
        var call = new ToolCallView(callId, tool, account ? "" : Target.Id, account ? "" : TargetName, summary, detail ?? (account ? "" : summary), risk, reasons, why, image);
        var approval = NeedsApproval(risk, tool);
        Emit(new AgentEvent.ToolProposed(call, approval == true));
        var finalCommand = summary;
        if (approval == null)
        {
            var r = new ToolResultView(false, risk == RiskLevel.Forbidden ? Z("绝对禁止的操作，已拒绝", "Forbidden operation, refused") : Z("被拒绝：当前为只读模式", "Denied: read-only mode"), "", null, true);
            Emit(new AgentEvent.ToolFinished(callId, r));
            Record(new AgentMessage.Tool(call, r));
            return null;
        }
        if (approval == true)
        {
            var tcs = new TaskCompletionSource<ApprovalDecision>(TaskCreationOptions.RunContinuationsAsynchronously);
            lock (lk) approvals[callId] = tcs;
            var decision = await tcs.Task;
            switch (decision)
            {
                case ApprovalDecision.Deny d:
                    var r = new ToolResultView(false, Z("用户拒绝", "Denied by user") + (d.Reason is { } reason ? Z("：", ": ") + reason : ""), "", null, true);
                    Emit(new AgentEvent.ToolFinished(callId, r));
                    Record(new AgentMessage.Tool(call, r));
                    return null;
                case ApprovalDecision.AllowSimilar:
                    lock (lk) allowSimilar.Add(tool + risk);
                    break;
                case ApprovalDecision.EditAndRun e:
                    finalCommand = e.Command;
                    break;
            }
        }
        ct.ThrowIfCancellationRequested();
        Emit(new AgentEvent.ToolStarted(callId));
        await Pause(250);
        foreach (var line in output.Split('\n'))
        {
            Emit(new AgentEvent.ToolOutput(callId, line + "\n"));
            await Pause(40);
            ct.ThrowIfCancellationRequested();
        }
        var res = new ToolResultView(true, finalCommand == summary ? resultSummary : Z("已执行修改后的命令", "Ran the edited command"), output, resultImage, false);
        Emit(new AgentEvent.ToolFinished(callId, res));
        Record(new AgentMessage.Tool(call, res));
        return output;
    }

    private async Task<string> Ask(string question, string[] options)
    {
        var qid = "q-" + ShortId();
        var tcs = new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        lock (lk) answers[qid] = tcs;
        Emit(new AgentEvent.AskUser(qid, question, options));
        Emit(new AgentEvent.Status(true, step));
        return await tcs.Task;
    }

    private async Task<string?> Secret(string prompt)
    {
        var rid = "s-" + ShortId();
        var tcs = new TaskCompletionSource<string?>(TaskCreationOptions.RunContinuationsAsynchronously);
        lock (lk) secrets[rid] = tcs;
        Emit(new AgentEvent.SecretRequested(rid, prompt, Target.Id));
        return await tcs.Task;
    }

    private async Task RunScript(string text, ContextRef[] refs, CancellationToken ct)
    {
        step = 0;
        Emit(new AgentEvent.Status(true, 0));
        Emit(new AgentEvent.Thinking("m-" + ShortId(), Z("先弄清楚用户要处理哪台设备，再决定下一步。", "Work out which device the user means, then decide what to do.")));
        await Pause(300);
        foreach (var r in refs.Where(r => r.Kind is ContextRefKind.Device or ContextRefKind.SshHost)) lock (lk) authorized.Add(r.Id);

        // 引用了设备 / 主机 / 文件：直接在那台上做；没引用但消息里点了名：先请用户同意连接
        var pick = refs.FirstOrDefault(r => r.Kind is ContextRefKind.Device or ContextRefKind.SshHost);
        var fileRef = refs.FirstOrDefault(r => r.Kind is ContextRefKind.File or ContextRefKind.Folder);
        if (pick != null) current = new AgentTarget(pick.Kind == ContextRefKind.Device ? AgentTargetKind.Device : AgentTargetKind.SshHost, pick.Id);
        else if (fileRef != null) current = new AgentTarget(client.Device(fileRef.TargetId) != null ? AgentTargetKind.Device : AgentTargetKind.SshHost, fileRef.TargetId);
        else if (client.DevicesSnapshot().FirstOrDefault(d => !d.IsSelf && d.Kind != DeviceKind.Mobile && text.IndexOf(d.Hostname, StringComparison.OrdinalIgnoreCase) >= 0) is { } named)
        {
            current = new AgentTarget(AgentTargetKind.Device, named.Id);
            if (!await UseTarget(ct)) { Emit(new AgentEvent.Status(false, step)); return; }
        }
        else current = null;

        if (refs.Any(r => r.Kind == ContextRefKind.Activity))
        {
            var items = await client.AccountActivity(20);
            var lines = string.Join("\n", items.Select(a => $"{a.Action}  {a.ActorName}  {a.TargetName}  {a.Ip}"));
            await Tool(ct, "recent_activity", Z("查看最近的连接记录", "Check recent activity"), RiskLevel.Read, Array.Empty<string>(), "", lines, Z($"{items.Length} 条", $"{items.Length} items"), account: true);
            await Say(Z("最近 2 小时有 3 条记录，都来自熟悉的 IP，没有异常：\n\n| 时间 | 用户 | 操作 | 设备 |\n|---|---|---|---|\n| 10 分钟前 | alice | 打开终端 | build-server-01 |\n| 1 小时前 | alice | 登录 | — |\n| 2 小时前 | bob | 结束远程桌面 | OFFICE-PC-07 |",
                "Three entries in the last 2 hours, all from known IPs. Nothing unusual:\n\n| Time | User | Action | Device |\n|---|---|---|---|\n| 10 min ago | alice | Opened terminal | build-server-01 |\n| 1 h ago | alice | Signed in | — |\n| 2 h ago | bob | Ended remote desktop | OFFICE-PC-07 |"), ct);
            Emit(new AgentEvent.Status(false, step));
            return;
        }
        if (current == null)
        {
            await ChatScript(ct);
            Emit(new AgentEvent.Status(false, step));
            return;
        }
        if (config.PlanFirst)
        {
            await Say(Z("计划：\n\n1. 查看磁盘与目录占用\n2. 找出可清理的旧文件\n3. 经你确认后清理\n4. 复查结果\n\n同意后我开始执行。",
                "Plan:\n\n1. Check disk and directory usage\n2. Find old files that can be removed\n3. Clean up after your approval\n4. Verify the result\n\nI will start once you agree."), ct);
            var a = await Ask(Z("按这个计划执行吗？", "Proceed with this plan?"), Zh ? new[] { "执行", "取消" } : new[] { "Proceed", "Cancel" });
            if (a != Z("执行", "Proceed")) { Emit(new AgentEvent.Status(false, step)); return; }
        }
        if (IsGui) await GuiScript(ct);
        else await ShellScript(ct);
        Emit(new AgentEvent.Status(false, step));
    }

    /// <summary>没引用设备时：账号级只读查询 + 普通回答（演示 Markdown）</summary>
    private async Task ChatScript(CancellationToken ct)
    {
        var devices = client.DevicesSnapshot().Where(d => !d.IsSelf && d.Kind != DeviceKind.Mobile).ToArray();
        var list = string.Join("\n", devices.Select(d => $"{d.Hostname}  {d.Platform}  {(d.Online ? "online" : "offline")}"));
        await Tool(ct, "list_devices", Z("查看设备与主机", "List devices and hosts"), RiskLevel.Read, Array.Empty<string>(), "", list, Z($"{devices.Length} 台设备", $"{devices.Length} devices"), account: true);
        var online = devices.Count(d => d.Online);
        await Say(Z($"你的账号里有 **{devices.Length} 台设备**，其中 {online} 台在线。\n\n## 在线设备\n\n- **build-server-01**：Ubuntu 24.04，构建服务器\n- **gpu-node-02**：Debian 12\n- **OFFICE-PC-07**：Windows 11\n  - 可以截图、点击、输入\n\n## 接下来可以\n\n1. 输入 `@` 引用一台设备，让我在上面执行命令\n2. 点 **+** 引用设备上的文件，让我帮你读日志\n\n比如清理旧备份可以这样做：\n\n```bash\ndu -sh /data/* | sort -rh | head\nfind /data/backups -name '*.tar.gz' -mtime +30 -delete\n```\n\n权限说明见 [项目主页](https://github.com/LemonITCN/lemon-galaxy)。",
            $"Your account has **{devices.Length} devices**, {online} of them online.\n\n## Online devices\n\n- **build-server-01**: Ubuntu 24.04, build server\n- **gpu-node-02**: Debian 12\n- **OFFICE-PC-07**: Windows 11\n  - I can take screenshots, click and type\n\n## What next\n\n1. Type `@` to reference a device and let me run commands on it\n2. Click **+** to attach a file from a device, e.g. a log\n\nCleaning up old backups would look like this:\n\n```bash\ndu -sh /data/* | sort -rh | head\nfind /data/backups -name '*.tar.gz' -mtime +30 -delete\n```\n\nSee the [project page](https://github.com/LemonITCN/lemon-galaxy) for details on permissions."), ct);
    }

    /// <summary>模型要用一台没引用过的设备：除完全信任外先请用户同意（只有允许 / 拒绝）</summary>
    private async Task<bool> UseTarget(CancellationToken ct)
    {
        var id = Target.Id;
        bool known, full;
        lock (lk) { known = authorized.Contains(id); full = mode == AiMode.FullTrust; }
        var needs = !known && !full;
        var callId = "call-" + ShortId() + "-use";
        var call = new ToolCallView(callId, "use_target", id, TargetName, Z($"连接 {TargetName}", $"Connect to {TargetName}"), "", RiskLevel.Read,
            needs ? new[] { Z("对话里没有引用这台设备", "This device was not referenced in the conversation") } : Array.Empty<string>(),
            Z("需要在这台设备上查看磁盘占用", "Need to check disk usage on this device"), null);
        Emit(new AgentEvent.ToolProposed(call, needs));
        if (needs)
        {
            var tcs = new TaskCompletionSource<ApprovalDecision>(TaskCreationOptions.RunContinuationsAsynchronously);
            lock (lk) approvals[callId] = tcs;
            if (await tcs.Task is ApprovalDecision.Deny)
            {
                var r = new ToolResultView(false, Z("用户拒绝", "Denied by user"), "", null, true);
                Emit(new AgentEvent.ToolFinished(callId, r));
                Record(new AgentMessage.Tool(call, r));
                await Say(Z($"好的，不连接 {TargetName}。", $"OK, I will not connect to {TargetName}."), ct);
                return false;
            }
        }
        ct.ThrowIfCancellationRequested();
        lock (lk) authorized.Add(id);
        Emit(new AgentEvent.ToolStarted(callId));
        await Pause(200);
        var ok = new ToolResultView(true, Z("已连接", "Connected"), "", null, false);
        Emit(new AgentEvent.ToolFinished(callId, ok));
        Record(new AgentMessage.Tool(call, ok));
        return true;
    }

    private async Task ShellScript(CancellationToken ct)
    {
        var build = client.Device("dev-build")!;
        await Say(Z($"好的，我先看一下 {TargetName} 的磁盘使用情况。", $"OK. Let me check the disk usage on {TargetName} first."), ct);
        var df = MockShell.Run("df -h", build, "~", Zh).text;
        await Tool(ct, "run_command", "df -h", RiskLevel.Read, new[] { Z("只读命令", "Read-only command") }, Z("查看各分区使用率", "See usage of each partition"), df, Z("退出码 0", "Exit code 0"));
        await Say(Z("/data 分区已用 92%。再看看是哪些目录占得多。", "/data is 92% full. Let me see which directories use the most space."), ct);
        var du = MockShell.Run("du", build, "~", Zh).text;
        await Tool(ct, "run_command", "du -sh /data/* | sort -rh | head", RiskLevel.Read, new[] { Z("只读命令", "Read-only command") }, Z("找出占用最大的目录", "Find the largest directories"), du, Z("退出码 0", "Exit code 0"));
        await Say(Z("/data/backups 占了 1.1T，里面有不少 30 天前的旧备份。建议删除 30 天前的备份文件：", "/data/backups uses 1.1T and holds many backups older than 30 days. I suggest deleting them:"), ct);
        var res = await Tool(ct, "run_command", "find /data/backups -name '*.tar.gz' -mtime +30 -delete", RiskLevel.Dangerous,
            Zh ? new[] { "find -delete 会删除文件", "作用于 /data 下的大量文件" } : new[] { "find -delete removes files", "Affects many files under /data" },
            Z("清理 30 天前的旧备份以释放空间", "Remove backups older than 30 days to free space"), Z("已删除 23 个文件\n", "23 files removed\n"), Z("释放 412 GB", "Freed 412 GB"));
        if (res == null)
        {
            await Say(Z("好的，不删除。你也可以先把旧备份挪到别的盘，或者告诉我只保留最近几份。", "OK, nothing was deleted. You could move old backups elsewhere, or tell me how many recent ones to keep."), ct);
            return;
        }
        await Say(Z("清理完成，/data 现在使用 70%。", "Done. /data is now 70% full."), ct);
        var a = await Ask(Z("要不要顺便设置每天自动清理 30 天前的备份？", "Should I also schedule a daily cleanup of backups older than 30 days?"),
            Zh ? new[] { "设置", "不用了" } : new[] { "Schedule it", "No thanks" });
        ct.ThrowIfCancellationRequested();
        if (a == Z("设置", "Schedule it"))
        {
            if (await Secret(Z("写入 root 的定时任务需要 deploy 的 sudo 密码", "Writing root's crontab needs the sudo password for deploy")) == null)
            {
                await Say(Z("没有拿到密码，先不设置了。", "No password provided, so I did not schedule it."), ct);
                return;
            }
            await Tool(ct, "run_command", "sudo crontab -l | { cat; echo '0 3 * * * find /data/backups -mtime +30 -delete'; } | sudo crontab -", RiskLevel.Dangerous,
                Zh ? new[] { "修改 crontab", "使用 sudo" } : new[] { "Modifies crontab", "Uses sudo" }, Z("每天 3 点自动清理", "Clean up every day at 03:00"), "", Z("退出码 0", "Exit code 0"));
            await Say(Z("已设置，每天凌晨 3 点执行。", "Scheduled for 03:00 every day."), ct);
        }
        else await Say(Z("好的，不设置。", "OK, not scheduling it."), ct);
    }

    private async Task GuiScript(CancellationToken ct)
    {
        var d = client.Device(Target.Id);
        if (d == null) return;
        var display = MockData.Displays(d)[0];
        var renderer = new MockDesktopRenderer(d, Zh, 1280, (int)(1280.0 * display.Height / display.Width));
        await Say(Z($"我先截个图看看 {TargetName} 现在的屏幕。", $"Let me take a screenshot of {TargetName} first."), ct);
        var shot = renderer.Png(1280);
        await Tool(ct, "screenshot", Z("截取屏幕 1", "Capture display 1"), RiskLevel.Read, new[] { Z("只读操作", "Read-only") }, Z("了解当前屏幕内容", "See what is on screen"), "",
            $"1280×{shot.Height}", resultImage: shot);
        await Say(Z("记事本里有未保存的内容。我点一下保存。", "Notepad has unsaved text. I will click Save."), ct);
        var p = renderer.SaveButtonPoint;
        var marked = renderer.Png(1280, p);
        await Tool(ct, "mouse", Z($"单击 ({(int)p.X}, {(int)p.Y})「保存」", $"Click ({(int)p.X}, {(int)p.Y}) \"Save\""), RiskLevel.Normal, new[] { Z("图形界面点击", "GUI click") },
            Z("保存记事本内容", "Save the Notepad content"), "", Z("已点击", "Clicked"), image: marked);
        await Say(Z("已保存。", "Saved."), ct);
    }
}
