using System.Text;

namespace LemonGalaxy.Core.Mock;

// 模拟内核：假账号、假设备、假加密库。只给开发与 selftest 用。
// 登录规则（便于手动演示各个步骤，与 Mac 端一致）：
//   - 密码为 wrong：账号或密码错误；
//   - 用户名含 totp：先要求两步验证码（任意 6 位数字通过）；
//   - 用户名含 new：登录后必须改密码；
//   - 其余任意用户名密码都能登录。

internal sealed class MockClient : ICoreClient
{
    public const string Version = "0.1.0-mock";
    private readonly SecureStore store;
    private readonly ClientListener listener;
    public readonly bool Zh;
    public readonly MockOptions Options;
    private readonly object lk = new();
    private ClientState state = ClientState.LoggedOut;
    private UserInfo? user;
    private ServerInfo? server;
    private List<DeviceInfo> devices;
    private List<VaultEntry> vault;
    private (string server, string username)? pending;
    private bool totpEnabled;
    private readonly Dictionary<string, MockConversation> saved = new();
    private readonly SerialQueue queue = new();
    private Timer? liveTimer;

    public MockClient(SecureStore store, ClientListener listener, string locale, MockOptions options)
    {
        this.store = store;
        this.listener = listener;
        Zh = locale.StartsWith("zh", StringComparison.OrdinalIgnoreCase);
        Options = options;
        devices = MockData.Devices(Zh).ToList();
        vault = MockData.Vault(Zh);
    }

    public bool IsMock => true;
    public string CoreVersion => Version;

    public Task Delay(int ms)
    {
        var scaled = ms * Options.LatencyScale;
        return scaled <= 0 ? Task.CompletedTask : Task.Delay((int)scaled);
    }

    private string Z(string zh, string en) => Zh ? zh : en;

    private void SetState(ClientState s)
    {
        lock (lk) state = s;
        queue.Post(() => listener.OnState(s));
    }

    public ClientState State() { lock (lk) return state; }
    public UserInfo? CurrentUser() { lock (lk) return user; }
    public ServerInfo? Server() { lock (lk) return server; }
    public string DeviceId() => "dev-self";

    public async Task<LoginResult> Login(string serverAddr, string username, string password, string? totp)
    {
        await Delay(450);
        if (ServerAddress.Parse(serverAddr) == null) throw new GalaxyException.Network(Z("无法连接服务器：地址格式不对", "Cannot reach the server: invalid address"));
        if (password == "wrong") throw new GalaxyException.Auth("invalid_credentials", Z("用户名或密码错误", "Incorrect username or password"));
        if (username.IndexOf("totp", StringComparison.OrdinalIgnoreCase) >= 0 && string.IsNullOrEmpty(totp)) return LoginResult.NeedTotp;
        if (!string.IsNullOrEmpty(totp) && (totp!.Length != 6 || !totp.All(char.IsDigit)))
            throw new GalaxyException.Auth("totp_invalid", Z("验证码不正确", "Incorrect verification code"));
        if (username.IndexOf("new", StringComparison.OrdinalIgnoreCase) >= 0)
        {
            lock (lk) pending = (serverAddr, username);
            return LoginResult.MustChangePassword;
        }
        FinishLogin(serverAddr, username);
        return LoginResult.Success;
    }

    private void FinishLogin(string serverAddr, string username)
    {
        var display = username.Length == 0 ? "?" : char.ToUpperInvariant(username[0]) + username.Substring(1);
        lock (lk)
        {
            user = new UserInfo(3, username, display, true, totpEnabled);
            server = new ServerInfo(serverAddr, Z("LemonIT 总部", "LemonIT HQ"), "0.1.0");
            pending = null;
        }
        store.Set("mock.session", Encoding.UTF8.GetBytes(serverAddr + "\n" + username));
        SetState(ClientState.Online);
        StartLiveSimulation();
    }

    public async Task CompleteFirstLogin(string newPassword)
    {
        await Delay(400);
        if (newPassword.Length < 8) throw new GalaxyException.Auth("weak_password", Z("新密码至少 8 位", "The new password must be at least 8 characters"));
        (string server, string username)? p;
        lock (lk) p = pending;
        if (p == null) throw new GalaxyException.Other(Z("请先登录", "Please sign in first"));
        FinishLogin(p.Value.server, p.Value.username);
    }

    public async Task ChangePassword(string oldPassword, string newPassword, string? totp)
    {
        await Delay(400);
        if (oldPassword == "wrong") throw new GalaxyException.Auth("invalid_credentials", Z("原密码不正确", "The current password is incorrect"));
        if (newPassword.Length < 8) throw new GalaxyException.Auth("weak_password", Z("新密码至少 8 位", "The new password must be at least 8 characters"));
    }

    public async Task Logout()
    {
        await Delay(150);
        store.Remove("mock.session");
        lock (lk) { user = null; server = null; }
        liveTimer?.Dispose();
        liveTimer = null;
        SetState(ClientState.LoggedOut);
    }

    public bool Restore()
    {
        var data = store.Get("mock.session");
        if (data == null) return false;
        var parts = Encoding.UTF8.GetString(data).Split(new[] { '\n' }, 2);
        if (parts.Length != 2) return false;
        FinishLogin(parts[0], parts[1]);
        return true;
    }

    public async Task<TotpSetup> TotpSetup()
    {
        await Delay(200);
        const string secret = "JBSWY3DPEHPK3PXPLGALAXY";
        return new TotpSetup(secret, $"otpauth://totp/LemonGalaxy:{CurrentUser()?.Username ?? "user"}?secret={secret}&issuer=LemonGalaxy");
    }

    public async Task TotpVerify(string code)
    {
        await Delay(200);
        if (code.Length != 6 || !code.All(char.IsDigit)) throw new GalaxyException.Auth("totp_invalid", Z("验证码不正确", "Incorrect verification code"));
        lock (lk) { totpEnabled = true; if (user != null) user = user with { TotpEnabled = true }; }
    }

    public async Task TotpDisable(string code)
    {
        await Delay(200);
        if (code.Length != 6) throw new GalaxyException.Auth("totp_invalid", Z("验证码不正确", "Incorrect verification code"));
        lock (lk) { totpEnabled = false; if (user != null) user = user with { TotpEnabled = false }; }
    }

    // ---------- 设备与加密库 ----------

    public DeviceInfo[] Devices() { lock (lk) return devices.ToArray(); }
    public DeviceInfo? Device(string id) { lock (lk) return devices.FirstOrDefault(d => d.Id == id); }

    public async Task RefreshDevices()
    {
        await Delay(300);
        queue.Post(listener.OnDevicesChanged);
    }

    public async Task RemoveDevice(string deviceId)
    {
        await Delay(200);
        lock (lk) devices.RemoveAll(d => d.Id == deviceId);
        queue.Post(listener.OnDevicesChanged);
    }

    public void SetLanDiscovery(bool active) { }

    public VaultEntry[] Vault() { lock (lk) return vault.ToArray(); }

    public async Task<VaultEntry> VaultSave(VaultEntry entry)
    {
        await Delay(120);
        var e = entry with { UpdatedAt = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() };
        lock (lk)
        {
            if (e.Id.Length == 0)
            {
                e = e with { Id = "v-" + Guid.NewGuid().ToString("N").Substring(0, 8) };
                vault.Add(e);
            }
            else
            {
                var i = vault.FindIndex(v => v.Id == e.Id);
                if (i >= 0) vault[i] = e;
                else vault.Add(e);
            }
        }
        queue.Post(listener.OnVaultChanged);
        return e;
    }

    public async Task VaultDelete(string id)
    {
        await Delay(120);
        lock (lk)
        {
            // 删除文件夹时把里面的条目移到上一级（界面会先确认）
            var f = vault.FirstOrDefault(v => v.Id == id);
            if (f?.Data is VaultData.Folder)
                for (var i = 0; i < vault.Count; i++)
                    if (vault[i].FolderId == id) vault[i] = vault[i] with { FolderId = f.FolderId };
            vault.RemoveAll(v => v.Id == id);
        }
        queue.Post(listener.OnVaultChanged);
    }

    // ---------- 连接 ----------

    public async Task<ICorePeer> Connect(string deviceId)
    {
        await Delay(600);
        var d = Device(deviceId) ?? throw new GalaxyException.NotFound(Z("设备不存在", "Device not found"));
        if (!d.Online && !d.LanSeen) throw new GalaxyException.Network(Z("设备离线", "The device is offline"));
        if (d.Kind == DeviceKind.Mobile) throw new GalaxyException.Permission(Z("手机端不能被控制", "Mobile devices cannot be controlled"));
        return new MockPeer(this, d);
    }

    public async Task<ICorePeer> ConnectAssist(string deviceCode, string verificationCode)
    {
        await Delay(900);
        var digits = new string(deviceCode.Where(char.IsDigit).ToArray());
        if (digits.Length != 9) throw new GalaxyException.NotFound(Z("设备码不存在", "Unknown device code"));
        if (verificationCode.Length < 6) throw new GalaxyException.Auth("invalid_credentials", Z("验证码不正确", "Incorrect verification code"));
        var d = new DeviceInfo("assist-" + digits, DeviceKind.Worker, Platform.Windows, "DESKTOP-7Q2K9LM", "Windows 11 23H2", "x86_64", "0.1.0", true, false, MockData.Now, false);
        return new MockPeer(this, d);
    }

    public async Task<ICoreSsh> ConnectSsh(string hostId, SecretPrompt prompt)
    {
        await Delay(500);
        VaultEntry? e;
        lock (lk) e = vault.FirstOrDefault(v => v.Id == hostId);
        if (e?.Data is not VaultData.SshHost h) throw new GalaxyException.NotFound(Z("主机不存在", "Host not found"));
        // 内核在自己的线程上同步调用 SecretPrompt；模拟内核同样放到后台线程
        bool known;
        lock (lk) known = vault.Any(v => v.Data is VaultData.KnownHost k && k.Host == h.Host && k.Port == h.Port);
        if (!known)
        {
            const string fp = "SHA256:Lm9uR2FsYXh5TW9ja0hvc3RLZXlGaW5nZXJwcmludA";
            var ok = await Task.Run(() => prompt.ConfirmHostKey(h.Host, h.Port, "ssh-ed25519", fp, false));
            if (!ok) throw new GalaxyException.Cancelled();
            await VaultSave(new VaultEntry("", null, 0, new VaultData.KnownHost(h.Host, h.Port, "ssh-ed25519", fp), 0));
        }
        if (h.Auth is SshAuth.Password { PasswordValue: null or "" })
        {
            var answer = await Task.Run(() => prompt.Ask(Z($"{h.Username}@{h.Host} 的密码：", $"Password for {h.Username}@{h.Host}:"), false));
            if (string.IsNullOrEmpty(answer)) throw new GalaxyException.Cancelled();
        }
        if (h.Auth is SshAuth.Interactive)
        {
            var answer = await Task.Run(() => prompt.Ask(Z("验证码（键盘交互）：", "Verification code (keyboard-interactive):"), true));
            if (answer == null) throw new GalaxyException.Cancelled();
        }
        var via = h.JumpDeviceId is { } jd ? "device:" + (Device(jd)?.Hostname ?? jd) : "direct";
        return new MockSsh(this, new SshInfo(hostId, h.Label, h.Host, h.Port, h.Username, via));
    }

    public async Task<RdpTarget> OpenRdp(string hostId)
    {
        await Delay(300);
        VaultEntry? e;
        lock (lk) e = vault.FirstOrDefault(v => v.Id == hostId);
        if (e?.Data is not VaultData.RdpHost r) throw new GalaxyException.NotFound(Z("主机不存在", "Host not found"));
        if (r.JumpDeviceId != null) return new RdpTarget(r.Label, "127.0.0.1", (ushort)(53_000 + r.Port % 1000), r.Username, r.Domain, r.Password, true, "fwd-" + hostId);
        return new RdpTarget(r.Label, r.Host, r.Port, r.Username, r.Domain, r.Password, false, "");
    }

    public void CloseRdp(string forwardId) { }

    public async Task<UpdateInfo?> CheckUpdate(string component, string platform, string arch, string current)
    {
        await Delay(600);
        return new UpdateInfo("0.2.0", "LemonGalaxy-0.2.0-windows-x64.zip", 24_117_248, "0f6c2b4e9d1a7c3b5e8f0a2d4c6e8b1a3d5f7c9e1b3a5c7e9f1d3b5a7c9e1f3b",
            "/api/updates/control/windows/LemonGalaxy-0.2.0-windows-x64.zip",
            Z("- 远程桌面支持隐私屏\n- SSH 直连与 SFTP\n- 修复若干问题", "- Privacy screen in remote desktop\n- SSH direct connections and SFTP\n- Bug fixes"));
    }

    public void DownloadUpdate(UpdateInfo info, string dest, TransferListener listener)
        => _ = new MockTransfer(info.Size, false, Zh, listener, Options.LatencyScale, ok =>
        {
            if (!ok) return;
            try { System.IO.File.WriteAllBytes(dest, new byte[16]); } catch { }
        });

    // ---------- AI ----------

    public async Task<AiModels> AiModels()
    {
        await Delay(200);
        return new AiModels(true, AiMode.FullTrust, new[]
        {
            new AiModel(1, "OpenCode Go", "kimi-k2", "Kimi K2", false, true, 131_072),
            new AiModel(1, "OpenCode Go", "qwen3-vl-plus", "Qwen3 VL Plus", true, true, 262_144),
            new AiModel(1, "OpenCode Go", "glm-4.6", "GLM 4.6", false, true, 200_000),
            new AiModel(2, Z("公司网关", "Company gateway"), "deepseek-v3.2", "DeepSeek V3.2", false, true, 128_000),
        });
    }

    public async Task<ICoreConversation> NewConversation(ConversationConfig config, AgentListener l)
    {
        await Delay(300);
        var c = new MockConversation(this, config, l);
        lock (lk) saved[c.Id()] = c;
        return c;
    }

    public ConversationSummary[] ListConversations()
    {
        List<MockConversation> all;
        lock (lk) all = saved.Values.ToList();
        return all.Select(c => c.Summary()).OrderByDescending(s => s.UpdatedAt).ToArray();
    }

    public async Task<ICoreConversation> OpenConversation(string id, ConversationConfig? config, AgentListener l)
    {
        await Delay(150);
        MockConversation? c;
        lock (lk) saved.TryGetValue(id, out c);
        if (c == null) throw new GalaxyException.NotFound(Z("对话不存在", "Conversation not found"));
        c.Listener = l;
        return c;
    }

    public void DeleteConversation(string id) { lock (lk) saved.Remove(id); }

    public bool DisableBuiltInHost() => true;
    public ITermScreen CreateScreen(ushort rows, ushort cols, uint scrollback) => new SimpleVt(rows, cols, scrollback);
    public IFrameDecoder CreateDecoder() => new MockFrameDecoder();
    public void SetLocale(string coreLocale) { }

    private void StartLiveSimulation()
    {
        if (!Options.LiveSimulation || liveTimer != null) return;
        // 每 25 秒让一台设备上下线一次，演示状态点变化
        liveTimer = new Timer(_ =>
        {
            lock (lk)
            {
                var i = devices.FindIndex(d => d.Id == "dev-nas");
                if (i >= 0) devices[i] = devices[i] with { Online = !devices[i].Online, LastSeenAt = MockData.Now };
            }
            listener.OnDevicesChanged();
        }, null, 25_000, 25_000);
    }

    public string TargetName(AgentTarget t)
    {
        lock (lk)
        {
            if (t.Kind == AgentTargetKind.Device)
            {
                var meta = vault.Select(v => v.Data).OfType<VaultData.DeviceMeta>().FirstOrDefault(m => m.DeviceId == t.Id);
                if (!string.IsNullOrEmpty(meta?.Alias)) return meta!.Alias!;
                return devices.FirstOrDefault(d => d.Id == t.Id)?.Hostname ?? t.Id;
            }
            return vault.FirstOrDefault(v => v.Id == t.Id)?.Data is VaultData.SshHost h ? h.Label : t.Id;
        }
    }

    public Platform TargetPlatform(AgentTarget t) => t.Kind == AgentTargetKind.Device ? Device(t.Id)?.Platform ?? Platform.Linux : Platform.Linux;
}

internal sealed class MockPeer : ICorePeer
{
    private readonly MockClient client;
    private readonly DeviceInfo device;
    private PeerListener? listener;
    private PathInfo current;
    private bool closed;
    private MockFileClient? files;
    private Timer? timer;

    public MockPeer(MockClient client, DeviceInfo device)
    {
        this.client = client;
        this.device = device;
        current = MockData.PathFor(device);
    }

    public PeerInfo Info() => new(device.Id, device.Hostname, device.Platform, device.OsVersion, device.Arch, MockData.Username(device),
        device.Platform == Platform.Windows ? "powershell" : "/bin/bash", MockData.PermissionsFor(device), MockData.Capabilities(device), current);

    public PathInfo Path() => current;

    public void SetListener(PeerListener l)
    {
        listener = l;
        // 中转起步的连接 4 秒后「升级」成直连，演示路径无缝切换
        if (!client.Options.LiveSimulation || current.Kind != PathKind.RelayUdp) return;
        timer = new Timer(_ =>
        {
            if (closed) return;
            current = new PathInfo(PathKind.Direct, 21, "203.0.113.24:51820");
            listener?.OnPathChanged(current);
        }, null, 4000, Timeout.Infinite);
    }

    public bool IsClosed() => closed;
    public void Disconnect() { closed = true; timer?.Dispose(); }

    public async Task<ICoreTerminal> OpenTerminal(TerminalOptions options, TerminalSink sink)
    {
        await client.Delay(250);
        if (!MockData.PermissionsFor(device).Terminal)
            throw new GalaxyException.Permission(client.Zh ? "对方没有给你终端权限" : "The remote owner has not granted terminal access");
        return new MockTerminalChannel(client, device, options, sink);
    }

    public async Task<TerminalSessionInfo[]> ListTerminals() { await client.Delay(150); return MockTerminalRegistry.Shared.Sessions(device.Id); }
    public async Task KillTerminal(string sessionId) { await client.Delay(100); MockTerminalRegistry.Shared.Remove(sessionId); }

    public ICoreFiles Files()
    {
        if (!MockData.PermissionsFor(device).Files)
            return new DeniedFiles(client.Zh ? "对方没有给你文件权限" : "The remote owner has not granted file access");
        return files ??= new MockFileClient(client, device.Platform, MockData.Username(device));
    }

    public async Task<ExecResult> Exec(ExecRequest request)
    {
        await client.Delay(300);
        var r = MockShell.Run(request.Command, device, request.Cwd ?? "~", client.Zh);
        return new ExecResult(r.code, r.text, "", false, false, 42);
    }

    public async Task<DisplayInfo[]> ListDisplays() { await client.Delay(150); return MockData.Displays(device); }

    public async Task<ICoreDisplay> OpenDisplay(DisplayOptions options, VideoSink sink)
    {
        await client.Delay(350);
        if (!MockData.PermissionsFor(device).View)
            throw new GalaxyException.Permission(client.Zh ? "对方没有给你查看画面的权限" : "The remote owner has not granted screen access");
        var displays = MockData.Displays(device);
        if (displays.Length == 0) throw new GalaxyException.NotFound(client.Zh ? "对方没有可用的屏幕" : "No display available");
        var d = displays.FirstOrDefault(x => x.Id == options.DisplayId) ?? displays[0];
        return new MockDisplayChannel(device, d, options, sink, client.Zh, client.Options.LiveSimulation);
    }

    public async Task<ImageData> Screenshot(uint displayId, uint maxWidth)
    {
        await client.Delay(200);
        var d = MockData.Displays(device).FirstOrDefault(x => x.Id == displayId) ?? MockData.Displays(device)[0];
        return new MockDesktopRenderer(device, client.Zh, 1280, (int)(1280.0 * d.Height / d.Width)).Png((int)maxWidth);
    }

    public async Task<ClipboardData?> ClipboardGet()
    {
        await client.Delay(80);
        return new ClipboardData.Text(client.Zh ? "远端剪贴板里的文字" : "Text from the remote clipboard");
    }

    public Task ClipboardSet(ClipboardData data) => client.Delay(80);

    public async Task<ICorePortForward> ForwardPort(ushort localPort, string remoteHost, ushort remotePort)
    {
        await client.Delay(200);
        return new MockPortForward(localPort == 0 ? (ushort)(50_000 + remotePort % 1000) : localPort);
    }

    public Task WakeOnLan(string mac) => client.Delay(100);
    public Task SetPrivacy(bool on) => client.Delay(150);
}

internal sealed class MockSsh : ICoreSsh
{
    private readonly MockClient client;
    private readonly SshInfo info;
    private readonly DeviceInfo device;
    private MockFileClient? files;
    private bool closed;

    public MockSsh(MockClient client, SshInfo info)
    {
        this.client = client;
        this.info = info;
        device = new DeviceInfo("ssh-" + info.Host, DeviceKind.Worker, Platform.Linux, info.Label, "Debian GNU/Linux 12", "x86_64", "", true, false, MockData.Now, false);
    }

    public SshInfo Info() => info;
    public bool IsClosed() => closed;
    public void Disconnect() => closed = true;

    public async Task<ICoreTerminal> OpenTerminal(TerminalOptions options, TerminalSink sink)
    {
        await client.Delay(200);
        return new MockTerminalChannel(client, device, options with { AttachSession = null }, sink, info.Username, register: false);
    }

    public ICoreFiles Files() => files ??= new MockFileClient(client, Platform.Linux, info.Username);

    public async Task<ExecResult> Exec(ExecRequest request)
    {
        await client.Delay(200);
        var r = MockShell.Run(request.Command, device, request.Cwd ?? "~", client.Zh);
        return new ExecResult(r.code, r.text, "", false, false, 30);
    }

    public Task<ICorePortForward> ForwardPort(ushort localPort, string remoteHost, ushort remotePort)
        => Task.FromResult<ICorePortForward>(new MockPortForward(localPort == 0 ? (ushort)(52_000 + remotePort % 1000) : localPort));
}

internal sealed class MockPortForward : ICorePortForward
{
    private readonly ushort port;
    public MockPortForward(ushort port) { this.port = port; }
    public ushort LocalPort() => port;
    public void Stop() { }
}

/// <summary>没有文件权限时：每个操作都报权限错误</summary>
internal sealed class DeniedFiles : ICoreFiles
{
    private readonly string message;
    public DeniedFiles(string message) { this.message = message; }
    private Exception E => new GalaxyException.Permission(message);
    public Task<string> Home() => Task.FromException<string>(E);
    public Task<FileEntry[]> List(string path) => Task.FromException<FileEntry[]>(E);
    public Task<FileEntry> Stat(string path) => Task.FromException<FileEntry>(E);
    public Task<string> Realpath(string path) => Task.FromException<string>(E);
    public Task Mkdir(string path) => Task.FromException(E);
    public Task Rename(string from, string to) => Task.FromException(E);
    public Task Remove(string path, bool recursive) => Task.FromException(E);
    public Task Chmod(string path, uint mode) => Task.FromException(E);
    public Task<FileEntry[]> Search(string root, string pattern, bool content) => Task.FromException<FileEntry[]>(E);
    public Task<string> ReadText(string path, ulong maxBytes) => Task.FromException<string>(E);
    public Task WriteText(string path, string text) => Task.FromException(E);
    public ICoreTransfer Download(string remote, string local, TransferListener listener) { listener.OnFinished(message); return new NoTransfer(); }
    public ICoreTransfer Upload(string local, string remote, TransferListener listener) { listener.OnFinished(message); return new NoTransfer(); }

    private sealed class NoTransfer : ICoreTransfer
    {
        public void Cancel() { }
        public void Pause() { }
        public void Resume() { }
    }
}
