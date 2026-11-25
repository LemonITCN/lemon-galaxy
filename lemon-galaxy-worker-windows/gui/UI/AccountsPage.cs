using System.Windows;
using System.Windows.Controls;
using LemonGalaxy.Ipc;

namespace LemonGalaxy.UI;

internal static partial class Pages
{
    // ---------------- 账号与权限 ----------------

    private static FrameworkElement AccountsPage(AppModel app)
        => new Region(() =>
        {
            var r = app.Rights;
            if (!r.ViewConfig) return W.Placeholder("lock", L.T("denied.title"), L.T("denied.accounts"));
            var add = W.Primary(L.T("accounts.signIn"), () => Login.Open(app), shield: r.NeedsElevation, enabled: r.CanManage);
            var list = app.Accounts;
            var rows = list == null
                ? new List<UIElement?> { K.Pad(W.Busy(L.T("common.loading")), 14) }
                : list.Count == 0
                    ? new List<UIElement?> { K.Pad(K.Text(L.T("accounts.empty"), 12.5, Th.Tertiary), 14, 14, 14, 14) }
                    : list.Select(a => (UIElement?)AccountRow(app, a, r.CanManage)).ToList();
            var server = app.Status?.Server ?? "";
            return Frame(
                W.Title(L.T("page.accounts"), L.T("accounts.info"), add),
                W.Rows(rows),
                server.Length == 0 ? null : K.Col(8, W.Section(L.T("accounts.server")),
                    W.Rows(W.Row(ServerHost(server), W.CopyButton(() => server), L.T("accounts.server.info")))));
        }).On(app.StatusChanged, app.AccountsChanged).Keyed(() => (app.Accounts, app.Rights, app.Status?.Server));

    private static FrameworkElement AccountRow(AppModel app, Account a, bool manage)
    {
        var sub = a.Active ? L.T("accounts.runsAs", a.Policy.SystemUser, Fmt.AiMode(a.Policy.AiCap)) : L.T("accounts.needsRelogin");
        var icon = new Grid { Width = 30, Height = 30, Children = { K.Avatar(a.DisplayName.Length > 0 ? a.DisplayName : a.Username, 30) } };
        var buttons = K.Row(8,
            a.Active ? null : W.Secondary(L.T("accounts.relogin"), () => Login.Open(app, a.Username), shield: app.Rights.NeedsElevation, enabled: manage),
            K.SecondaryButton(L.T("accounts.permissions"), () => PolicyDialog.Open(app, a)),
            K.SecondaryButton(L.T("accounts.signOut"), () => Ui.Run(async () =>
            {
                if (!await app.Confirm(L.T("accounts.signOut.title", a.Username), L.T("accounts.signOut.body"), L.T("accounts.signOut"), true)) return;
                if (await app.Do(Req.Logout(a.UserId)) != null) app.ShowToast(L.T("accounts.signedOut", a.Username));
            }), destructive: true, enabled: manage));
        return W.Row(a.Title, buttons, subtitle: sub, icon: icon, subtitleColor: a.Active ? null : Th.Warning);
    }
}

/// <summary>账号在本机的权限（弹层）</summary>
internal static class PolicyDialog
{
    public static void Open(AppModel app, Account a)
    {
        var manage = app.Rights.CanManage;
        var p = a.Policy;
        var view = p.View;
        var control = p.Control;
        var files = p.Files;
        var terminal = p.Terminal;
        var clipboard = p.Clipboard;
        var audio = p.Audio;
        var notify = p.Notify;
        var ai = p.AiCap;
        var user = p.SystemUser;
        var users = new List<string> { user };
        var canChoose = false;
        FrameworkElement? card = null;
        var saving = false;
        var usersBox = new Border();

        void RenderUsers()
        {
            var dd = K.Dropdown(users.Select(u => (u, u)), user, v => user = v, minWidth: 200);
            dd.IsEnabled = manage && canChoose;
            usersBox.Child = dd;
        }
        RenderUsers();

        FrameworkElement Sw(string key, bool on, Action<bool> set, string? info = null)
        {
            var t = K.Switch(on, set);
            t.IsEnabled = manage;
            return W.Row(L.T(key), t, info);
        }

        var aiDd = K.Dropdown(Fmt.AiModes.Select(m => (m, Fmt.AiMode(m))), ai, v => ai = v, minWidth: 180);
        aiDd.IsEnabled = manage;
        var error = K.Text("", 12, Th.Danger, wrap: true);
        error.Visibility = Visibility.Collapsed;
        var spinner = K.Spinner(14);
        spinner.Visibility = Visibility.Collapsed;

        void Save() => Ui.Run(async () =>
        {
            if (saving) return;
            saving = true;
            spinner.Visibility = Visibility.Visible;
            try
            {
                var np = new Policy(user, view, control, files, terminal, clipboard, audio, ai, notify);
                var confirmRoot = false;
                while (true)
                {
                    var r = await app.Call(Req.SetPolicy(a.UserId, np, confirmRoot));
                    if (!r.Ok && r.Code == ErrorCode.ConfirmRoot && !confirmRoot)
                    {
                        if (!await app.Confirm(L.T("root.title"), r.Message, L.T("root.confirm"), true)) return;
                        confirmRoot = true;
                        continue;
                    }
                    if (!r.Ok)
                    {
                        error.Text = r.Message;
                        error.Visibility = Visibility.Visible;
                        return;
                    }
                    app.ShowToast(L.T("policy.saved"));
                    app.CloseModal(card);
                    return;
                }
            }
            finally
            {
                saving = false;
                spinner.Visibility = Visibility.Collapsed;
            }
        });

        var body = K.Col(14,
            Dialogs.Header(L.T("policy.title"), a.Title, "shield", close: () => app.CloseModal(card)),
            W.Section(L.T("policy.runAs")),
            W.Rows(W.Row(L.T("policy.systemUser"), usersBox, L.T("policy.systemUser.info"))),
            W.Section(L.T("policy.allow")),
            W.Rows(
                Sw("policy.view", view, v => view = v),
                Sw("policy.control", control, v => control = v),
                Sw("policy.files", files, v => files = v),
                Sw("policy.terminal", terminal, v => terminal = v),
                Sw("policy.clipboard", clipboard, v => clipboard = v),
                Sw("policy.audio", audio, v => audio = v),
                Sw("policy.notify", notify, v => notify = v, L.T("policy.notify.info"))),
            W.Section("AI"),
            W.Rows(W.Row(L.T("policy.aiCap"), aiDd, L.T("policy.aiCap.info"))),
            error,
            Dialogs.Buttons(spinner, K.SecondaryButton(L.T("common.cancel"), () => app.CloseModal(card)), W.Primary(L.T("common.save"), Save, shield: app.Rights.NeedsElevation, enabled: manage)));
        var scroll = K.Scroll(body);
        scroll.MaxHeight = 620;
        card = Dialogs.Card(scroll, 520);
        app.ShowModal(card);

        // 可选的系统用户
        Ui.Run(async () =>
        {
            if (app.Client == null) return;
            var r = await app.Client.Call(Req.SystemUsers());
            if (r.Type != "system_users") return;
            var all = r.J.AS("users");
            if (!all.Any(u => string.Equals(u, user, StringComparison.OrdinalIgnoreCase))) all.Insert(0, user);
            users = all;
            canChoose = r.J.B("can_choose");
            RenderUsers();
        });
    }
}

/// <summary>登录向导（弹层）：服务器、用户名、密码 → 两步验证码 → 首次登录设新密码 → 选系统用户（SYSTEM 二次确认）。</summary>
internal sealed class Login
{
    private enum Step { Creds, Totp, NewPassword, SystemUser }

    private readonly AppModel app;
    private Step step = Step.Creds;
    private bool busy, replaceServer;
    private readonly Field server, user, pass, totp, new1, new2;
    private readonly Border stepBox = new(), sysBox = new();
    private readonly TextBlock error = K.Text("", 12, Th.Danger, wrap: true);
    private readonly TextBlock hint = K.Text("", 12.5, Th.Secondary, wrap: true);
    private readonly FrameworkElement spinner = K.Spinner(14);
    private readonly Border nextBox = new();
    private string sysUser = "";
    private List<string> sysUsers = new();
    private bool canChoose;
    private FrameworkElement? card;

    public static void Open(AppModel app, string? username = null) => new Login(app, username).Show();

    private Login(AppModel app, string? username)
    {
        this.app = app;
        server = K.TextField(L.T("login.server.placeholder"), app.Status?.Server ?? "", mono: true);
        user = K.TextField(L.T("login.username"), username ?? "");
        pass = K.PasswordField(L.T("login.password"));
        totp = K.TextField("123456", "", mono: true);
        new1 = K.PasswordField(L.T("login.newPassword"));
        new2 = K.PasswordField(L.T("login.confirm"));
        foreach (var f in new[] { server, user, pass, totp, new1, new2 }) f.Submitted += Submit;
        error.Visibility = Visibility.Collapsed;
        spinner.Visibility = Visibility.Collapsed;
    }

    private static FrameworkElement Field(string label, UIElement input, string? info = null)
    {
        var l = K.Text(label, 12.5, Th.Secondary);
        l.Width = 86;
        return K.Row(10, l, K.Grow(input), info == null ? K.Gap(18) : K.Info(info));
    }

    private void Show()
    {
        Render();
        var body = K.Col(16,
            Dialogs.Header(L.T("login.title"), null, "user", close: Close),
            stepBox, error,
            Dialogs.Buttons(spinner, K.SecondaryButton(L.T("common.cancel"), Close), nextBox));
        card = Dialogs.Card(body, 500);
        app.ShowModal(card);
        Ui.After(50, () => (server.Text.Length == 0 ? server : user.Text.Length == 0 ? user : pass).FocusInput());
    }

    private void Render()
    {
        stepBox.Child = step switch
        {
            Step.Creds => K.Col(10,
                Field(L.T("login.server"), server, L.T("login.server.info")),
                Field(L.T("login.username"), user),
                Field(L.T("login.password"), pass)),
            Step.Totp => K.Col(12, hint, Field(L.T("login.totp"), totp)),
            Step.NewPassword => K.Col(12, hint, Field(L.T("login.newPassword"), new1, L.T("login.newPassword.info")), Field(L.T("login.confirm"), new2)),
            _ => K.Col(12, hint, Field(L.T("login.systemUser"), sysBox, L.T("policy.systemUser.info"))),
        };
        var title = step switch
        {
            Step.Creds => L.T("login.signIn"),
            Step.Totp => L.T("login.verify"),
            Step.NewPassword => L.T("login.setAndSignIn"),
            _ => L.T("login.done"),
        };
        var b = W.Primary(title, Submit, shield: app.Rights.NeedsElevation);
        b.IsEnabled = !busy;
        nextBox.Child = b;
    }

    private void Go(Step s)
    {
        step = s;
        ShowError("");
        Render();
        Ui.After(30, () =>
        {
            switch (s)
            {
                case Step.Creds: pass.FocusInput(); break;
                case Step.Totp: totp.FocusInput(); break;
                case Step.NewPassword: new1.FocusInput(); break;
            }
        });
    }

    private void ShowError(string m)
    {
        error.Text = m;
        error.Visibility = m.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
    }

    private void SetBusy(bool b)
    {
        busy = b;
        spinner.Visibility = b ? Visibility.Visible : Visibility.Collapsed;
        if (nextBox.Child is UIElement e) e.IsEnabled = !b;
    }

    private void Close()
    {
        app.CloseModal(card);
        if (app.Client != null) _ = app.Client.Call(Req.LoginCancel());
    }

    private void Submit()
    {
        if (busy) return;
        Dictionary<string, object?> req;
        switch (step)
        {
            case Step.Creds or Step.Totp:
                if (user.Text.Trim().Length == 0 || pass.Text.Length == 0)
                {
                    ShowError(L.T("login.missing"));
                    return;
                }
                req = Req.Login(server.Text.Trim(), user.Text.Trim(), pass.Text, step == Step.Totp ? totp.Text.Trim() : null, replaceServer);
                break;
            case Step.NewPassword:
                if (new1.Text.Length < 8) { ShowError(L.T("login.tooShort")); return; }
                if (new1.Text != new2.Text) { ShowError(L.T("login.mismatch")); return; }
                req = Req.LoginSetPassword(new1.Text);
                break;
            default:
                if (sysUser.Length == 0) return;
                req = Req.LoginFinish(sysUser, false);
                break;
        }
        Ui.Run(() => Send(req));
    }

    private async Task Send(Dictionary<string, object?> req)
    {
        SetBusy(true);
        try
        {
            while (true)
            {
                var r = await app.Call(req);
                if (r.Type == "login")
                {
                    var st = r.J.O("step");
                    switch (st?.S("kind"))
                    {
                        case "need_totp":
                            hint.Text = L.T("login.totp.hint", user.Text.Trim());
                            Go(Step.Totp);
                            return;
                        case "confirm_server_change":
                            if (await app.Confirm(L.T("login.switch.title"), L.T("login.switch.body", st.L("accounts"), ServerHostOf(st.S("current_server"))), L.T("login.switch.confirm"), true))
                            {
                                replaceServer = true;
                                req["replace_server"] = true;
                                continue;
                            }
                            return;
                        case "need_new_password":
                            hint.Text = L.T("login.newPassword.hint", st.S("username"));
                            Go(Step.NewPassword);
                            return;
                        case "choose_system_user":
                            sysUsers = st.AS("users");
                            canChoose = st.B("can_choose");
                            var def = st.S("default_user");
                            sysUser = sysUsers.FirstOrDefault(u => string.Equals(u, def, StringComparison.OrdinalIgnoreCase)) ?? (sysUsers.Count > 0 ? sysUsers[0] : def);
                            if (!sysUsers.Contains(sysUser)) sysUsers.Insert(0, sysUser);
                            var dd = K.Dropdown(sysUsers.Select(u => (u, u)), sysUser, v => sysUser = v, minWidth: 220);
                            dd.IsEnabled = canChoose;
                            sysBox.Child = dd;
                            var name = st.S("display_name");
                            var who = name.Length == 0 ? st.S("username") : $"{name}（{st.S("username")}）";
                            hint.Text = canChoose ? L.T("login.sysuser.hint", who) : L.T("login.sysuser.fixed", who, def);
                            Go(Step.SystemUser);
                            return;
                        case "done":
                            app.ShowToast(L.T("login.signedIn", st.S("username")));
                            app.CloseModal(card);
                            return;
                    }
                    return;
                }
                if (r.Ok) return;
                if (r.Code == ErrorCode.ConfirmRoot && req["op"] as string == "login_finish" && !(req["confirm_root"] is true))
                {
                    if (await app.Confirm(L.T("root.title"), r.Message, L.T("root.confirm"), true))
                    {
                        req["confirm_root"] = true;
                        continue;
                    }
                    return;
                }
                // 登录失效：回第一步；密码错：清掉密码
                if (r.Code == ErrorCode.Invalid && req["op"] as string != "login" && (r.Message.Contains("重新登录") || r.Message.Contains("start again"))) Go(Step.Creds);
                if (r.Code == ErrorCode.InvalidCredentials)
                {
                    Go(Step.Creds);
                    pass.Text = "";
                }
                ShowError(r.Message);
                return;
            }
        }
        finally
        {
            SetBusy(false);
        }
    }

    private static string ServerHostOf(string s)
    {
        var i = s.IndexOf('#');
        return i > 0 ? s.Substring(0, i) : s;
    }
}
