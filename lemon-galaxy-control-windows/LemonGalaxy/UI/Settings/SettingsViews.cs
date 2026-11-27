using System.Globalization;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>设置：账号（改密码、两步验证）、外观（主题、配色、语言、布局）、终端、连接、AI、关于</summary>
internal static class SettingsViews
{
    private static readonly SettingsPage[] Pages = { SettingsPage.Account, SettingsPage.Appearance, SettingsPage.Terminal, SettingsPage.Connection, SettingsPage.Ai, SettingsPage.About };

    private static string Key(SettingsPage p) => p switch
    {
        SettingsPage.Account => "account",
        SettingsPage.Appearance => "appearance",
        SettingsPage.Terminal => "terminal",
        SettingsPage.Connection => "connection",
        SettingsPage.Ai => "ai",
        _ => "about",
    };

    private static string Icon(SettingsPage p) => p switch
    {
        SettingsPage.Account => "userCircle",
        SettingsPage.Appearance => "palette",
        SettingsPage.Terminal => "terminal",
        SettingsPage.Connection => "network",
        SettingsPage.Ai => "sparkles",
        _ => "info",
    };

    public static FrameworkElement Middle(AppModel app)
    {
        var list = new Region(() =>
        {
            var items = new List<UIElement?>();
            foreach (var p in Pages)
            {
                var sel = app.SettingsPage == p && app.ActiveTabId == null;
                var b = new Btn
                {
                    Height = 34, CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 0, 10, 0), Normal = sel ? Th.Selection : Brushes.Transparent, HoverBrush = sel ? Th.Selection : Th.Hover,
                    Child = K.Row(10, K.Width(K.Center(K.Icon(Icon(p), 15, sel ? Th.Accent : Th.Secondary)), 20), K.Grow(K.Text(L.T("settings.page." + Key(p)), 13.5, Th.Text, sel ? FontWeights.SemiBold : FontWeights.Normal)),
                        p == SettingsPage.About && app.Updater.HasUpdate ? Sidebar.UpdateDot() : null),
                };
                b.UpdateLook();
                var page = p;
                b.Click += () => { app.SettingsPage = page; app.ActiveTabId = null; app.Nav.Notify(); };
                items.Add(b);
            }
            return K.Margin(K.Col(2, items), 8);
        }).On(app.Nav, app.Updater);
        var dock = new DockPanel();
        var top = K.Col(0, Shell.ColumnHeader(L.T("nav.settings")), K.HLine());
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        dock.Children.Add(K.Scroll(list));
        return dock;
    }

    public static FrameworkElement Home(AppModel app)
        => new Region(() => app.SettingsPage switch
        {
            SettingsPage.Account => Account(app),
            SettingsPage.Appearance => new Region(() => Appearance(app)).On(app.Settings),
            SettingsPage.Terminal => new Region(() => Terminal(app)).On(app.Settings),
            SettingsPage.Connection => new Region(() => Connection(app)).On(app.Settings),
            SettingsPage.Ai => new Region(() => Ai(app)).On(app.Settings),
            _ => About(app),
        }).Keyed(() => app.SettingsPage).On(app.Nav);

    private static FrameworkElement PageTitle(string t) => K.Semi(t, 21);

    private static FrameworkElement Section(string title, string? info, params UIElement?[] content)
        => K.Col(10, K.Row(5, K.SectionLabel(title), info == null ? null : K.Info(info)), K.Card(K.Col(14, content)));

    private static FrameworkElement ToggleRow(string label, bool on, Action<bool> set, string? info = null)
        => K.Row(6, K.Text(label, 13.5), info == null ? null : K.Info(info), K.Spacer(), K.Switch(on, set));

    private static FrameworkElement FormRow(string label, UIElement content, double labelWidth = 170)
        => K.Row(12, K.Width(K.Text(label, 13, Th.Secondary).Also(t => t.TextAlignment = TextAlignment.Right), labelWidth), content);

    // ---------- 账号 ----------

    private static FrameworkElement Account(AppModel app)
    {
        var u = app.User;
        var s = app.Server;
        var header = K.Row(14, K.Avatar(u?.DisplayName ?? "", 50), K.Grow(K.Col(3, K.Semi(u?.DisplayName ?? "", 16), K.Text("@" + (u?.Username ?? ""), 13, Th.Secondary))),
            K.SecondaryButton(L.T("account.logout"), () => app.Confirm(L.T("account.logout"), L.T("account.logout.message"), L.T("account.logout"), () => Ui.Run(app.Logout)), destructive: true, icon: "logout"));
        var info = HostsViews.InfoGrid(new[]
        {
            (L.T("account.server"), s?.Name ?? "—"), (L.T("account.address"), s?.Address ?? "—"), (L.T("account.serverVersion"), s?.Version ?? "—"),
            (L.T("account.ai"), u?.AiEnabled == true ? L.T("common.on") : L.T("common.off")),
        });
        return K.PageScroll(K.Col(22, PageTitle(L.T("settings.page.account")), header, info, ChangePassword(app), Totp(app)), 780);
    }

    private static FrameworkElement ChangePassword(AppModel app)
    {
        var totpEnabled = app.User?.TotpEnabled ?? false;
        var oldPw = K.PasswordField(width: 260);
        var newPw = K.PasswordField(width: 260);
        var confirm = K.PasswordField(width: 260);
        var code = K.TextField("000000", mono: true, width: 120);
        var message = K.Text("", 12.5, Th.Success);
        var busy = false;
        Btn? submit = null;
        void Update()
        {
            confirm.Invalid = confirm.Text.Length > 0 && confirm.Text != newPw.Text;
            if (submit != null) submit.IsEnabled = !busy && oldPw.Text.Length > 0 && newPw.Text.Length >= 8 && newPw.Text == confirm.Text;
        }
        submit = K.PrimaryButton(L.T("account.changePassword"), () =>
        {
            busy = true;
            Update();
            Ui.Run(async () =>
            {
                try
                {
                    await app.Client.ChangePassword(oldPw.Text, newPw.Text, code.Text.Trim().Length == 0 ? null : code.Text.Trim());
                    message.Text = L.T("account.passwordChanged");
                    message.Foreground = Th.Success;
                    oldPw.Text = newPw.Text = confirm.Text = code.Text = "";
                }
                catch (Exception e)
                {
                    message.Text = Fmt.Error(e);
                    message.Foreground = Th.Danger;
                }
                busy = false;
                Update();
            });
        });
        foreach (var f in new[] { oldPw, newPw, confirm }) f.Changed += _ => Update();
        Update();
        return Section(L.T("account.changePassword"), L.T("account.changePassword.info"),
            FormRow(L.T("account.oldPassword"), oldPw),
            FormRow(L.T("account.newPassword"), newPw),
            FormRow(L.T("account.confirmPassword"), confirm),
            totpEnabled ? FormRow(L.T("account.totpCode"), code) : null,
            K.Row(10, K.Gap(170), submit, K.Grow(message)));
    }

    private sealed class TotpState : Observable
    {
        public TotpSetup? Setup;
        public bool Enabled;
        public string Code = "";
    }

    private static FrameworkElement Totp(AppModel app)
    {
        var st = new TotpState { Enabled = app.Client.CurrentUser()?.TotpEnabled ?? false };
        var body = new Region(() =>
        {
            if (st.Setup is { } setup)
            {
                var code = K.TextField("000000", st.Code, v => st.Code = v, mono: true, width: 120, fontSize: 14);
                var confirm = K.PrimaryButton(L.T("account.totp.confirm"), () => Ui.Run(async () =>
                {
                    try
                    {
                        await app.Client.TotpVerify(st.Code.Trim());
                        st.Enabled = true;
                        st.Setup = null;
                        st.Code = "";
                        st.Notify();
                    }
                    catch (Exception e) { app.ShowError(e); }
                }));
                return K.Row(18,
                    K.Top(new Border { Background = Brushes.White, CornerRadius = new CornerRadius(8), Padding = new Thickness(4), Child = QrImage.Image(setup.OtpauthUri, 140) }),
                    K.Grow(K.Col(10,
                        K.Text(L.T("account.totp.scan"), 13, Th.Secondary, wrap: true),
                        K.SelectableText(setup.Secret, 13.5, Th.Text, mono: true),
                        K.Row(8, code, confirm, K.SecondaryButton(L.T("common.cancel"), () => { st.Setup = null; st.Notify(); })))));
            }
            var status = K.Row(6, K.Icon(st.Enabled ? "shieldCheck" : "shield", 16, st.Enabled ? Th.Success : Th.Secondary),
                K.Text(st.Enabled ? L.T("account.totp.on") : L.T("account.totp.off"), 13.5, st.Enabled ? Th.Success : Th.Secondary));
            if (st.Enabled)
            {
                var code = K.TextField(L.T("account.totp.codeToDisable"), st.Code, v => st.Code = v, width: 170);
                return K.Row(8, status, K.Spacer(), code, K.SecondaryButton(L.T("account.totp.disable"), () => Ui.Run(async () =>
                {
                    try
                    {
                        await app.Client.TotpDisable(st.Code.Trim());
                        st.Enabled = false;
                        st.Code = "";
                        st.Notify();
                    }
                    catch (Exception e) { app.ShowError(e); }
                }), destructive: true));
            }
            return K.Row(8, status, K.Spacer(), K.PrimaryButton(L.T("account.totp.enable"), () => Ui.Run(async () =>
            {
                try
                {
                    st.Setup = await app.Client.TotpSetup();
                    st.Notify();
                }
                catch (Exception e) { app.ShowError(e); }
            })));
        }).On(st);
        return Section(L.T("account.totp"), L.T("account.totp.info"), body);
    }

    // ---------- 外观 ----------

    private static FrameworkElement Appearance(AppModel app)
    {
        var s = app.Settings;
        FrameworkElement ModeTile(string mode, string title)
        {
            var sel = s.Appearance == mode;
            var preview = new Border { Width = 156, Height = 96, CornerRadius = new CornerRadius(8), ClipToBounds = true, Child = ThemePreview(mode, s.Palette) };
            var b = new Btn
            {
                CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(sel ? 2 : 1), NormalBorder = sel ? Th.Accent : Th.Border, Padding = new Thickness(2),
                Child = preview,
            };
            b.UpdateLook();
            b.Click += () => s.Appearance = mode;
            return K.Col(8, b, K.Center(K.Text(title, 12.5, Th.Text, sel ? FontWeights.SemiBold : FontWeights.Normal)));
        }
        var swatches = new StackPanel { Orientation = Orientation.Horizontal };
        foreach (var f in Palettes.Families)
        {
            var sel = s.Palette == f.Id;
            var dot = new Border { Width = 28, Height = 28, CornerRadius = new CornerRadius(14), Background = Th.Hex(f.Swatch), Child = sel ? K.Center(K.Icon("check", 13, Brushes.White, 3)) : null };
            var ring = new Border { Width = 36, Height = 36, CornerRadius = new CornerRadius(18), BorderThickness = new Thickness(sel ? 2 : 0), BorderBrush = Th.Text, Child = K.Center(dot) };
            var b = new Btn { Child = K.Col(6, K.Center(ring), K.Center(K.Text(L.T(f.NameKey), 12, sel ? Th.Text : Th.Secondary))), Margin = new Thickness(0, 0, 14, 0), Padding = new Thickness(2) };
            var id = f.Id;
            b.Click += () => s.Palette = id;
            swatches.Children.Add(b);
        }
        var langs = new List<(string, string)> { ("system", L.T("settings.language.system")) };
        langs.AddRange(L.Packs.Select(p => (p.Code, p.Name)));
        return K.PageScroll(K.Col(22,
            PageTitle(L.T("settings.page.appearance")),
            Section(L.T("settings.theme"), null, K.Row(16, ModeTile("system", L.T("settings.theme.system")), ModeTile("light", L.T("settings.theme.light")), ModeTile("dark", L.T("settings.theme.dark")))),
            Section(L.T("settings.palette"), null, swatches),
            Section(L.T("settings.language"), L.T("settings.language.info"), K.Row(8, K.Text(L.T("settings.language"), 13.5), K.Spacer(), K.Dropdown(langs, s.Language, v => s.Language = v, 180))),
            Section(L.T("settings.layout"), null,
                ToggleRow(L.T("settings.compactSidebar"), s.SidebarCompact, v => s.SidebarCompact = v),
                ToggleRow(L.T("settings.hideList"), s.ListHidden, v => s.ListHidden = v),
                ToggleRow(L.T("settings.autoHideList"), s.AutoHideList, v => s.AutoHideList = v, L.T("settings.autoHideList.info")))), 780);
    }

    /// <summary>主题缩略图：画一个迷你窗口</summary>
    private static FrameworkElement ThemePreview(string mode, string palette)
    {
        FrameworkElement Mini(Palette q)
        {
            var side = new StackPanel { Margin = new Thickness(8) };
            for (var i = 0; i < 4; i++)
                side.Children.Add(new Border { Height = 4, Width = 22, CornerRadius = new CornerRadius(2), Margin = new Thickness(0, 0, 0, 5), HorizontalAlignment = HorizontalAlignment.Left, Background = Th.B(i == 0 ? q.Accent : Palettes.A(q.SidebarText, 0.3)) });
            var main = new StackPanel { Margin = new Thickness(8) };
            for (var i = 0; i < 3; i++)
                main.Children.Add(new Border { Height = 13, CornerRadius = new CornerRadius(3), Margin = new Thickness(0, 0, 0, 5), Background = Th.B(i == 1 ? q.Selection : q.Card), BorderBrush = Th.B(q.Border), BorderThickness = new Thickness(0.5) });
            var g = new Grid();
            g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(40) });
            g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            g.Children.Add(new Border { Background = Th.B(q.Sidebar), Child = side });
            var m = new Border { Background = Th.B(q.Content), Child = main };
            Grid.SetColumn(m, 1);
            g.Children.Add(m);
            return g;
        }
        if (mode != "system") return Mini(Palettes.Make(palette, mode == "dark"));
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var a = new Border { ClipToBounds = true, Child = Mini(Palettes.Make(palette, false)) };
        var b = new Border { ClipToBounds = true, Child = Mini(Palettes.Make(palette, true)) };
        Grid.SetColumn(b, 1);
        grid.Children.Add(a);
        grid.Children.Add(b);
        return grid;
    }

    // ---------- 终端 ----------

    private static FrameworkElement Stepper(int value, int min, int max, int step, Func<int, string> format, Action<int> set)
    {
        var minus = K.IconButton("minus", null, () => set(Math.Max(min, value - step)), size: 12, w: 28, h: 28);
        var plus = K.IconButton("plus", null, () => set(Math.Min(max, value + step)), size: 12, w: 28, h: 28);
        minus.IsEnabled = value > min;
        plus.IsEnabled = value < max;
        var text = K.Text(format(value), 13, Th.Text, FontWeights.Medium);
        text.TextAlignment = TextAlignment.Center;
        text.Width = 70;
        return new Border { BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(7), Background = Th.Elevated, Child = K.Row(0, minus, text, plus) };
    }

    private static FrameworkElement TerminalPreview(TerminalTheme t, string font, int size)
    {
        var ff = TerminalFonts.Family(font);
        TextBlock T(string s, Color c) => new() { Text = s, Foreground = Th.B(c), FontFamily = ff, FontSize = size };
        var line1 = new StackPanel { Orientation = Orientation.Horizontal, Children = { T("deploy@web-01", t.Ansi[10]), T(":", t.Foreground), T("~/app", t.Ansi[12]), T("$ ls --color", t.Foreground) } };
        var line2 = new StackPanel { Orientation = Orientation.Horizontal, Children = { T("config.yaml   ", t.Foreground), T("deploy.sh   ", t.Ansi[10]), T("logs   ", t.Ansi[12]), T("backup.tar.gz", t.Ansi[1]) } };
        var swatches = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 6, 0, 0) };
        for (var i = 0; i < 16; i++) swatches.Children.Add(new Border { Width = 16, Height = 12, CornerRadius = new CornerRadius(2), Margin = new Thickness(0, 0, 3, 0), Background = Th.B(t.Ansi[i]) });
        return new Border
        {
            Background = Th.B(t.Background), CornerRadius = new CornerRadius(10), BorderBrush = Th.Border, BorderThickness = new Thickness(1), Padding = new Thickness(14),
            Child = new StackPanel { Children = { line1, K.Margin(line2, 0, 3, 0, 0), swatches } },
        };
    }

    private static FrameworkElement Terminal(AppModel app)
    {
        var s = app.Settings;
        var grid = new AdaptiveGrid { MinItem = 150, Spacing = 10 };
        foreach (var t in TerminalTheme.All)
        {
            var sel = s.TerminalTheme == t.Id;
            var dots = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 4, 0, 0) };
            foreach (var i in new[] { 1, 2, 3, 4, 5, 6 }) dots.Children.Add(new Border { Width = 12, Height = 8, CornerRadius = new CornerRadius(1.5), Margin = new Thickness(0, 0, 2, 0), Background = Th.B(t.Ansi[i]) });
            var sample = new Border
            {
                Background = Th.B(t.Background), CornerRadius = new CornerRadius(6), BorderBrush = Th.Border, BorderThickness = new Thickness(1), Padding = new Thickness(8),
                Child = new StackPanel { Children = { new TextBlock { Text = "$ ls", FontFamily = Th.MonoFont, FontSize = 10.5, Foreground = Th.B(t.Foreground) }, dots } },
            };
            var b = new Btn
            {
                Padding = new Thickness(6), CornerRadius = new CornerRadius(9), BorderThickness = new Thickness(2), NormalBorder = sel ? Th.Accent : Brushes.Transparent,
                Child = K.Col(6, sample, K.Text(t.Name, 12, sel ? Th.Text : Th.Secondary, sel ? FontWeights.SemiBold : FontWeights.Normal)),
            };
            b.UpdateLook();
            var id = t.Id;
            b.Click += () => s.TerminalTheme = id;
            grid.Children.Add(b);
        }
        var fonts = new List<(string, string)> { ("", L.T("settings.terminal.defaultFont", TerminalFonts.Default)) };
        fonts.AddRange(TerminalFonts.Families.Select(f => (f, f)));
        return K.PageScroll(K.Col(22,
            PageTitle(L.T("settings.page.terminal")),
            TerminalPreview(TerminalTheme.Named(s.TerminalTheme), s.TerminalFont, s.TerminalFontSize),
            Section(L.T("settings.terminal.theme"), L.T("settings.terminal.theme.info"), grid),
            Section(L.T("settings.terminal.font"), null,
                K.Row(8, K.Text(L.T("settings.terminal.family"), 13.5), K.Spacer(), K.Dropdown(fonts, s.TerminalFont, v => s.TerminalFont = v, 240)),
                K.Row(8, K.Text(L.T("settings.terminal.size"), 13.5), K.Spacer(), Stepper(s.TerminalFontSize, 9, 28, 1, v => v + " pt", v => s.TerminalFontSize = v)),
                ToggleRow(L.T("settings.terminal.blink"), s.TerminalCursorBlink, v => s.TerminalCursorBlink = v)),
            Section(L.T("settings.terminal.behavior"), null,
                ToggleRow(L.T("settings.terminal.copyOnSelect"), s.TerminalCopyOnSelect, v => s.TerminalCopyOnSelect = v),
                ToggleRow(L.T("settings.terminal.rightClickPaste"), s.TerminalRightClickPaste, v => s.TerminalRightClickPaste = v, L.T("settings.terminal.rightClickPaste.info")),
                K.Row(6, K.Icon("keyboard", 14), K.Grow(K.Text(L.T("settings.terminal.shortcuts"), 12.5, Th.Secondary, wrap: true))))), 780);
    }

    // ---------- 连接 ----------

    private static FrameworkElement Connection(AppModel app)
    {
        var s = app.Settings;
        return K.PageScroll(K.Col(22,
            PageTitle(L.T("settings.page.connection")),
            Section(L.T("settings.connection.display"), L.T("settings.connection.display.info"),
                K.Row(8, K.Text(L.T("remote.quality"), 13.5), K.Spacer(), K.Segmented(new[]
                {
                    ("smooth", L.T("remote.quality.smooth")), ("balanced", L.T("remote.quality.balanced")), ("sharp", L.T("remote.quality.sharp")), ("custom", L.T("remote.quality.custom")),
                }, s.DisplayQuality, q =>
                {
                    var (fps, kbps) = AppSettings.Quality(q);
                    s.DisplayQuality = q;
                    if (q != "custom") { s.DisplayFps = fps; s.DisplayBitrateKbps = kbps; }
                })),
                K.Row(8, K.Text(L.T("remote.fps"), 13.5), K.Spacer(), K.Dropdown(new[] { 30, 60, 90, 120, 144 }.Select(f => (f, f + " fps")), s.DisplayFps, v => { s.DisplayFps = v; s.DisplayQuality = "custom"; }, 120)),
                K.Row(8, K.Text(L.T("remote.bitrate"), 13.5), K.Spacer(), Stepper(s.DisplayBitrateKbps, 2000, 50000, 2000, v => (v / 1000).ToString(CultureInfo.InvariantCulture) + " Mbps",
                    v => { s.DisplayBitrateKbps = v; s.DisplayQuality = "custom"; })),
                K.Row(6, K.Icon("info", 14), K.Grow(K.Text(L.T("settings.connection.h264only"), 12.5, Th.Secondary, wrap: true))),
                ToggleRow(L.T("remote.clipboard"), s.ClipboardSync, v => s.ClipboardSync = v)),
            Section(L.T("settings.connection.terminal"), null,
                ToggleRow(L.T("settings.connection.reattach"), s.ReattachTerminals, v => s.ReattachTerminals = v, L.T("settings.connection.reattach.info")))), 780);
    }

    // ---------- AI ----------

    private static FrameworkElement Ai(AppModel app)
    {
        var s = app.Settings;
        return K.PageScroll(K.Col(22,
            PageTitle(L.T("settings.page.ai")),
            Section(L.T("settings.ai.defaultMode"), L.T("ai.mode.info"), AiViews.ModePicker(s.AiMode, AiMode.FullTrust, m => s.AiMode = m)),
            Section(L.T("settings.ai.behavior"), null, ToggleRow(L.T("ai.planFirst"), s.AiPlanFirst, v => s.AiPlanFirst = v, L.T("ai.planFirst.info")))), 780);
    }

    // ---------- 关于 ----------

    private static FrameworkElement UpdateSection(AppModel app)
    {
        var u = app.Updater;
        var body = new Region(() =>
        {
            var check = K.SecondaryButton(L.T("update.check"), u.Check, icon: "refresh", enabled: !u.Busy);
            switch (u.State)
            {
                case UpdateState.Checking:
                    return K.Row(8, K.Spinner(14), K.Text(L.T("update.checking"), 13, Th.Secondary), K.Spacer(), check);
                case UpdateState.UpToDate:
                    return K.Row(8, K.Icon("checkCircle", 15, Th.Success), K.Text(L.T("update.upToDate", AppInfo.Version), 13, Th.Text), K.Spacer(), check);
                case UpdateState.Failed when u.Info == null:
                    return K.Row(8, K.Icon("alertCircle", 15, Th.Danger), K.Grow(K.Text(u.Error ?? "", 13, Th.Danger, wrap: true)), check);
                case UpdateState.Available or UpdateState.Downloading or UpdateState.Downloaded or UpdateState.Installing or UpdateState.Failed when u.Info is { } info:
                    // 「立即更新」一路自动：没下载好先下载，下载好就退出、替换、重新打开
                    var action = u.State switch
                    {
                        UpdateState.Installing => null,
                        UpdateState.Downloading when u.InstallRequested => null,
                        _ => K.PrimaryButton(L.T("update.now"), u.InstallNow, icon: "refresh"),
                    };
                    var sub = u.State switch
                    {
                        UpdateState.Downloaded => L.T("update.ready", Fmt.Bytes(info.Size)),
                        _ => $"{info.FileName} · {Fmt.Bytes(info.Size)}",
                    };
                    var col = new List<UIElement?>
                    {
                        K.Row(8, K.Icon("sparkles", 15, Th.Accent), K.Grow(K.Col(2, K.Semi(L.T("update.available", info.Version), 13.5), K.Text(sub, 12, Th.Tertiary))), action),
                    };
                    if (u.State == UpdateState.Downloading)
                        col.Add(K.Col(4, K.Progress(u.Total == 0 ? 0 : (double)u.Done / u.Total, Th.Accent, 5),
                            K.Text((u.InstallRequested ? L.T("update.downloadingInstall") + " · " : "") + $"{Fmt.Bytes(u.Done)} / {Fmt.Bytes(u.Total)}", 11.5, Th.Tertiary)));
                    if (u.State == UpdateState.Installing)
                        col.Add(K.Row(8, K.Spinner(14), K.Grow(K.Text(L.T("update.installing"), 12.5, Th.Secondary, wrap: true))));
                    if (u.State == UpdateState.Failed && u.Error is { Length: > 0 } err)
                        col.Add(K.Row(8, K.Icon("alertCircle", 14, Th.Danger), K.Grow(K.Text(err, 12.5, Th.Danger, wrap: true))));
                    else if (u.LastFailure is { } last && last.Sha256 == info.Sha256 && u.State != UpdateState.Installing)
                        col.Add(K.Row(8, K.Icon("alertCircle", 14, Th.Warning), K.Grow(K.Text(L.T("update.lastFailed", L.T("update.fail." + (last.Code.Length > 0 ? last.Code : "other"))), 12.5, Th.Secondary, wrap: true))));
                    if (info.Notes.Trim().Length > 0) col.Add(K.Box(K.Text(info.Notes.Trim(), 12.5, Th.Secondary, wrap: true), Th.CodeBg, 7, null, 0, new Thickness(10, 8, 10, 8)));
                    return K.Col(10, col);
                default:
                    return K.Row(8, K.Text(L.T("update.current", AppInfo.Version), 13, Th.Text), K.Spacer(), check);
            }
        }).On(u, u.Progress);
        return Section(L.T("update.title"), L.T("update.info"), body);
    }

    private static readonly (string name, string desc, string file)[] Licenses =
    {
        ("LemonGalaxy", "MIT · github.com/LemonITCN/lemon-galaxy", "LemonGalaxy-LICENSE.txt"),
    };

    private static FrameworkElement About(AppModel app)
    {
        var license = new Region(() => null);
        string? shown = null;
        var rows = new List<UIElement?>();
        foreach (var (name, desc, file) in Licenses)
        {
            var f = file;
            rows.Add(K.Row(8, K.Grow(K.Col(2, K.Text(name, 13.5, Th.Text, FontWeights.Medium), K.Text(desc, 12, Th.Tertiary))), K.SecondaryButton(L.T("about.viewLicense"), () =>
            {
                shown = shown == f ? null : f;
                string? text = null;
                if (shown != null) try { text = File.ReadAllText(IOPath.Combine(AppInfo.ResourcesDir, "Licenses", shown)); } catch { }
                license.Child = text == null ? null : K.Box(K.SelectableText(text, 11.5, Th.Secondary, mono: true), Th.Elevated, 6, null, 0, new Thickness(10));
            })));
        }
        rows.Add(license);
        var core = new List<UIElement?> { K.Text(L.T("about.core", app.Client.CoreVersion), 12.5, Th.Tertiary) };
        if (app.Client.IsMock)
        {
            core.Add(K.Badge(L.T("about.mock"), Th.P.Warning));
            core.Add(K.Info(L.T("about.mock.info") + (app.CoreFallbackReason != null ? "\n\n" + L.T("about.mock.fallback", app.CoreFallbackReason) : "")));
        }
        return K.PageScroll(K.Col(22,
            K.Row(16, K.AppMark(64), K.Col(4, K.Semi("LemonGalaxy", 22), K.Text(L.T("about.version", AppInfo.Version, AppInfo.Build), 13, Th.Secondary), K.Row(6, core))),
            UpdateSection(app),
            Section(L.T("about.licenses"), null, rows.ToArray()),
            K.Text(L.T("about.system", AppInfo.OsVersion, Environment.Is64BitProcess ? "x64" : "x86", Environment.Version.ToString()), 12, Th.Tertiary),
            K.Text("Copyright © 2026 LemonIT.CN · MIT License", 12, Th.Tertiary)), 780);
    }
}
