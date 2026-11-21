using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace LemonGalaxy.UI;

internal enum DraftKind { Ssh, Rdp, Credential, Folder, Device, Snippet }

/// <summary>编辑面板的草稿：所有条目类型共用一份字段，保存时按类型组装 VaultData。</summary>
internal sealed class EntryDraft : Observable
{
    public DraftKind Kind;
    public string Id = "", DeviceId = "";
    public string? FolderId;
    public string Label = "", Host = "", Port = "", Username = "", AuthKind = "password", Password = "", CredentialId = "",
        JumpDeviceId = "", JumpHostId = "", Domain = "", Tags = "", Note = "", TerminalTheme = "", PrivateKey = "", Passphrase = "", Color = "", Command = "";
    public bool Favorite, Expanded = true;
    public long Sort;

    public static EntryDraft From(EditTarget target, AppModel app)
    {
        switch (target)
        {
            case EditTarget.NewSsh n: return new EntryDraft { Kind = DraftKind.Ssh, FolderId = n.Folder, Port = "22" };
            case EditTarget.NewRdp n: return new EntryDraft { Kind = DraftKind.Rdp, FolderId = n.Folder, Port = "3389" };
            case EditTarget.NewCredential: return new EntryDraft { Kind = DraftKind.Credential };
            case EditTarget.NewSnippet: return new EntryDraft { Kind = DraftKind.Snippet };
            case EditTarget.Device dev:
            {
                var d = new EntryDraft { Kind = DraftKind.Device, DeviceId = dev.Id };
                if (app.MetaFor(dev.Id) is { Data: VaultData.DeviceMeta m } me)
                {
                    d.Id = me.Id; d.FolderId = me.FolderId; d.Sort = me.Sort;
                    d.Label = m.Alias ?? ""; d.Tags = string.Join(", ", m.Tags); d.Note = m.Note; d.Favorite = m.Favorite;
                }
                return d;
            }
            case EditTarget.Vault v:
            {
                var e = app.Entry(v.Id);
                if (e == null) return new EntryDraft { Kind = DraftKind.Ssh };
                EntryDraft d;
                switch (e.Data)
                {
                    case VaultData.SshHost h:
                        d = new EntryDraft
                        {
                            Kind = DraftKind.Ssh, Label = h.Label, Host = h.Host, Port = h.Port.ToString(), Username = h.Username,
                            JumpDeviceId = h.JumpDeviceId ?? "", JumpHostId = h.JumpHostId ?? "", Tags = string.Join(", ", h.Tags), Note = h.Note,
                            Favorite = h.Favorite, TerminalTheme = h.TerminalTheme ?? "",
                        };
                        switch (h.Auth)
                        {
                            case SshAuth.Password pw: d.AuthKind = "password"; d.Password = pw.PasswordValue ?? ""; break;
                            case SshAuth.PrivateKey pk: d.AuthKind = "key"; d.CredentialId = pk.CredentialId; break;
                            case SshAuth.Agent: d.AuthKind = "agent"; break;
                            default: d.AuthKind = "interactive"; break;
                        }
                        break;
                    case VaultData.RdpHost r:
                        d = new EntryDraft
                        {
                            Kind = DraftKind.Rdp, Label = r.Label, Host = r.Host, Port = r.Port.ToString(), Username = r.Username, Password = r.Password ?? "",
                            Domain = r.Domain ?? "", JumpDeviceId = r.JumpDeviceId ?? "", Tags = string.Join(", ", r.Tags), Note = r.Note, Favorite = r.Favorite,
                        };
                        break;
                    case VaultData.Credential c:
                        d = new EntryDraft { Kind = DraftKind.Credential, Label = c.Label, Username = c.Username, Password = c.Password ?? "", PrivateKey = c.PrivateKey ?? "", Passphrase = c.Passphrase ?? "" };
                        break;
                    case VaultData.Folder f:
                        d = new EntryDraft { Kind = DraftKind.Folder, Label = f.Name, Color = f.Color ?? "", Expanded = f.Expanded };
                        break;
                    case VaultData.Snippet s:
                        d = new EntryDraft { Kind = DraftKind.Snippet, Label = s.Label, Command = s.Command, Tags = string.Join(", ", s.Tags) };
                        break;
                    default:
                        d = new EntryDraft { Kind = DraftKind.Ssh };
                        break;
                }
                d.Id = e.Id;
                d.FolderId = e.FolderId;
                d.Sort = e.Sort;
                return d;
            }
        }
        return new EntryDraft();
    }

    public string[] TagList => Tags.Split(new[] { ',', '，', ' ' }, StringSplitOptions.RemoveEmptyEntries).Select(t => t.Trim()).Where(t => t.Length > 0).Distinct().ToArray();

    public ushort? PortValue => ushort.TryParse(Port.Trim(), out var p) && p != 0 ? p : null;

    public bool IsValid => Kind switch
    {
        DraftKind.Ssh => Label.Trim().Length > 0 && Host.Trim().Length > 0 && PortValue != null && Username.Trim().Length > 0 && (AuthKind != "key" || CredentialId.Length > 0),
        DraftKind.Rdp => Label.Trim().Length > 0 && Host.Trim().Length > 0 && PortValue != null && Username.Trim().Length > 0,
        DraftKind.Credential or DraftKind.Folder => Label.Trim().Length > 0,
        DraftKind.Snippet => Label.Trim().Length > 0 && Command.Trim().Length > 0,
        _ => true,
    };

    public VaultEntry Build()
    {
        string? Opt(string s) => s.Trim().Length == 0 ? null : s.Trim();
        VaultData data = Kind switch
        {
            DraftKind.Ssh => new VaultData.SshHost(Label.Trim(), Host.Trim(), PortValue ?? 22, Username.Trim(), AuthKind switch
            {
                "key" => new SshAuth.PrivateKey(CredentialId),
                "agent" => new SshAuth.Agent(),
                "interactive" => new SshAuth.Interactive(),
                _ => new SshAuth.Password(Password.Length == 0 ? null : Password),
            }, Opt(JumpDeviceId), Opt(JumpHostId), TagList, Note, Favorite, Opt(TerminalTheme)),
            DraftKind.Rdp => new VaultData.RdpHost(Label.Trim(), Host.Trim(), PortValue ?? 3389, Username.Trim(), Password.Length == 0 ? null : Password, Opt(Domain), Opt(JumpDeviceId), TagList, Note, Favorite),
            DraftKind.Credential => new VaultData.Credential(Label.Trim(), Username.Trim(), Password.Length == 0 ? null : Password, PrivateKey.Trim().Length == 0 ? null : PrivateKey, Passphrase.Length == 0 ? null : Passphrase),
            DraftKind.Folder => new VaultData.Folder(Label.Trim(), Opt(Color), Expanded),
            DraftKind.Device => new VaultData.DeviceMeta(DeviceId, Opt(Label), TagList, Note, Favorite),
            _ => new VaultData.Snippet(Label.Trim(), Command, TagList),
        };
        return new VaultEntry(Id, FolderId, Sort, data, 0);
    }
}

/// <summary>右侧编辑面板（SSH / RDP 主机、凭据、文件夹、设备、片段）</summary>
internal static class Editors
{
    private static readonly string[] FolderColors = { "", "#3B82F6", "#10B981", "#F59E0B", "#EF4444", "#8B5CF6", "#EC4899", "#64748B" };

    public static FrameworkElement Panel(AppModel app, EditTarget target)
    {
        var d = EntryDraft.From(target, app);
        var title = target switch
        {
            EditTarget.NewSsh => L.T("editor.newSsh"),
            EditTarget.NewRdp => L.T("editor.newRdp"),
            EditTarget.NewCredential => L.T("editor.newCredential"),
            EditTarget.NewSnippet => L.T("editor.newSnippet"),
            EditTarget.Device => L.T("editor.device"),
            _ => d.Kind switch
            {
                DraftKind.Ssh => L.T("editor.ssh"),
                DraftKind.Rdp => L.T("editor.rdp"),
                DraftKind.Credential => L.T("editor.credential"),
                DraftKind.Folder => L.T("editor.folder"),
                DraftKind.Snippet => L.T("editor.snippet"),
                _ => L.T("editor.device"),
            },
        };
        var fields = d.Kind switch
        {
            DraftKind.Ssh => SshFields(app, d),
            DraftKind.Rdp => RdpFields(app, d),
            DraftKind.Credential => CredentialFields(app, d),
            DraftKind.Folder => FolderFields(app, d),
            DraftKind.Device => DeviceFields(app, d),
            _ => SnippetFields(d),
        };
        var saving = false;
        Btn? save = null;
        void UpdateSave() { if (save != null) save.IsEnabled = d.IsValid && !saving; }
        save = K.PrimaryButton(L.T("common.save"), () =>
        {
            if (!d.IsValid || saving) return;
            saving = true;
            UpdateSave();
            Ui.Run(async () =>
            {
                var saved = await app.Save(d.Build());
                if (saved != null)
                {
                    if (d.Kind == DraftKind.Snippet) app.SelectedSnippet = saved.Id;
                    else if (d.Kind is not (DraftKind.Device or DraftKind.Credential)) app.Selection = saved.Id;
                    app.Editing = null;
                    app.Nav.Notify();
                    app.ShowToast(L.T("common.saved"));
                }
                saving = false;
                UpdateSave();
            });
        });
        d.Watch(save, UpdateSave);
        UpdateSave();
        var footer = new List<UIElement?>();
        if (d.Id.Length > 0 && d.Kind != DraftKind.Device)
            footer.Add(K.SecondaryButton(L.T("common.delete"), () =>
            {
                var id = d.Id;
                app.Confirm(L.T("hosts.delete.title"), L.T("hosts.delete.message", d.Label), L.T("common.delete"), () => Ui.Run(async () =>
                {
                    await app.Delete(id);
                    app.Editing = null;
                    app.Nav.Notify();
                }));
            }, destructive: true));
        footer.Add(K.Spacer());
        footer.Add(K.SecondaryButton(L.T("common.cancel"), () => { app.Editing = null; app.Nav.Notify(); }));
        footer.Add(save);

        var dock = new DockPanel();
        var head = new Border
        {
            Height = 46, Padding = new Thickness(16, 0, 10, 0), BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 0, 0, 1),
            Child = K.Row(6, K.Grow(K.Semi(title, 14.5)), K.IconButton("close", L.T("common.close"), () => { app.Editing = null; app.Nav.Notify(); }, size: 12)),
        };
        DockPanel.SetDock(head, Dock.Top);
        dock.Children.Add(head);
        var foot = new Border { Padding = new Thickness(12), BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 1, 0, 0), Child = K.Row(8, footer) };
        DockPanel.SetDock(foot, Dock.Bottom);
        dock.Children.Add(foot);
        dock.Children.Add(K.Scroll(K.Pad(fields, 16)));
        return dock;
    }

    private static FrameworkElement Labeled(string label, UIElement input, string? info = null)
        => K.Col(5, K.Row(4, K.Text(label, 12, Th.Secondary, FontWeights.Medium), info == null ? null : K.Info(info)), input);

    private static Field Text(EntryDraft d, string value, Action<string> set, string placeholder = "", bool mono = false)
    {
        var f = K.TextField(placeholder, value, v => { set(v); d.Notify(); }, mono);
        return f;
    }

    private static FrameworkElement FolderPicker(AppModel app, EntryDraft d)
    {
        var items = new List<(string? value, string label)> { (null, L.T("hosts.root")) };
        items.AddRange(app.Nodes().Where(n => n.Kind == NodeKind.Folder && n.Id != d.Id).Select(n => ((string?)n.Id, n.Title)));
        return Labeled(L.T("editor.folder"), K.Dropdown(items, d.FolderId, v => { d.FolderId = v; d.Notify(); }));
    }

    private static List<DeviceInfo> JumpDevices(AppModel app) => app.Devices.Where(x => !x.IsSelf && x.Kind != DeviceKind.Mobile).ToList();

    private static FrameworkElement SshFields(AppModel app, EntryDraft d)
    {
        var port = Text(d, d.Port, v => d.Port = v, "22");
        port.Width = 76;
        port.Changed += _ => port.Invalid = d.Port.Length > 0 && d.PortValue == null;
        var authArea = new Region(() =>
        {
            switch (d.AuthKind)
            {
                case "password":
                    var pw = K.PasswordField(L.T("ssh.password.ask"), d.Password, v => { d.Password = v; d.Notify(); });
                    return Labeled(L.T("ssh.password"), pw, L.T("ssh.password.info"));
                case "key":
                    var creds = new List<(string, string)> { ("", L.T("ssh.credential.choose")) };
                    creds.AddRange(app.Credentials.Select(c => (c.Id, c.Data is VaultData.Credential cc ? cc.Label : c.Id)));
                    return Labeled(L.T("ssh.credential"), K.Row(8,
                        K.Grow(K.Dropdown(creds, d.CredentialId, v => { d.CredentialId = v; d.Notify(); })),
                        K.SecondaryButton(L.T("ssh.credential.new"), () => { app.HostsTab = HostsListTab.Credentials; app.Editing = new EditTarget.NewCredential(); app.Nav.Notify(); })));
                case "agent":
                    return K.Row(6, K.Icon("info", 14, Th.Warning), K.Grow(K.Text(L.T("ssh.agent.unsupported"), 12.5, Th.Secondary, wrap: true)));
                default:
                    return null;
            }
        }).Keyed(() => d.AuthKind).On(d);
        var jumpItems = new List<(string, string)> { ("", L.T("ssh.jump.none")) };
        jumpItems.AddRange(JumpDevices(app).Select(x => ("dev:" + x.Id, app.DeviceName(x.Id))));
        jumpItems.AddRange(app.SshHosts.Where(e => e.Id != d.Id).Select(e => ("ssh:" + e.Id, "SSH · " + app.SshLabel(e.Id))));
        var jumpCurrent = d.JumpDeviceId.Length > 0 ? "dev:" + d.JumpDeviceId : d.JumpHostId.Length > 0 ? "ssh:" + d.JumpHostId : "";
        var themes = new List<(string, string)> { ("", L.T("ssh.theme.default")) };
        themes.AddRange(TerminalTheme.All.Select(t => (t.Id, t.Name)));
        return K.Col(14,
            Labeled(L.T("editor.label"), Text(d, d.Label, v => d.Label = v, "web-01")),
            K.Row(8, K.Grow(Labeled(L.T("ssh.host"), Text(d, d.Host, v => d.Host = v, "10.0.0.5"))), Labeled(L.T("ssh.port"), port)),
            Labeled(L.T("ssh.username"), Text(d, d.Username, v => d.Username = v, "root")),
            Labeled(L.T("ssh.auth"), K.Dropdown(new[]
            {
                ("password", L.T("ssh.auth.password")), ("key", L.T("ssh.auth.key")), ("interactive", L.T("ssh.auth.interactive")), ("agent", L.T("ssh.auth.agent")),
            }, d.AuthKind, v => { d.AuthKind = v; d.Notify(); }), L.T("ssh.auth.info")),
            authArea,
            Labeled(L.T("ssh.jumpVia"), K.Dropdown(jumpItems, jumpCurrent, v =>
            {
                d.JumpDeviceId = v.StartsWith("dev:", StringComparison.Ordinal) ? v.Substring(4) : "";
                d.JumpHostId = v.StartsWith("ssh:", StringComparison.Ordinal) ? v.Substring(4) : "";
                d.Notify();
            }), L.T("ssh.jumpVia.info")),
            Labeled(L.T("ssh.theme"), K.Dropdown(themes, d.TerminalTheme, v => { d.TerminalTheme = v; d.Notify(); })),
            CommonFields(app, d));
    }

    private static FrameworkElement RdpFields(AppModel app, EntryDraft d)
    {
        var port = Text(d, d.Port, v => d.Port = v, "3389");
        port.Width = 76;
        var domain = Text(d, d.Domain, v => d.Domain = v, "CORP");
        domain.Width = 100;
        var jumpItems = new List<(string, string)> { ("", L.T("ssh.jump.none")) };
        jumpItems.AddRange(JumpDevices(app).Select(x => (x.Id, app.DeviceName(x.Id))));
        return K.Col(14,
            Labeled(L.T("editor.label"), Text(d, d.Label, v => d.Label = v, "WIN-SRV")),
            K.Row(8, K.Grow(Labeled(L.T("ssh.host"), Text(d, d.Host, v => d.Host = v, "10.0.0.5"))), Labeled(L.T("ssh.port"), port)),
            K.Row(8, Labeled(L.T("rdp.domain"), domain), K.Grow(Labeled(L.T("ssh.username"), Text(d, d.Username, v => d.Username = v, "Administrator")))),
            Labeled(L.T("ssh.password"), K.PasswordField(L.T("ssh.password.ask"), d.Password, v => { d.Password = v; d.Notify(); }), L.T("ssh.password.info")),
            Labeled(L.T("ssh.jump"), K.Dropdown(jumpItems, d.JumpDeviceId, v => { d.JumpDeviceId = v; d.Notify(); }), L.T("rdp.jump.info")),
            CommonFields(app, d));
    }

    private static FrameworkElement CommonFields(AppModel app, EntryDraft d)
        => K.Col(14,
            FolderPicker(app, d),
            Labeled(L.T("hosts.tags"), Text(d, d.Tags, v => d.Tags = v, "prod, web"), L.T("hosts.tags.info")),
            Labeled(L.T("hosts.note"), K.TextArea("", d.Note, 64, v => { d.Note = v; d.Notify(); })),
            K.Row(6, K.Grow(K.Text(L.T("hosts.favorite"), 13)), K.Switch(d.Favorite, v => { d.Favorite = v; d.Notify(); })));

    private static string KeySummary(string k)
    {
        if (k.Contains("OPENSSH PRIVATE KEY")) return "OpenSSH";
        if (k.Contains("RSA PRIVATE KEY")) return "RSA (PEM)";
        if (k.Contains("EC PRIVATE KEY")) return "ECDSA (PEM)";
        if (k.Contains("PRIVATE KEY")) return "PKCS#8";
        return L.T("cred.unknownFormat");
    }

    private static FrameworkElement CredentialFields(AppModel app, EntryDraft d)
    {
        var key = K.TextArea("", d.PrivateKey, 130, v => { d.PrivateKey = v; d.Notify(); }, mono: true, fontSize: 11);
        var summary = new Region(() => d.PrivateKey.Trim().Length == 0 ? null : K.Text(KeySummary(d.PrivateKey), 11.5, Th.Tertiary)).On(d);
        var label = Text(d, d.Label, v => d.Label = v, L.T("cred.label.placeholder"));
        var import = K.SecondaryButton(L.T("cred.import"), () =>
        {
            var dlg = new Microsoft.Win32.OpenFileDialog
            {
                CheckFileExists = true,
                InitialDirectory = IOPath.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".ssh"),
            };
            if (dlg.ShowDialog() != true) return;
            try
            {
                var text = File.ReadAllText(dlg.FileName);
                key.Text = text;
                if (d.Label.Length == 0) label.Text = IOPath.GetFileName(dlg.FileName);
            }
            catch (Exception e) { app.ShowError(e); }
        });
        return K.Col(14,
            Labeled(L.T("editor.label"), label),
            Labeled(L.T("ssh.username"), Text(d, d.Username, v => d.Username = v, "deploy")),
            Labeled(L.T("ssh.password"), K.PasswordField("", d.Password, v => { d.Password = v; d.Notify(); })),
            Labeled(L.T("cred.privateKey"), K.Col(6, key, K.Row(6, K.Grow(summary), import)), L.T("cred.privateKey.info")),
            Labeled(L.T("cred.passphrase"), K.PasswordField("", d.Passphrase, v => { d.Passphrase = v; d.Notify(); })));
    }

    private static FrameworkElement FolderFields(AppModel app, EntryDraft d)
    {
        var colors = new Region(() =>
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal };
            foreach (var c in FolderColors)
            {
                var sel = d.Color == c;
                var dot = new Border { Width = 22, Height = 22, CornerRadius = new CornerRadius(11), Background = c.Length == 0 ? Th.B(Th.P.SecondaryText, 0.3) : Th.Hex(c) };
                var ring = new Btn { Width = 30, Height = 30, CornerRadius = new CornerRadius(15), BorderThickness = new Thickness(sel ? 2 : 0), BorderBrush = Th.Text, Child = K.Center(dot), Margin = new Thickness(0, 0, 4, 0) };
                var cc = c;
                ring.Click += () => { d.Color = cc; d.Notify(); };
                row.Children.Add(ring);
            }
            return row;
        }).On(d);
        return K.Col(14,
            Labeled(L.T("editor.label"), Text(d, d.Label, v => d.Label = v)),
            Labeled(L.T("folder.color"), colors),
            FolderPicker(app, d));
    }

    private static FrameworkElement DeviceFields(AppModel app, EntryDraft d)
    {
        var dev = app.Device(d.DeviceId);
        return K.Col(14,
            dev == null ? null : K.Row(8, K.PlatformGlyph(dev.Platform, 16), K.Text(dev.Hostname, 13, Th.Text, FontWeights.Medium), K.StatusDot(K.DotOf(dev), 7, Th.List)),
            Labeled(L.T("device.alias"), Text(d, d.Label, v => d.Label = v, dev?.Hostname ?? "")),
            CommonFields(app, d));
    }

    private static FrameworkElement SnippetFields(EntryDraft d)
        => K.Col(14,
            Labeled(L.T("editor.label"), Text(d, d.Label, v => d.Label = v)),
            Labeled(L.T("snippet.command"), K.TextArea("", d.Command, 150, v => { d.Command = v; d.Notify(); }, mono: true, fontSize: 12.5)),
            Labeled(L.T("hosts.tags"), Text(d, d.Tags, v => d.Tags = v, "linux, docker")));
}
