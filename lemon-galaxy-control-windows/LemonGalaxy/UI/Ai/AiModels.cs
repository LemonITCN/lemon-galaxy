using System.IO;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

/// <summary>AI 分区的总状态：可用模型、新建对话草稿、已打开的对话、本机保存的历史</summary>
internal sealed class AiHub
{
    private readonly AppModel app;
    public readonly Observable Changed = new();
    public AiModels? Models;
    public bool LoadingModels;
    public string? ModelsError;
    public List<AgentTarget> DraftTargets = new();
    /// <summary>"provider_id:model_id"</summary>
    public string DraftModel = "";
    public AiMode DraftMode = AiMode.ConfirmDangerous;
    public int DraftTrustMinutes;
    public bool DraftPlanFirst;
    public string DraftPrompt = "";
    public readonly List<AiConversationTab> Conversations = new();
    public ConversationSummary[] Summaries = Array.Empty<ConversationSummary>();

    public AiHub(AppModel app)
    {
        this.app = app;
        DraftMode = app.Settings.AiMode;
        DraftPlanFirst = app.Settings.AiPlanFirst;
    }

    public void LoadModels()
    {
        if (Models != null || LoadingModels || !app.LoggedIn) return;
        LoadingModels = true;
        DraftMode = app.Settings.AiMode;
        DraftPlanFirst = app.Settings.AiPlanFirst;
        Changed.Notify();
        Ui.Run(async () =>
        {
            try
            {
                var m = await app.Client.AiModels();
                Models = m;
                ModelsError = null;
                if (DraftModel.Length == 0)
                {
                    var last = app.Settings.AiLastModel;
                    DraftModel = m.Items.Any(i => Key(i) == last) ? last : m.Items.Select(Key).FirstOrDefault() ?? "";
                }
                if (DraftMode.Rank() > m.MaxMode.Rank()) DraftMode = m.MaxMode;
            }
            catch (Exception e) { ModelsError = Fmt.Error(e); }
            LoadingModels = false;
            Changed.Notify();
        });
    }

    public static string Key(AiModel m) => $"{m.ProviderId}:{m.Id}";
    public AiModel? SelectedModel => Models?.Items.FirstOrDefault(m => Key(m) == DraftModel);

    public void ReloadSummaries()
    {
        var open = new HashSet<string>(Conversations.Select(c => c.ConversationId).Where(x => x != null).Select(x => x!));
        try { Summaries = app.Client.ListConversations().Where(s => !open.Contains(s.Id)).ToArray(); }
        catch { Summaries = Array.Empty<ConversationSummary>(); }
        Changed.Notify();
    }

    public string TargetName(AgentTarget t) => t.Kind == AgentTargetKind.Device ? app.DeviceName(t.Id) : app.SshLabel(t.Id);

    public bool UsesDevice(string deviceId) => Conversations.Any(c => c.Config.Targets.Any(t => t.Kind == AgentTargetKind.Device && t.Id == deviceId));

    public void ToggleTarget(AgentTarget t)
    {
        var i = DraftTargets.FindIndex(x => x == t);
        if (i >= 0) DraftTargets.RemoveAt(i);
        else DraftTargets.Add(t);
        Changed.Notify();
    }

    public bool CanStart => SelectedModel != null && DraftTargets.Count > 0 && DraftPrompt.Trim().Length > 0;

    /// <summary>新建对话（内核会先连上所有目标设备）并发出第一条消息</summary>
    public AiConversationTab? Start()
    {
        var m = SelectedModel;
        if (m == null || !CanStart) return null;
        app.Settings.AiLastModel = DraftModel;
        uint? trust = DraftMode == AiMode.FullTrust && DraftTrustMinutes > 0 ? (uint)DraftTrustMinutes : null;
        var prompt = DraftPrompt.Trim();
        var config = new ConversationConfig(m.ProviderId, m.Id, DraftTargets.ToArray(), DraftMode, DraftPlanFirst, trust, prompt.Length > 40 ? prompt.Substring(0, 40) : prompt);
        var tab = new AiConversationTab(app, config, m) { TrustMinutes = DraftTrustMinutes };
        Conversations.Insert(0, tab);
        app.AddTab(tab);
        DraftPrompt = "";
        Changed.Notify();
        tab.Create(prompt);
        return tab;
    }

    /// <summary>重新打开本机保存的对话：优先用创建时保存的配置；老记录没有配置时按目标名称找回设备 id</summary>
    public void Reopen(ConversationSummary s)
    {
        var saved = s.Config;
        var model = Models?.Items.FirstOrDefault(x => x.Id == (saved?.ModelId ?? s.ModelId) && (saved == null || x.ProviderId == saved.ProviderId))
                    ?? new AiModel(saved?.ProviderId ?? 0, "", s.ModelId, s.ModelId, false, true, 0);
        ConversationConfig config;
        if (saved != null) config = saved with { Title = s.Title };
        else
        {
            var targets = s.TargetNames.Select(n => app.Devices.FirstOrDefault(d => d.Hostname == n || app.DeviceName(d.Id) == n))
                .Where(d => d != null).Select(d => new AgentTarget(AgentTargetKind.Device, d!.Id)).ToArray();
            config = new ConversationConfig(model.ProviderId, model.Id, targets, app.Settings.AiMode, false, null, s.Title);
        }
        var tab = new AiConversationTab(app, config, model) { SavedTargetNames = s.TargetNames };
        tab.Title = s.Title;
        Conversations.Insert(0, tab);
        app.AddTab(tab);
        tab.Open(s.Id);
        ReloadSummaries();
    }

    public void Delete(ConversationSummary s)
    {
        try { app.Client.DeleteConversation(s.Id); } catch (Exception e) { app.ShowError(e); }
        ReloadSummaries();
    }

    public void Focus(AiConversationTab c)
    {
        if (!app.Tabs.Contains(c)) app.Tabs.Add(c);
        app.ActivateTab(c.Id);
    }
}

// ---------- 对话里的条目 ----------

internal abstract class ChatItem : Observable
{
    public string Id = Guid.NewGuid().ToString("N");
}

internal sealed class ChatUser : ChatItem { public string Text = ""; }
internal sealed class ChatError : ChatItem { public string Text = ""; }

/// <summary>助手回复或思考过程（流式累积）</summary>
internal sealed class ChatText : ChatItem
{
    public string Text = "";
    public bool Thinking;
    public bool Done;
    public bool Expanded;
}

internal enum ToolDecision { Auto, Pending, Approved, ApprovedSimilar, Denied, Edited }
internal enum ToolPhase { Proposed, Running, Finished }

internal sealed class ToolCard : ChatItem
{
    public readonly ToolCallView Call;
    public ToolDecision Decision;
    public string? DenyReasonSent;
    public string? EditedCommand;
    public ToolPhase Phase = ToolPhase.Proposed;
    public string Output = "";
    public ToolResultView? Result;
    public bool Expanded, Editing, Denying;
    public string EditText, DenyReason = "";

    public ToolCard(ToolCallView call, bool needsApproval)
    {
        Call = call;
        Id = call.CallId;
        Decision = needsApproval ? ToolDecision.Pending : ToolDecision.Auto;
        EditText = call.Summary;
    }
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

/// <summary>一个 AI 对话标签。内核对象要异步创建（先连上目标设备），创建完成前发出的消息先排队。</summary>
internal sealed class AiConversationTab : TabModel
{
    public ICoreConversation? Conversation;
    public readonly ConversationConfig Config;
    public readonly AiModel Model;
    private readonly ListenerAdapter listener;
    public bool Preparing;
    public readonly List<ChatItem> Items = new();
    private readonly Dictionary<string, ChatText> texts = new();
    public bool Running;
    public uint Step;
    public ulong PromptTokens, CompletionTokens;
    public AiMode Mode;
    public AiMode? EffectiveMode;
    public DateTime? TrustUntil;
    public int TrustMinutes;
    public string Composer = "";
    public SecretItem? ActiveSecret;
    public ImageData? ZoomImage;
    public string[]? SavedTargetNames;
    private readonly List<string> pending = new();
    /// <summary>条目列表增减（不含条目内部变化）</summary>
    public readonly Observable ItemsChanged = new();

    public AiConversationTab(AppModel app, ConversationConfig config, AiModel model)
        : base(app, TabKind.Ai, string.IsNullOrEmpty(config.Title) ? L.T("ai.newConversation") : config.Title!)
    {
        Config = config;
        Model = model;
        Mode = config.Mode;
        listener = new ListenerAdapter(this);
    }

    public string? ConversationId
    {
        get
        {
            try { return Conversation?.Id(); }
            catch { return null; }
        }
    }

    public string[] TargetNames => SavedTargetNames ?? Config.Targets.Select(t => App.Ai.TargetName(t)).ToArray();

    public bool HasPendingApproval => Items.OfType<ToolCard>().Any(c => c.Decision == ToolDecision.Pending);

    public void Create(string firstMessage)
    {
        AppendUser(firstMessage);
        pending.Add(firstMessage);
        Attach(c => c.NewConversation(Config, listener));
    }

    public void Open(string savedId) => Attach(c => c.OpenConversation(savedId, Config, listener));

    private void Attach(Func<ICoreClient, Task<ICoreConversation>> make)
    {
        Preparing = true;
        Running = true;
        Live = true;
        Notify();
        Ui.Run(async () =>
        {
            try
            {
                var conv = await make(App.Client);
                Conversation = conv;
                Preparing = false;
                if (Items.Count == 0) RestoreHistory();
                var t = conv.Title();
                if (t.Length > 0) Title = t;
                var queued = pending.ToList();
                pending.Clear();
                if (queued.Count == 0) Running = false;
                foreach (var text in queued) conv.Send(text);
                Notify();
            }
            catch (Exception e)
            {
                Preparing = false;
                Running = false;
                Live = false;
                Add(new ChatError { Text = Fmt.Error(e) });
                Notify();
            }
        });
    }

    private void Add(ChatItem item)
    {
        Items.Add(item);
        ItemsChanged.Notify();
    }

    private void AppendUser(string t)
    {
        if (Title == L.T("ai.newConversation")) Title = t.Length > 24 ? t.Substring(0, 24) : t;
        Add(new ChatUser { Text = t });
    }

    public void Send(string text)
    {
        var t = text.Trim();
        if (t.Length == 0) return;
        AppendUser(t);
        Running = true;
        Live = true;
        Notify();
        if (Conversation != null) Conversation.Send(t);
        else pending.Add(t);
    }

    public void Stop()
    {
        try { Conversation?.Stop(); } catch { }
        Running = false;
        foreach (var c in Items.OfType<ToolCard>().Where(c => c.Decision == ToolDecision.Pending))
        {
            c.Decision = ToolDecision.Denied;
            c.Notify();
        }
        Notify();
    }

    public void ChangeMode(AiMode m, int minutes)
    {
        Mode = m;
        TrustMinutes = minutes;
        try { Conversation?.SetMode(m, m == AiMode.FullTrust && minutes > 0 ? (uint)minutes : null); } catch (Exception e) { App.ShowError(e); }
        Notify();
    }

    public void Decide(ToolCard card, ApprovalDecision d)
    {
        try { Conversation?.Decide(card.Call.CallId, d); } catch (Exception e) { App.ShowError(e); }
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
        try { Conversation?.Answer(ask.Id, text); } catch (Exception e) { App.ShowError(e); }
    }

    public void ProvideSecret(SecretItem req, string secret)
    {
        // 密码只交给内核送到被控端，不留在界面状态里，也不进对话历史
        try { Conversation?.ProvideSecret(req.Id, secret); } catch (Exception e) { App.ShowError(e); }
        req.Done = true;
        req.Notify();
        ActiveSecret = null;
        Notify();
    }

    public void CancelSecret(SecretItem req)
    {
        try { Conversation?.ProvideSecret(req.Id, ""); } catch { }
        req.Cancelled = true;
        req.Notify();
        ActiveSecret = null;
        Notify();
    }

    public void RestoreHistory()
    {
        if (Conversation == null) return;
        foreach (var m in Conversation.Messages())
        {
            switch (m)
            {
                case AgentMessage.User u:
                    Items.Add(new ChatUser { Text = u.Text });
                    break;
                case AgentMessage.Assistant a:
                    if (a.Thinking.Length > 0) Items.Add(new ChatText { Id = a.Id + "-thinking", Text = a.Thinking, Thinking = true, Done = true });
                    var t = new ChatText { Id = a.Id, Text = a.Text, Done = true };
                    texts[a.Id] = t;
                    Items.Add(t);
                    break;
                case AgentMessage.Tool tool:
                    Items.Add(new ToolCard(tool.Call, false) { Result = tool.Result, Output = tool.Result?.Output ?? "", Phase = ToolPhase.Finished, Decision = tool.Result?.Denied == true ? ToolDecision.Denied : ToolDecision.Auto });
                    break;
            }
        }
        ItemsChanged.Notify();
    }

    private ToolCard? Card(string id) => Items.OfType<ToolCard>().FirstOrDefault(c => c.Call.CallId == id);

    private ChatText TextItem(string id, bool thinking)
    {
        if (texts.TryGetValue(id, out var t)) return t;
        t = new ChatText { Id = id, Thinking = thinking };
        texts[id] = t;
        Add(t);
        return t;
    }

    public void Handle(AgentEvent e)
    {
        switch (e)
        {
            case AgentEvent.AssistantDelta d:
            {
                var t = TextItem(d.MessageId, false);
                t.Text += d.Text;
                t.Notify();
                break;
            }
            case AgentEvent.AssistantDone d:
                if (texts.TryGetValue(d.MessageId, out var done)) { done.Done = true; done.Notify(); }
                break;
            case AgentEvent.Thinking th:
            {
                var t = TextItem(th.MessageId, true);
                t.Text += th.Text;
                t.Notify();
                break;
            }
            case AgentEvent.ToolProposed p:
                Add(new ToolCard(p.Call, p.NeedsApproval));
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
                    if (f.Result.Denied && c3.Decision == ToolDecision.Auto) c3.Decision = ToolDecision.Denied;
                    if (f.Result.Output.Length > c3.Output.Length) c3.Output = f.Result.Output;
                    c3.Notify();
                }
                break;
            case AgentEvent.AskUser a:
                Add(new AskItem { Id = a.QuestionId, Question = a.Question, Options = a.Options });
                break;
            case AgentEvent.SecretRequested r:
                var req = new SecretItem { Id = r.RequestId, Prompt = r.Prompt, TargetId = r.TargetId };
                Add(req);
                ActiveSecret = req;
                Notify();
                break;
            case AgentEvent.Usage u:
                PromptTokens = u.PromptTokens;
                CompletionTokens = u.CompletionTokens;
                Notify();
                break;
            case AgentEvent.Status st:
                Running = st.Running;
                Step = st.Step;
                Notify();
                break;
            case AgentEvent.ModeChanged mc:
                Mode = mc.Mode;
                EffectiveMode = mc.Effective;
                TrustUntil = mc.UntilMs is { } ms ? Fmt.Local(ms) : null;
                Notify();
                break;
            case AgentEvent.Error err:
                Add(new ChatError { Text = err.Message });
                break;
        }
    }

    public override void Close()
    {
        if (Running) { try { Conversation?.Stop(); } catch { } }
        App.Ai.Conversations.Remove(this);
        App.Ai.ReloadSummaries();
    }

    protected override FrameworkElement CreateView() => AiViews.Conversation(App, this);

    private sealed class ListenerAdapter : AgentListener
    {
        private readonly WeakReference<AiConversationTab> owner;
        public ListenerAdapter(AiConversationTab t) { owner = new WeakReference<AiConversationTab>(t); }
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
