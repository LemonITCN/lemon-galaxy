using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;

namespace LemonGalaxy.UI;

internal enum LoginStep { Credentials, Totp, ChangePassword }

internal sealed class LoginModel : Observable
{
    public LoginStep Step = LoginStep.Credentials;
    public string Server = "", Username = "", Password = "", Totp = "", NewPassword = "", ConfirmPassword = "";
    public bool Busy;
    public string? Error;
    public bool RememberServer = true;
}

/// <summary>登录：服务器地址（含证书指纹）、用户名密码 → 两步验证 → 首次登录改密码</summary>
internal static class LoginView
{
    public static FrameworkElement Build(AppModel app, Action<LoginModel>? preset = null)
    {
        var s = app.Settings;
        var m = new LoginModel { Server = s.LastServer, Username = s.LastUsername };
        // 模拟内核：预填演示地址，方便直接体验（任意密码可登录，用户名含 totp / new 可演示两步验证与首次改密码）
        if (app.Client.IsMock && m.Server.Length == 0 && !app.SelfTest)
        {
            m.Server = "demo.lemongalaxy.test:7443#sha256-TW9ja0NvcmVEZW1vU2VydmVyRmluZ2VycHJpbnQwMDA";
            if (m.Username.Length == 0) m.Username = "alice";
        }
        preset?.Invoke(m);

        var root = new Grid { Background = Th.Content, ClipToBounds = true };
        // 克制的背景：极淡的强调色径向渐变
        root.Children.Add(new Border
        {
            Background = new RadialGradientBrush(Palettes.A(Th.P.Accent, Th.P.IsDark ? 0.16 : 0.10), Colors.Transparent)
            { Center = new Point(0, 0), GradientOrigin = new Point(0, 0), RadiusX = 0.9, RadiusY = 1.1 },
        });
        root.Children.Add(new Border
        {
            Background = new RadialGradientBrush(Palettes.A(Th.P.Accent, Th.P.IsDark ? 0.08 : 0.05), Colors.Transparent)
            { Center = new Point(1, 1), GradientOrigin = new Point(1, 1), RadiusX = 0.8, RadiusY = 0.9 },
        });

        // 只在步骤变化时重建卡片（输入框要保持焦点）
        var card = new Region(() => Card(app, m)).Keyed(() => m.Step).On(m);
        var col = K.Col(22,
            K.Center(K.Col(10, K.Center(K.AppMark(54)), K.Center(K.Semi("LemonGalaxy", 23)))),
            new Border
            {
                Width = 400, Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(14), Padding = new Thickness(26, 24, 26, 24),
                Effect = new System.Windows.Media.Effects.DropShadowEffect { BlurRadius = 26, ShadowDepth = 6, Direction = 270, Opacity = Th.P.IsDark ? 0.4 : 0.1 },
                Child = card,
            },
            K.Center(Footer(app)));
        col.HorizontalAlignment = HorizontalAlignment.Center;
        col.VerticalAlignment = VerticalAlignment.Center;
        root.Children.Add(col);
        var buttons = new WindowButtons { HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top };
        root.Children.Add(buttons);
        return root;
    }

    private static FrameworkElement Footer(AppModel app)
    {
        var s = app.Settings;
        var lang = K.MenuButton(K.Row(5, K.Icon("globe", 13, Th.Secondary), K.Text(L.Pack.Name, 12.5, Th.Secondary)), () =>
        {
            var list = new List<MenuEntry> { new(L.T("settings.language.system"), () => s.Language = "system", Checked: s.Language == "system") };
            list.AddRange(L.Packs.Select(p => new MenuEntry(p.Name, () => s.Language = p.Code, Checked: s.Language == p.Code)));
            return list;
        });
        var items = new List<UIElement?> { lang };
        if (app.Client.IsMock)
        {
            items.Add(K.Badge(L.T("about.mock"), Th.P.Warning));
            if (app.CoreFallbackReason != null) items.Add(K.Info(L.T("about.mock.fallback", app.CoreFallbackReason)));
        }
        return K.Row(12, items);
    }

    private static FrameworkElement Field(string label, UIElement input, string? info = null)
        => K.Col(6, K.Row(4, K.Text(label, 12.5, Th.Secondary), info == null ? null : K.Info(info)), input);

    private static FrameworkElement Card(AppModel app, LoginModel m)
    {
        var error = new Region(() => m.Error == null ? null
            : K.Row(6, K.Top(K.Icon("alertCircle", 15, Th.Danger)), K.Grow(K.Text(m.Error, 12.5, Th.Danger, wrap: true)))).On(m);
        var body = m.Step switch
        {
            LoginStep.Totp => Totp(app, m),
            LoginStep.ChangePassword => ChangePassword(app, m),
            _ => Credentials(app, m),
        };
        return K.Col(16, body, error);
    }

    private static FrameworkElement Credentials(AppModel app, LoginModel m)
    {
        var server = K.TextField("galaxy.example.com:7443#sha256-…", m.Server, mono: true, fontSize: 12.5);
        var invalid = K.Text(L.T("login.server.invalid"), 12, Th.Danger);
        var user = K.TextField("", m.Username);
        var pwd = K.PasswordField("", m.Password);
        var remember = K.CheckBox(L.T("login.remember"), m.RememberServer, v => m.RememberServer = v, 12.5);
        var signIn = LoginButton(L.T("login.signIn"), m, () => Login(app, m));
        void Update()
        {
            var valid = ServerAddress.Parse(m.Server) != null;
            server.Invalid = m.Server.Length > 0 && !valid;
            invalid.Visibility = server.Invalid ? Visibility.Visible : Visibility.Collapsed;
            signIn.IsEnabled = !m.Busy && valid && m.Username.Length > 0 && m.Password.Length > 0;
        }
        server.Changed += v => { m.Server = v; Update(); };
        user.Changed += v => { m.Username = v; Update(); };
        pwd.Changed += v => { m.Password = v; Update(); };
        pwd.Submitted += () => { if (signIn.IsEnabled) Login(app, m); };
        user.Submitted += () => pwd.FocusInput();
        server.Submitted += () => user.FocusInput();
        m.Watch(signIn, Update);
        Update();
        return K.Col(14,
            K.Semi(L.T("login.title"), 17),
            Field(L.T("login.server"), K.Col(5, server, invalid), L.T("login.server.info")),
            Field(L.T("login.username"), user),
            Field(L.T("login.password"), pwd),
            remember,
            signIn);
    }

    private static Btn LoginButton(string title, LoginModel m, Action a)
    {
        var b = K.PrimaryButton(title, a);
        b.Height = 34;
        var spinner = new Region(() => m.Busy ? K.Spinner(14, Th.AccentText) : null).On(m);
        b.Child = K.Center(K.Row(8, spinner, K.Text(title, 13.5, Th.AccentText, FontWeights.SemiBold)));
        return b;
    }

    private static FrameworkElement StepHeader(string title, string icon) => K.Row(9, K.Icon(icon, 18, Th.Accent), K.Semi(title, 17));

    private static FrameworkElement Totp(AppModel app, LoginModel m)
    {
        var code = K.TextField("000000", m.Totp, mono: true, fontSize: 20);
        code.Height = 42;
        if (code.Box != null) code.Box.TextAlignment = TextAlignment.Center;
        var verify = K.PrimaryButton(L.T("login.verify"), () => Login(app, m));
        void Update() => verify.IsEnabled = !m.Busy && m.Totp.Count(char.IsDigit) == 6;
        code.Changed += v => { m.Totp = v; Update(); };
        code.Submitted += () => { if (verify.IsEnabled) Login(app, m); };
        m.Watch(verify, Update);
        Update();
        return K.Col(14,
            StepHeader(L.T("login.totp.title"), "shield"),
            K.Text(L.T("login.totp.prompt"), 13, Th.Secondary, wrap: true),
            code,
            K.Row(8, K.SecondaryButton(L.T("common.back"), () => { m.Step = LoginStep.Credentials; m.Totp = ""; m.Error = null; m.Notify(); }), K.Spacer(), verify));
    }

    private static int Strength(string s)
    {
        var score = 0;
        if (s.Length >= 8) score++;
        if (s.Length >= 12) score++;
        if (s.Any(char.IsDigit) && s.Any(char.IsLetter)) score++;
        if (s.Any(c => !char.IsLetterOrDigit(c))) score++;
        return score;
    }

    private static FrameworkElement ChangePassword(AppModel app, LoginModel m)
    {
        var np = K.PasswordField("", m.NewPassword);
        var cp = K.PasswordField("", m.ConfirmPassword);
        var bars = new Region(() =>
        {
            var st = Strength(m.NewPassword);
            var color = st <= 1 ? Th.Danger : st == 2 ? Th.Warning : Th.Success;
            var g = new UniformGrid { Columns = 4, Height = 4 };
            for (var i = 0; i < 4; i++) g.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = i < st ? color : Th.Border, Margin = new Thickness(i == 0 ? 0 : 2, 0, i == 3 ? 0 : 2, 0) });
            return g;
        }).On(m);
        var submit = LoginButton(L.T("login.change.submit"), m, () => Change(app, m));
        void Update()
        {
            cp.Invalid = m.ConfirmPassword.Length > 0 && m.ConfirmPassword != m.NewPassword;
            submit.IsEnabled = !m.Busy && m.NewPassword.Length >= 8 && m.NewPassword == m.ConfirmPassword;
        }
        np.Changed += v => { m.NewPassword = v; Update(); m.Notify(); };
        cp.Changed += v => { m.ConfirmPassword = v; Update(); };
        cp.Submitted += () => { if (submit.IsEnabled) Change(app, m); };
        m.Watch(submit, Update);
        Update();
        return K.Col(14,
            StepHeader(L.T("login.change.title"), "key"),
            K.Row(6, K.Text(L.T("login.change.prompt"), 13, Th.Secondary), K.Info(L.T("login.change.info"))),
            Field(L.T("login.change.new"), np),
            bars,
            Field(L.T("login.change.confirm"), cp),
            submit);
    }

    private static void Login(AppModel app, LoginModel m)
    {
        if (m.Busy) return;
        m.Busy = true;
        m.Error = null;
        m.Notify();
        var totp = m.Step == LoginStep.Totp ? new string(m.Totp.Where(char.IsDigit).ToArray()) : null;
        Ui.Run(async () =>
        {
            try
            {
                var r = await app.Client.Login(m.Server.Trim(), m.Username, m.Password, totp);
                switch (r)
                {
                    case LoginResult.Success:
                        Remember(app, m);
                        app.DidLogin();
                        break;
                    case LoginResult.NeedTotp:
                        m.Step = LoginStep.Totp;
                        break;
                    case LoginResult.MustChangePassword:
                        m.Step = LoginStep.ChangePassword;
                        break;
                }
            }
            catch (Exception e) { m.Error = Fmt.Error(e); }
            m.Busy = false;
            m.Notify();
        });
    }

    private static void Change(AppModel app, LoginModel m)
    {
        if (m.Busy || m.NewPassword != m.ConfirmPassword || m.NewPassword.Length < 8) return;
        m.Busy = true;
        m.Error = null;
        m.Notify();
        Ui.Run(async () =>
        {
            try
            {
                await app.Client.CompleteFirstLogin(m.NewPassword);
                Remember(app, m);
                app.DidLogin();
            }
            catch (Exception e) { m.Error = Fmt.Error(e); }
            m.Busy = false;
            m.Notify();
        });
    }

    private static void Remember(AppModel app, LoginModel m)
    {
        app.Settings.LastUsername = m.Username;
        app.Settings.LastServer = m.RememberServer ? m.Server.Trim() : "";
        m.Password = m.NewPassword = m.ConfirmPassword = "";
    }
}
