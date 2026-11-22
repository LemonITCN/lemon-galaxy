using System.IO;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

/// <summary>AI 页面能不能用（docs/ai-chat-ui.md 第 5 节）</summary>
internal enum AiAvailability { Loading, Failed, Disabled, NoModels, Ready }

/// <summary>输入框的草稿：文字与引用。每个对话一份，新对话一份；切换对话时各自保留。</summary>
internal sealed class ComposerDraft : Observable
{
    public string Text = "";
    public readonly List<ContextRef> Refs = new();
    /// <summary>新建对话失败的原因，显示在输入框上方</summary>
    public string? Error;
    /// <summary>新对话正在创建（发送按钮转圈、不能再发）</summary>
    public bool Sending;

    public bool IsEmpty => Text.Trim().Length == 0 && Refs.Count == 0;

    public void AddRef(ContextRef r)
    {
        if (Refs.Any(x => x.Kind == r.Kind && x.TargetId == r.TargetId && x.Id == r.Id)) return;
        Refs.Add(r);
        Notify();
    }

    public void RemoveRef(ContextRef r)
    {
        Refs.Remove(r);
        Notify();
    }
}

/// <summary>对话列表里的一行：本机保存的对话，或本次打开过的对话</summary>
internal sealed record AiRow(string Id, string Title, long UpdatedAt, AiChat? Live);

/// <summary>AI 页面的总状态：可用模型、对话列表、当前对话、新对话的草稿与默认设置</summary>
internal sealed class AiHub
{
    private readonly AppModel app;
    /// <summary>模型列表、对话列表、当前对话变化</summary>
    public readonly Observable Changed = new();
    public AiModels? Models;
    public bool LoadingModels;
    public string? ModelsError;

    /// <summary>本次运行里打开过的对话（内核对象常驻，后台任务继续跑）</summary>
    public readonly List<AiChat> Live = new();
    /// <summary>右侧显示的对话；null 表示新对话</summary>
    public AiChat? Current;
    public readonly ComposerDraft NewDraft = new();
    public ConversationSummary[] Summaries = Array.Empty<ConversationSummary>();
    public string Search = "";

    // 新对话的模型、档位、先出计划（取自本机设置）
    /// <summary>"provider_id:model_id"</summary>
    public string DraftModel = "";
    public AiMode DraftMode = AiMode.ConfirmDangerous;
    /// <summary>完全信任的时限（分钟），0 表示本对话内不限时</summary>
    public int DraftTrustMinutes;
    public bool DraftPlanFirst;

    public AiHub(AppModel app)
    {
        this.app = app;
        ResetDraftSettings();
    }

    public AppModel App => app;

    public AiAvailability Availability
    {
        get
        {
            if (app.User?.AiEnabled == false) return AiAvailability.Disabled;
            if (Models == null) return LoadingModels || ModelsError == null ? AiAvailability.Loading : AiAvailability.Failed;
            if (!Models.Enabled) return AiAvailability.Disabled;
            if (Models.Items.Length == 0) return AiAvailability.NoModels;
            return AiAvailability.Ready;
        }
    }

    // ---------- 模型 ----------

    /// <summary>进入 AI 页面就查一次可用模型；已有结果时后台刷新，不清空界面</summary>
    public void LoadModels()
    {
        if (LoadingModels || !app.LoggedIn) return;
        LoadingModels = true;
        if (Models == null) ModelsError = null;
        Changed.Notify();
        Ui.Run(async () =>
        {
            try
            {
                Models = await app.Client.AiModels();
                ModelsError = null;
                ResolveDraft();
            }
            catch (Exception e)
            {
                ModelsError = Fmt.Error(e);
                // 刷新失败但之前加载成功过：继续用旧结果
            }
            LoadingModels = false;
            Changed.Notify();
            foreach (var c in Live) c.Notify();
        });
    }

    public void Retry()
    {
        Models = null;
        ModelsError = null;
        LoadModels();
    }

    public static string Key(AiModel m) => $"{m.ProviderId}:{m.Id}";
    public static string Key(ulong providerId, string modelId) => $"{providerId}:{modelId}";

    public AiModel? Model(string key) => Models?.Items.FirstOrDefault(m => Key(m) == key);

    public string ModelName(string key) => Model(key)?.Name ?? (key.IndexOf(':') is var i and >= 0 ? key.Substring(i + 1) : key);

    public AiMode MaxMode => Models?.MaxMode ?? AiMode.FullTrust;

    public AiMode Cap(AiMode m) => m.Rank() > MaxMode.Rank() ? MaxMode : m;

    private void ResetDraftSettings()
    {
        var s = app.Settings;
        DraftModel = s.AiLastModel;
        DraftMode = s.AiMode;
        DraftTrustMinutes = s.AiTrustMinutes;
        DraftPlanFirst = s.AiPlanFirst;
        ResolveDraft();
    }

    /// <summary>上次选的模型不可用时取列表第一个；档位不超过管理员上限</summary>
    private void ResolveDraft()
    {
        if (Models == null) return;
        if (Model(DraftModel) == null) DraftModel = Models.Items.Select(Key).FirstOrDefault() ?? "";
        DraftMode = Cap(DraftMode);
    }

    // ---------- 对话列表 ----------

    public void ReloadSummaries()
    {
        try { Summaries = app.Client.ListConversations(); }
        catch { Summaries = Array.Empty<ConversationSummary>(); }
        Changed.Notify();
    }

    public List<AiRow> Rows()
    {
        var rows = new List<AiRow>();
        var seen = new HashSet<string>();
        foreach (var s in Summaries)
        {
            var live = Live.FirstOrDefault(c => c.Id == s.Id);
            seen.Add(s.Id);
            var title = live != null && live.Title.Length > 0 ? live.Title : s.Title;
            rows.Add(new AiRow(s.Id, title, Math.Max(s.UpdatedAt, live?.UpdatedAt ?? 0), live));
        }
        foreach (var c in Live.Where(c => c.Id != null && !seen.Contains(c.Id)))
            rows.Add(new AiRow(c.Id!, c.Title, c.UpdatedAt, c));
        var q = Search.Trim();
        return rows.Where(r => q.Length == 0 || r.Title.IndexOf(q, StringComparison.CurrentCultureIgnoreCase) >= 0)
            .OrderByDescending(r => r.UpdatedAt).ToList();
    }

    public void NewChat()
    {
        Current = null;
        ResetDraftSettings();
        NewDraft.Error = null;
        NewDraft.Notify();
        Changed.Notify();
    }

    /// <summary>从设备、主机页的「AI」进来：新对话，输入框里先放上这些引用</summary>
    public void StartWith(IEnumerable<ContextRef> refs)
    {
        NewChat();
        NewDraft.Refs.Clear();
        foreach (var r in refs) NewDraft.AddRef(r);
        NewDraft.Notify();
    }

    public void Select(string id)
    {
        var live = Live.FirstOrDefault(c => c.Id == id);
        if (live != null)
        {
            Current = live;
            Changed.Notify();
            return;
        }
        var s = Summaries.FirstOrDefault(x => x.Id == id);
        if (s != null) Open(s);
    }

    /// <summary>打开本机保存的对话：用 messages() 恢复消息流，模型与档位取对话保存的配置</summary>
    private void Open(ConversationSummary s)
    {
        var cfg = s.Config;
        var chat = new AiChat(this)
        {
            Id = s.Id, Title = s.Title, UpdatedAt = s.UpdatedAt, Opening = true,
            ModelKey = cfg != null ? Key(cfg.ProviderId, cfg.ModelId) : Models?.Items.FirstOrDefault(m => m.Id == s.ModelId) is { } mm ? Key(mm) : "0:" + s.ModelId,
            Mode = cfg?.Mode ?? Cap(app.Settings.AiMode),
            TrustMinutes = (int)(cfg?.TrustMinutes ?? 0),
            PlanFirst = cfg?.PlanFirst ?? false,
        };
        Live.Add(chat);
        Current = chat;
        Changed.Notify();
        Ui.Run(async () =>
        {
            try
            {
                var conv = await app.Client.OpenConversation(s.Id, null, chat.Listener);
                chat.Attach(conv);
                chat.Restore();
                try
                {
                    var m = conv.Model();
                    if (m.ModelId.Length > 0) chat.ModelKey = Key(m.ProviderId, m.ModelId);
                }
                catch { }
            }
            catch (Exception e)
            {
                Live.Remove(chat);
                chat.AddError(Fmt.Error(e));
            }
            chat.Opening = false;
            chat.Notify();
            Changed.Notify();
        });
    }

    public void Delete(string id)
    {
        var live = Live.FirstOrDefault(c => c.Id == id);
        if (live != null)
        {
            if (live.Running) live.Stop();
            Live.Remove(live);
        }
        try { app.Client.DeleteConversation(id); } catch (Exception e) { app.ShowError(e); }
        if (Current != null && Current.Id == id) NewChat();
        ReloadSummaries();
    }

    // ---------- 发送 ----------

    /// <summary>新对话发第一条消息时才创建；创建失败时消息留在输入框里，上方显示错误</summary>
    public void SendNew()
    {
        var d = NewDraft;
        if (d.IsEmpty || d.Sending || Availability != AiAvailability.Ready) return;
        var m = Model(DraftModel) ?? Models!.Items[0];
        var text = d.Text.Trim();
        var refs = d.Refs.ToArray();
        var mode = Cap(DraftMode);
        uint? trust = mode == AiMode.FullTrust && DraftTrustMinutes > 0 ? (uint)DraftTrustMinutes : null;
        var chat = new AiChat(this) { ModelKey = Key(m), Mode = mode, TrustMinutes = DraftTrustMinutes, PlanFirst = DraftPlanFirst };
        var config = new ConversationConfig(m.ProviderId, m.Id, Array.Empty<AgentTarget>(), mode, DraftPlanFirst, trust, null);
        d.Sending = true;
        d.Error = null;
        d.Notify();
        Ui.Run(async () =>
        {
            try
            {
                var conv = await app.Client.NewConversation(config, chat.Listener);
                chat.Attach(conv);
                d.Text = "";
                d.Refs.Clear();
                d.Sending = false;
                d.Notify();
                Live.Add(chat);
                Current = chat;
                chat.Send(text, refs);
                ReloadSummaries();
            }
            catch (Exception e)
            {
                d.Sending = false;
                d.Error = Fmt.Error(e);
                d.Notify();
            }
        });
    }

    public void Send(AiChat? chat)
    {
        if (chat == null) { SendNew(); return; }
        var d = chat.Draft;
        if (d.IsEmpty || chat.Running || chat.Conversation == null) return;
        var text = d.Text.Trim();
        var refs = d.Refs.ToArray();
        d.Text = "";
        d.Refs.Clear();
        d.Error = null;
        d.Notify();
        chat.Send(text, refs);
        Changed.Notify();
    }

    // ---------- 模型 / 档位 / 先出计划 ----------

    public void SetModel(AiChat? chat, AiModel m)
    {
        if (chat?.Conversation != null)
        {
            try { chat.Conversation.SetModel(m.ProviderId, m.Id); }
            catch (Exception e) { app.ShowError(e); return; }
        }
        if (chat != null) { chat.ModelKey = Key(m); chat.Notify(); }
        else DraftModel = Key(m);
        app.Settings.AiLastModel = Key(m);
        Changed.Notify();
    }

    public void SetMode(AiChat? chat, AiMode mode, int trustMinutes)
    {
        mode = Cap(mode);
        if (chat != null)
        {
            chat.Mode = mode;
            chat.TrustMinutes = trustMinutes;
            try { chat.Conversation?.SetMode(mode, mode == AiMode.FullTrust && trustMinutes > 0 ? (uint)trustMinutes : null); }
            catch (Exception e) { app.ShowError(e); }
            chat.Notify();
        }
        else
        {
            DraftMode = mode;
            DraftTrustMinutes = trustMinutes;
        }
        app.Settings.AiMode = mode;
        app.Settings.AiTrustMinutes = trustMinutes;
        Changed.Notify();
    }

    public void SetPlanFirst(bool on)
    {
        DraftPlanFirst = on;
        app.Settings.AiPlanFirst = on;
        Changed.Notify();
    }

    // ---------- 引用 ----------

    /// <summary>@ 与「设备或主机」可选的：账号里的设备（不含本机与手机）与 SSH 主机，在线的在前</summary>
    public List<ContextRef> TargetCandidates(string filter)
    {
        var q = filter.Trim();
        bool Hit(string s) => q.Length == 0 || s.IndexOf(q, StringComparison.CurrentCultureIgnoreCase) >= 0;
        var list = new List<ContextRef>();
        foreach (var d in app.Devices.Where(d => !d.IsSelf && d.Kind != DeviceKind.Mobile).OrderByDescending(d => d.Online || d.LanSeen))
        {
            var name = app.DeviceName(d.Id);
            if (Hit(name) || Hit(d.Hostname)) list.Add(new ContextRef(ContextRefKind.Device, "", d.Id, name));
        }
        foreach (var e in app.SshHosts)
        {
            var name = app.SshLabel(e.Id);
            var host = e.Data is VaultData.SshHost h ? h.Host : "";
            if (Hit(name) || Hit(host)) list.Add(new ContextRef(ContextRefKind.SshHost, "", e.Id, name));
        }
        return list;
    }

    public static ContextRef DeviceRef(AppModel app, string id) => new(ContextRefKind.Device, "", id, app.DeviceName(id));
    public static ContextRef SshRef(AppModel app, string id) => new(ContextRefKind.SshHost, "", id, app.SshLabel(id));

    /// <summary>文件 / 文件夹引用所在的设备或主机名</summary>
    public string WhereName(string targetId) => app.Device(targetId) != null ? app.DeviceName(targetId) : app.SshLabel(targetId);
}

// ---------- 对话里的条目 ----------

internal abstract class ChatItem : Observable
{
    public string Id = Guid.NewGuid().ToString("N");
}

internal sealed class UserItem : ChatItem
{
    public string Text = "";
    public ContextRef[] Refs = Array.Empty<ContextRef>();
}

internal sealed class ErrorItem : ChatItem { public string Text = ""; }

/// <summary>模型回复（流式累积，Markdown 渲染）</summary>
internal sealed class ReplyItem : ChatItem
{
    public string Text = "";
    public bool Done;
}

/// <summary>思考过程：默认折叠，进行中时显示「正在思考」</summary>
internal sealed class ThinkingItem : ChatItem
{
    public string Text = "";
    public bool Active;
    public bool Expanded;
}

internal enum ToolDecision { Auto, Pending, Approved, ApprovedSimilar, Denied, Edited }
internal enum ToolPhase { Proposed, Running, Finished }

internal sealed class ToolItem : ChatItem
{
    public readonly ToolCallView Call;
    public ToolDecision Decision;
    public string? DenyReasonSent;
    public string? EditedCommand;
    public ToolPhase Phase = ToolPhase.Proposed;
    public string Output = "";
    public ToolResultView? Result;
    public bool Expanded, OutputExpanded, Editing, Denying;
    public string EditText, DenyReason = "";

    public ToolItem(ToolCallView call, bool needsApproval)
    {
        Call = call;
        Id = call.CallId;
        Decision = needsApproval ? ToolDecision.Pending : ToolDecision.Auto;
        EditText = call.Detail.Length > 0 ? call.Detail : call.Summary;
    }

    public bool IsUseTarget => Call.Tool == "use_target";
    public bool Failed => Result is { Ok: false } || Decision == ToolDecision.Denied;
}

internal sealed class AskItem : ChatItem
{
    public string Question = "";
    public string[] Options = Array.Empty<string>();
    public string? Answer;
    public string FreeText = "";
}

internal sealed class SecretItem : ChatItem
{
    public string Prompt = "", TargetId = "";
    public bool Done, Cancelled;
}

/// <summary>一个对话：内核对象、消息流、运行状态、模型与档位、输入框草稿</summary>
internal sealed class AiChat : Observable
{
    private readonly AiHub hub;
    public string? Id;
    public ICoreConversation? Conversation;
    public string Title = "";
    public long UpdatedAt = Clock.NowMs;
    public bool Opening;
    public readonly List<ChatItem> Items = new();
    /// <summary>条目增加（不含条目内部变化）</summary>
    public readonly Observable ItemsChanged = new();
    public bool Running;
    public uint Step;
    public ulong PromptTokens, CompletionTokens;
    public string ModelKey = "";
    public AiMode Mode = AiMode.ConfirmDangerous;
    public int TrustMinutes;
    public bool PlanFirst;
    public AiMode? EffectiveMode;
    public DateTime? TrustUntil;
    public readonly ComposerDraft Draft = new();
    public ImageData? ZoomImage;
    public readonly AgentListener Listener;
    private readonly Dictionary<string, ReplyItem> replies = new();
    private readonly Dictionary<string, ThinkingItem> thoughts = new();

    public AiChat(AiHub hub)
    {
        this.hub = hub;
        Listener = new ListenerAdapter(this);
    }

    public bool HasPendingApproval => Items.OfType<ToolItem>().Any(c => c.Decision == ToolDecision.Pending);

    public void Attach(ICoreConversation conv)
    {
        Conversation = conv;
        try { Id = conv.Id(); } catch { }
        try
        {
            var t = conv.Title();
            if (t.Length > 0) Title = t;
        }
        catch { }
    }

    private void Add(ChatItem item)
    {
        Items.Add(item);
        ItemsChanged.Notify();
    }

    public void AddError(string text) => Add(new ErrorItem { Text = text });

    public void Send(string text, ContextRef[] refs)
    {
        Add(new UserItem { Text = text, Refs = refs });
        Running = true;
        UpdatedAt = Clock.NowMs;
        try { Conversation?.SendWithRefs(text, refs); }
        catch (Exception e)
        {
            Running = false;
            AddError(Fmt.Error(e));
        }
        if (Title.Length == 0)
        {
            try { Title = Conversation?.Title() ?? ""; } catch { }
            if (Title.Length == 0) Title = text.Length > 40 ? text.Substring(0, 40) : text.Length > 0 ? text : refs.FirstOrDefault()?.Label ?? "";
        }
        Notify();
    }

    public void Stop()
    {
        try { Conversation?.Stop(); } catch { }
        Running = false;
        foreach (var c in Items.OfType<ToolItem>().Where(c => c.Decision == ToolDecision.Pending))
        {
            c.Decision = ToolDecision.Denied;
            c.Notify();
        }
        EndThinking();
        Notify();
    }

    public void Decide(ToolItem card, ApprovalDecision d)
    {
        try { Conversation?.Decide(card.Call.CallId, d); } catch (Exception e) { hub.App.ShowError(e); }
        switch (d)
        {
            case ApprovalDecision.AllowOnce: card.Decision = ToolDecision.Approved; break;
            case ApprovalDecision.AllowSimilar: card.Decision = ToolDecision.ApprovedSimilar; break;
            case ApprovalDecision.Deny deny: card.Decision = ToolDecision.Denied; card.DenyReasonSent = deny.Reason; break;
            case ApprovalDecision.EditAndRun e: card.Decision = ToolDecision.Edited; card.EditedCommand = e.Command; break;
        }
        card.Editing = card.Denying = false;
        card.Notify();
        Notify();
    }

    public void Answer(AskItem ask, string text)
    {
        ask.Answer = text;
        ask.Notify();
        try { Conversation?.Answer(ask.Id, text); } catch (Exception e) { hub.App.ShowError(e); }
    }

    public void ProvideSecret(SecretItem req, string secret)
    {
        // 密码只交给内核送到被控端，不留在界面状态里，也不进对话历史
        try { Conversation?.ProvideSecret(req.Id, secret); } catch (Exception e) { hub.App.ShowError(e); }
        req.Done = true;
        req.Notify();
    }

    public void CancelSecret(SecretItem req)
    {
        try { Conversation?.ProvideSecret(req.Id, ""); } catch { }
        req.Cancelled = true;
        req.Notify();
    }

    /// <summary>打开历史对话：用 messages() 恢复消息流</summary>
    public void Restore()
    {
        if (Conversation == null) return;
        foreach (var m in Conversation.Messages())
        {
            switch (m)
            {
                case AgentMessage.User u:
                    Items.Add(new UserItem { Text = u.Text, Refs = u.Refs });
                    break;
                case AgentMessage.Assistant a:
                    if (a.Thinking.Length > 0) Items.Add(new ThinkingItem { Id = a.Id + "#thinking", Text = a.Thinking });
                    if (a.Text.Length > 0) Items.Add(new ReplyItem { Id = a.Id, Text = a.Text, Done = true });
                    break;
                case AgentMessage.Tool tool:
                    Items.Add(new ToolItem(tool.Call, false)
                    {
                        Result = tool.Result, Output = tool.Result?.Output ?? "", Phase = ToolPhase.Finished,
                        Decision = tool.Result?.Denied == true ? ToolDecision.Denied : ToolDecision.Auto,
                    });
                    break;
            }
        }
        ItemsChanged.Notify();
    }

    private ToolItem? Card(string id) => Items.OfType<ToolItem>().FirstOrDefault(c => c.Call.CallId == id);

    private void EndThinking()
    {
        foreach (var t in thoughts.Values.Where(t => t.Active))
        {
            t.Active = false;
            t.Notify();
        }
    }

    private ReplyItem Reply(string id)
    {
        if (replies.TryGetValue(id, out var r)) return r;
        EndThinking();
        r = new ReplyItem { Id = id };
        replies[id] = r;
        Add(r);
        return r;
    }

    public void Handle(AgentEvent e)
    {
        switch (e)
        {
            case AgentEvent.AssistantDelta d:
            {
                var r = Reply(d.MessageId);
                r.Text += d.Text;
                r.Notify();
                break;
            }
            case AgentEvent.AssistantDone d:
                if (replies.TryGetValue(d.MessageId, out var done)) { done.Done = true; done.Notify(); }
                break;
            case AgentEvent.Thinking th:
            {
                if (!thoughts.TryGetValue(th.MessageId, out var t))
                {
                    t = new ThinkingItem { Id = th.MessageId + "#thinking", Active = true };
                    thoughts[th.MessageId] = t;
                    Add(t);
                }
                t.Text += th.Text;
                t.Notify();
                break;
            }
            case AgentEvent.ToolProposed p:
                EndThinking();
                Add(new ToolItem(p.Call, p.NeedsApproval));
                Notify();
                break;
            case AgentEvent.ToolStarted s:
                if (Card(s.CallId) is { } c1) { c1.Phase = ToolPhase.Running; c1.Notify(); }
                break;
            case AgentEvent.ToolOutput o:
                if (Card(o.CallId) is { } c2) { c2.Output += o.Text; Ui.Coalesce(c2, c2.Notify); }
                break;
            case AgentEvent.ToolFinished f:
                if (Card(f.CallId) is { } c3)
                {
                    c3.Result = f.Result;
                    c3.Phase = ToolPhase.Finished;
                    if (f.Result.Denied && c3.Decision is ToolDecision.Auto or ToolDecision.Pending) c3.Decision = ToolDecision.Denied;
                    if (f.Result.Output.Length > c3.Output.Length) c3.Output = f.Result.Output;
                    c3.Notify();
                    Notify();
                }
                break;
            case AgentEvent.AskUser a:
                EndThinking();
                Add(new AskItem { Id = a.QuestionId, Question = a.Question, Options = a.Options });
                break;
            case AgentEvent.SecretRequested r:
                EndThinking();
                Add(new SecretItem { Id = r.RequestId, Prompt = r.Prompt, TargetId = r.TargetId });
                break;
            case AgentEvent.Usage u:
                PromptTokens = u.PromptTokens;
                CompletionTokens = u.CompletionTokens;
                Notify();
                break;
            case AgentEvent.Status st:
                var wasRunning = Running;
                Running = st.Running;
                Step = st.Step;
                if (!st.Running)
                {
                    EndThinking();
                    foreach (var r in replies.Values.Where(r => !r.Done)) { r.Done = true; r.Notify(); }
                    UpdatedAt = Clock.NowMs;
                    try { var t = Conversation?.Title(); if (!string.IsNullOrEmpty(t)) Title = t!; } catch { }
                }
                Notify();
                if (wasRunning && !st.Running) hub.ReloadSummaries();
                break;
            case AgentEvent.ModeChanged mc:
                Mode = mc.Mode;
                EffectiveMode = mc.Effective;
                TrustUntil = mc.UntilMs is { } ms ? Fmt.Local(ms) : null;
                Notify();
                break;
            case AgentEvent.Error err:
                EndThinking();
                Add(new ErrorItem { Text = err.Message });
                break;
        }
    }

    private sealed class ListenerAdapter : AgentListener
    {
        private readonly WeakReference<AiChat> owner;
        public ListenerAdapter(AiChat t) { owner = new WeakReference<AiChat>(t); }
        public void OnEvent(AgentEvent e) => Ui.Post(() => { if (owner.TryGetTarget(out var t)) t.Handle(e); });
    }
}

internal static class Img
{
    /// <summary>内核给的图片（PNG / JPEG 字节）→ WPF 图片</summary>
    public static BitmapSource? From(ImageData d)
    {
        if (d.Data.Length == 0) return null;
        try
        {
            var bi = new BitmapImage();
            bi.BeginInit();
            bi.CacheOption = BitmapCacheOption.OnLoad;
            bi.StreamSource = new MemoryStream(d.Data);
            bi.EndInit();
            bi.Freeze();
            return bi;
        }
        catch { return null; }
    }

    public static Color RiskColor(RiskLevel r) => r switch
    {
        RiskLevel.Read => Th.P.Success,
        RiskLevel.Normal => Th.P.Info,
        RiskLevel.Unknown => Th.P.Warning,
        _ => Th.P.Danger,
    };

    public static string RiskKey(RiskLevel r) => r switch
    {
        RiskLevel.Read => "read",
        RiskLevel.Normal => "normal",
        RiskLevel.Dangerous => "dangerous",
        RiskLevel.Unknown => "unknown",
        _ => "forbidden",
    };

    public static string RiskIcon(RiskLevel r) => r switch
    {
        RiskLevel.Read => "eye",
        RiskLevel.Normal => "pencil",
        RiskLevel.Dangerous => "warning",
        RiskLevel.Unknown => "ask",
        _ => "ban",
    };
}
