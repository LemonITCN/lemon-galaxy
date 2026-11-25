using System.Windows;
using LemonGalaxy.Ipc;
using LemonGalaxy.Setup;
using LemonGalaxy.UI;

namespace LemonGalaxy;

internal enum Page { Overview, Accounts, Assist, Settings, Events, About }

/// <summary>
/// 界面状态：连着的服务、最近一次状态、账号 / 设置 / 日志、当前页、提示条、窗口内弹层。
/// 所有改配置的请求经 <see cref="Call"/>：服务回「要管理员授权」时自动弹 UAC 授权后重试一次。
/// </summary>
internal sealed class AppModel
{
    public static AppModel? Current;

    public readonly AppSettings Settings;
    public readonly ControlClient? Client;
    public readonly bool SelfTest;

    /// <summary>最近一次状态；没连上服务时为 null</summary>
    public Status? Status;
    public bool EverConnected;
    /// <summary>启动后 1.2 秒内还连不上才显示「服务没运行」，免得一闪</summary>
    public bool ConnectGraceOver;
    public ServiceState Service = ServiceState.Unknown;
    public List<Account>? Accounts;
    public WorkerSettings? Worker;
    public List<LogEntry>? Events;
    public Page Page = Page.Overview;
    public string? Toast;
    public bool ToastError;
    private long toastSerial;
    /// <summary>窗口内弹层（后开的在上面）</summary>
    public readonly List<FrameworkElement> Modals = new();
    /// <summary>界面正在做的长动作（安装、启动服务、授权），按钮显示转圈</summary>
    public string? Busy;

    public readonly Observable StatusChanged = new();
    public readonly Observable AccountsChanged = new();
    public readonly Observable WorkerChanged = new();
    public readonly Observable EventsChanged = new();
    public readonly Observable PageChanged = new();
    public readonly Observable ToastChanged = new();
    public readonly Observable ModalChanged = new();

    /// <summary>托盘、协助请求窗口订阅</summary>
    public event Action<Session>? SessionStarted;
    public event Action<long, string, string>? AssistRequested;
    public event Action<long>? AssistRequestClosed;
    public event Action? AllAssistRequestsClosed;

    private (long rev, bool view, int accounts, int relogin)? lastSig;

    public AppModel(AppSettings settings, bool selfTest)
    {
        Settings = settings;
        SelfTest = selfTest;
        Current = this;
        if (selfTest) return;
        Client = new ControlClient();
        Client.Connected += OnConnected;
        Client.Disconnected += OnDisconnected;
        Client.EventReceived += OnEvent;
        Client.Start();
        RefreshService();
        Ui.Every(3000, () => { if (Status == null) RefreshService(); });
        Ui.After(1200, () =>
        {
            ConnectGraceOver = true;
            StatusChanged.Notify();
        });
    }

    public bool Connected => Status != null;
    public Rights Rights => Status?.Rights ?? new Rights(false, false, ManageRight.Denied);

    public void RefreshService()
    {
        Bg.Run(Installer.QueryService).ContinueWith(t =>
        {
            if (t.Result == Service) return;
            Service = t.Result;
            StatusChanged.Notify();
        }, TaskScheduler.FromCurrentSynchronizationContext());
    }

    // ---------------- 连接 ----------------

    private void OnConnected()
    {
        Ui.Run(async () =>
        {
            await Client!.Call(Req.Hello(L.Pack.CoreLocale));
            var r = await Client.Call(Req.Status());
            if (r.Type == "status" && r.J.O("status") is { } s) SetStatus(Ipc.Status.From(s));
        });
    }

    private void OnDisconnected()
    {
        Status = null;
        lastSig = null;
        AllAssistRequestsClosed?.Invoke();
        RefreshService();
        StatusChanged.Notify();
    }

    public void SetStatus(Status s)
    {
        Status = s;
        EverConnected = true;
        Service = ServiceState.Running;
        StatusChanged.Notify();
        // 配置被改过（界面或命令行）、权限、账号数变了：重新读账号与设置
        var sig = (s.ConfigRev, s.Rights.ViewConfig, s.AccountCount, s.NeedsRelogin);
        if (lastSig != sig)
        {
            lastSig = sig;
            if (s.Rights.ViewConfig)
            {
                ReloadAccounts();
                ReloadWorker();
            }
            else
            {
                Accounts = null;
                Worker = null;
                AccountsChanged.Notify();
                WorkerChanged.Notify();
            }
        }
    }

    private void OnEvent(J ev)
    {
        switch (ev.S("type"))
        {
            case "status":
                if (ev.O("status") is { } s) SetStatus(Ipc.Status.From(s));
                break;
            case "session_started":
                if (ev.O("session") is { } ss) SessionStarted?.Invoke(Session.From(ss));
                break;
            case "assist_request":
                AssistRequested?.Invoke(ev.L("request_id"), ev.S("from_user"), ev.S("from_device"));
                break;
            case "assist_request_closed":
                AssistRequestClosed?.Invoke(ev.L("request_id"));
                break;
            case "assist_ended":
                ShowToast(L.T("assist.ended"));
                break;
        }
    }

    // ---------------- 请求 ----------------

    /// <summary>发请求；服务要管理员授权时弹 UAC，授权后重试一次</summary>
    public async Task<Response> Call(Dictionary<string, object?> req)
    {
        if (Client == null) return ControlClient.Gone();
        var r = await Client.Call(req);
        if (r.Ok || r.Code != ErrorCode.ElevationRequired) return r;
        var a = await Authorize();
        if (!a.Ok) return Response.Error(ErrorCode.AuthFailed, a.Cancelled ? L.T("uac.cancelled") : L.T("uac.failed", a.Error ?? ""));
        return await Client.Call(req);
    }

    /// <summary>经 UAC 授权（几分钟内这个连接能直接改配置）</summary>
    public async Task<Elevation.Result> Authorize()
    {
        Busy = "uac";
        StatusChanged.Notify();
        try { return await Elevation.Authorize(Client!); }
        finally
        {
            Busy = null;
            StatusChanged.Notify();
        }
    }

    /// <summary>发请求，出错时显示提示条；成功返回回复，失败返回 null</summary>
    public async Task<Response?> Do(Dictionary<string, object?> req)
    {
        var r = await Call(req);
        if (r.Ok) return r;
        ShowToast(r.Message, true);
        return null;
    }

    public void ReloadAccounts()
    {
        if (Client == null) return;
        Ui.Run(async () =>
        {
            var r = await Client.Call(Req.Accounts());
            if (r.Type == "accounts")
            {
                Accounts = r.J.A("accounts").Select(Account.From).ToList();
                AccountsChanged.Notify();
            }
        });
    }

    public void ReloadWorker()
    {
        if (Client == null) return;
        Ui.Run(async () =>
        {
            var r = await Client.Call(Req.Settings());
            if (r.Type == "settings" && r.J.O("settings") is { } s)
            {
                Worker = WorkerSettings.From(s);
                WorkerChanged.Notify();
            }
        });
    }

    public void ReloadEvents()
    {
        if (Client == null) return;
        Ui.Run(async () =>
        {
            var r = await Client.Call(Req.Events(500));
            if (r.Type == "events")
            {
                Events = r.J.A("events").Select(LogEntry.From).ToList();
                EventsChanged.Notify();
            }
        });
    }

    // ---------------- 界面 ----------------

    public void Go(Page p)
    {
        Page = p;
        PageChanged.Notify();
        if (p == Page.Events) ReloadEvents();
        if (p is Page.Accounts or Page.Settings or Page.Assist && Rights.ViewConfig)
        {
            if (p == Page.Accounts) ReloadAccounts();
            else ReloadWorker();
        }
    }

    public void ShowToast(string text, bool error = false)
    {
        Toast = text;
        ToastError = error;
        var n = ++toastSerial;
        ToastChanged.Notify();
        if (SelfTest) return;
        Ui.After(error ? 8000 : 4000, () =>
        {
            if (toastSerial != n) return;
            Toast = null;
            ToastChanged.Notify();
        });
    }

    public void ShowError(Exception e)
    {
        Log.Error("界面出错：" + e);
        ShowToast(e.Message, true);
    }

    public void ShowModal(FrameworkElement card)
    {
        Modals.Add(card);
        ModalChanged.Notify();
    }

    public void CloseModal(FrameworkElement? card = null)
    {
        if (Modals.Count == 0) return;
        if (card == null) Modals.RemoveAt(Modals.Count - 1);
        else Modals.Remove(card);
        ModalChanged.Notify();
    }

    /// <summary>窗口内确认框</summary>
    public Task<bool> Confirm(string title, string body, string ok, bool destructive = false)
    {
        var tcs = new TaskCompletionSource<bool>();
        FrameworkElement? card = null;
        void Done(bool v)
        {
            CloseModal(card);
            tcs.TrySetResult(v);
        }
        card = Dialogs.ConfirmCard(title, body, ok, destructive, () => Done(true), () => Done(false));
        ShowModal(card);
        return tcs.Task;
    }

    /// <summary>以管理员身份做一件事（安装、启动服务、卸载），期间界面显示转圈</summary>
    public async Task<bool> RunElevated(string busy, params string[] args)
    {
        Busy = busy;
        StatusChanged.Notify();
        try
        {
            var r = await Elevation.Run(args);
            if (r.Cancelled) ShowToast(L.T("uac.cancelled"), true);
            else if (!r.Ok) ShowToast(L.T("setup.failed", r.Error ?? ""), true);
            return r.Ok;
        }
        finally
        {
            Busy = null;
            RefreshService();
            StatusChanged.Notify();
        }
    }
}
