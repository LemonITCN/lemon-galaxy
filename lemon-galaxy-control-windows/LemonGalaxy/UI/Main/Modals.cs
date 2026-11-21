using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>主窗口里的模态卡片内容</summary>
internal static class Modals
{
    public static FrameworkElement Header(AppModel app, string title, string? subtitle = null, UIElement? icon = null, string? info = null, bool closable = true)
    {
        var text = K.Col(2, K.Row(6, K.Semi(title, 15), info == null ? null : K.Info(info)), subtitle == null ? null : K.Text(subtitle, 12, Th.Secondary));
        return K.Row(10, icon, K.Grow(text), closable ? K.IconButton("close", L.T("common.close"), () => app.CloseModal(true), size: 12) : null);
    }

    public static FrameworkElement Buttons(params UIElement?[] buttons)
        => K.Row(8, new UIElement?[] { K.Spacer() }.Concat(buttons).ToArray());

    /// <summary>双击设备后的连接方式选择：远程桌面 / 终端 / 文件 / AI</summary>
    public static FrameworkElement ConnectChooser(AppModel app, string deviceId)
    {
        var d = app.Device(deviceId);
        var gui = d != null && d.Platform is Platform.Windows or Platform.Macos;
        FrameworkElement Choice(string icon, string title, bool enabled, Action a)
        {
            var b = new Btn
            {
                Height = 86, CornerRadius = new CornerRadius(9), BorderThickness = new Thickness(1), Normal = Th.Elevated, NormalBorder = Th.Border,
                HoverBrush = Th.B(K.Mix(Th.P.Elevated, Th.P.Accent, 0.08)), HoverBorder = Th.B(Th.P.Accent, 0.6),
                Child = K.Center(K.Col(9, K.Center(K.Icon(icon, 24, enabled ? Th.Text : Th.Tertiary, 1.5)), K.Center(K.Text(title, 12.5, enabled ? Th.Text : Th.Tertiary, FontWeights.Medium)))),
            };
            b.UpdateLook();
            b.IsEnabled = enabled;
            b.Click += () => { app.CloseModal(); a(); };
            return K.Grow(b);
        }
        var choices = K.Row(10,
            Choice("display", L.T("connect.desktop"), gui, () => app.OpenRemoteDesktop(deviceId)),
            Choice("terminal", L.T("connect.terminal"), true, () => app.OpenTerminalAsking(deviceId)),
            Choice("folder", L.T("connect.files"), true, () => app.OpenSftp(deviceId)),
            Choice("sparkles", L.T("connect.ai"), app.AiEnabled, () => app.OpenAi(new AgentTarget(AgentTargetKind.Device, deviceId))));
        FrameworkElement? offline = null;
        if (d is { Online: false })
            offline = K.Row(6, K.StatusDot(K.DotOf(d), 7, Th.Card), K.Text(d.LanSeen ? L.T("device.lanOnly") : L.T("device.offline"), 12, Th.Secondary));
        return K.Col(16,
            Header(app, app.DeviceName(deviceId), d == null ? null : $"{d.Hostname} · {d.OsVersion}", K.PlatformGlyph(d?.Platform ?? Platform.Linux, 22, Th.Secondary)),
            choices, offline);
    }

    /// <summary>SSH 的密码 / 口令 / 键盘交互问题</summary>
    public static FrameworkElement SecretPromptCard(AppModel app, string title, string prompt, bool echo, Action<string?> respond)
    {
        var field = echo ? K.TextField() : K.PasswordField();
        var answered = false;
        void Submit()
        {
            if (answered) return;
            answered = true;
            var v = field.Text;
            app.CloseModal();
            respond(v);
        }
        void Cancel()
        {
            if (answered) return;
            answered = true;
            app.CloseModal();
            respond(null);
        }
        field.Submitted += Submit;
        field.Escaped += Cancel;
        return K.Col(14,
            K.Row(10, K.Icon("key", 18, Th.Accent), K.Grow(K.Semi(title, 15))),
            K.Text(prompt, 13, Th.Secondary, wrap: true),
            field,
            Buttons(K.SecondaryButton(L.T("common.cancel"), Cancel), K.PrimaryButton(L.T("common.ok"), Submit)));
    }

    /// <summary>首次连接主机或主机密钥变化时确认指纹</summary>
    public static FrameworkElement HostKeyCard(AppModel app, string host, ushort port, string keyType, string fingerprint, bool changed, Action<bool> respond)
    {
        var done = false;
        void Answer(bool ok)
        {
            if (done) return;
            done = true;
            app.CloseModal();
            respond(ok);
        }
        var target = port == 22 ? host : $"{host}:{port}";
        FrameworkElement message = changed
            ? K.Box(K.Row(10, K.Top(K.Icon("shieldAlert", 18, Th.Danger)), K.Grow(K.Col(4,
                    K.Text(L.T("hostkey.changed.message", target), 13, Th.Danger, FontWeights.Medium, wrap: true),
                    K.Text(L.T("hostkey.changed.hint"), 12.5, Th.Danger, wrap: true)))),
                Th.B(Th.P.Danger, 0.1), 9, Th.B(Th.P.Danger, 0.4), 1, new Thickness(12))
            : K.Text(L.T("hostkey.new.message", target), 13, Th.Secondary, wrap: true);
        FrameworkElement Line(string label, UIElement value) => K.Row(10, K.Width(K.Text(label, 12, Th.Tertiary), 110), K.Grow(value));
        var details = K.Box(K.Col(6,
                Line(L.T("hostkey.host"), K.Text(target, 12.5, Th.Text, mono: true)),
                Line(L.T("hostkey.type"), K.Text(keyType, 12.5, Th.Text, mono: true)),
                Line(L.T("hostkey.fingerprint"), K.SelectableText(fingerprint, 12.5, Th.Text, mono: true))),
            Th.CodeBg, 7, null, 0, new Thickness(10, 8, 10, 8));
        return K.Col(14,
            K.Row(10, K.Icon(changed ? "shieldAlert" : "shield", 18, changed ? Th.Danger : Th.Accent),
                K.Grow(K.Semi(changed ? L.T("hostkey.changed.title") : L.T("hostkey.new.title"), 15)), K.Info(L.T("hostkey.info"))),
            message, details,
            Buttons(K.SecondaryButton(L.T("common.cancel"), () => Answer(false)),
                K.PrimaryButton(changed ? L.T("hostkey.trustAnyway") : L.T("hostkey.trust"), () => Answer(true), destructive: changed)));
    }

    /// <summary>临时协助：输入对方屏幕上的设备码与验证码</summary>
    public static FrameworkElement AssistConnectCard(AppModel app)
    {
        var deviceCode = K.TextField("123 456 789", mono: true, fontSize: 15);
        var code = K.TextField("", mono: true, fontSize: 15);
        var error = K.Text("", 12.5, Th.Danger, wrap: true);
        error.Visibility = Visibility.Collapsed;
        var busy = false;
        Btn? go = null;
        void Update() { if (go != null) go.IsEnabled = !busy && deviceCode.Text.Count(char.IsDigit) == 9 && code.Text.Length >= 6; }
        void Connect()
        {
            if (busy || go is { IsEnabled: false }) return;
            busy = true;
            Update();
            error.Visibility = Visibility.Collapsed;
            Ui.Run(async () =>
            {
                try
                {
                    var s = await app.Client.ConnectAssist(deviceCode.Text, code.Text);
                    var id = app.RegisterAssistPeer(s);
                    app.CloseModal();
                    app.OpenRemoteDesktop(id);
                }
                catch (Exception e)
                {
                    error.Text = Fmt.Error(e);
                    error.Visibility = Visibility.Visible;
                }
                busy = false;
                Update();
            });
        }
        go = K.PrimaryButton(L.T("assist.connect.go"), Connect);
        deviceCode.Changed += _ => Update();
        code.Changed += _ => Update();
        code.Submitted += Connect;
        Update();
        return K.Col(14,
            Header(app, L.T("assist.connect.title"), info: L.T("assist.connect.info")),
            K.Col(6, K.Text(L.T("assist.deviceCode"), 12.5, Th.Secondary), deviceCode),
            K.Col(6, K.Text(L.T("assist.verificationCode"), 12.5, Th.Secondary), code),
            error,
            Buttons(K.SecondaryButton(L.T("common.cancel"), () => app.CloseModal()), go));
    }

    /// <summary>被控端保留的终端会话：选一个接续，或新开</summary>
    public static FrameworkElement TerminalSessionsCard(AppModel app, string deviceId, TerminalSessionInfo[] sessions)
    {
        var rows = new List<UIElement?>();
        for (var i = 0; i < sessions.Length; i++)
        {
            var s = sessions[i];
            var attach = K.SecondaryButton(L.T("terminal.sessions.attach"), () =>
            {
                app.CloseModal();
                if (app.ActiveTab is TerminalTab t && t.DeviceId == deviceId && t.Phase != TerminalPhase.Running) t.Reconnect(s);
                else app.OpenTerminal(deviceId, s);
            });
            rows.Add(K.Margin(K.Row(10, K.Icon("terminal", 15), K.Grow(K.Col(2, K.Text(s.Title, 13, Th.Text, FontWeights.Medium), K.Text(L.T("terminal.sessions.started", Fmt.Date(s.CreatedAt)), 11.5, Th.Tertiary))), attach), 0, 8, 0, 8));
            if (i < sessions.Length - 1) rows.Add(K.HLine());
        }
        return K.Col(14,
            Header(app, L.T("terminal.sessions.title"), app.DeviceName(deviceId)),
            K.Box(K.Col(0, rows), Th.Elevated, 8, Th.Border, 1, new Thickness(12, 0, 12, 0)),
            Buttons(K.PrimaryButton(L.T("terminal.sessions.new"), () => { app.CloseModal(); app.OpenTerminal(deviceId); })));
    }

    public static FrameworkElement ConfirmCard(AppModel app, string title, string message, string button, bool destructive, Action action)
        => K.Col(12,
            K.Semi(title, 15),
            K.Text(message, 13, Th.Secondary, wrap: true),
            K.Margin(Buttons(K.SecondaryButton(L.T("common.cancel"), () => app.CloseModal()), K.PrimaryButton(button, () => { app.CloseModal(); action(); }, destructive)), 0, 6, 0, 0));

    /// <summary>单行输入的小卡片（重命名、新建文件夹）</summary>
    public static void Prompt(AppModel app, string title, string initial, string button, Action<string> onOk, string placeholder = "")
    {
        app.ShowModal(new ModalSpec
        {
            Width = 380,
            Build = () =>
            {
                var f = K.TextField(placeholder, initial);
                void Ok()
                {
                    var v = f.Text.Trim();
                    if (v.Length == 0) return;
                    app.CloseModal();
                    onOk(v);
                }
                f.Submitted += Ok;
                f.Loaded += (_, _) => f.FocusInput();
                return K.Col(14, K.Semi(title, 15), f, Buttons(K.SecondaryButton(L.T("common.cancel"), () => app.CloseModal()), K.PrimaryButton(button, Ok)));
            },
        });
    }
}
