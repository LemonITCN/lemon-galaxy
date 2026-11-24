using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>「设备与主机」：中间列（树、凭据、已知主机）与首页（概览 / 详情 + 右侧编辑面板）</summary>
internal static class HostsViews
{
    // ---------- 中间列 ----------

    public static FrameworkElement Middle(AppModel app)
    {
        var newMenu = K.MenuButton(K.Icon("plus", 15, Th.Secondary), () =>
        {
            var folder = CurrentFolder(app);
            void Edit(EditTarget t, HostsListTab tab) { app.HostsTab = tab; app.Editing = t; app.Section = NavSection.Hosts; app.ActiveTabId = null; app.Nav.Notify(); }
            return new List<MenuEntry>
            {
                new(L.T("hosts.new.folder"), () => { app.HostsTab = HostsListTab.Hosts; app.NewFolder(folder); }),
                new(L.T("hosts.new.ssh"), () => Edit(new EditTarget.NewSsh(folder), HostsListTab.Hosts)),
                new(L.T("hosts.new.rdp"), () => Edit(new EditTarget.NewRdp(folder), HostsListTab.Hosts)),
                new(L.T("hosts.new.credential"), () => Edit(new EditTarget.NewCredential(), HostsListTab.Credentials)),
                MenuEntry.Sep,
                new(L.T("assist.connect.title"), () => app.ShowModal(new ModalSpec { Width = 420, Build = () => Modals.AssistConnectCard(app) })),
            };
        }, L.T("common.new"));
        var header = Shell.ColumnHeader(L.T("nav.hosts"),
            K.IconButton("refresh", L.T("common.refresh"), () => Ui.Run(async () => { try { await app.Client.RefreshDevices(); } catch (Exception e) { app.ShowError(e); } }), size: 14),
            newMenu);
        var seg = K.Segmented(new[]
        {
            (HostsListTab.Hosts, L.T("hosts.tab.hosts")), (HostsListTab.Credentials, L.T("hosts.tab.credentials")), (HostsListTab.KnownHosts, L.T("hosts.tab.known")),
        }, app.HostsTab, t => { app.HostsTab = t; app.Nav.Notify(); });
        seg.HorizontalAlignment = HorizontalAlignment.Stretch;
        if (seg.Child is Flex f) foreach (UIElement c in f.Children) Flex.SetGrow(c, 1);
        var body = app.HostsTab switch
        {
            HostsListTab.Credentials => CredentialsList(app),
            HostsListTab.KnownHosts => KnownHostsList(app),
            _ => HostsTab(app),
        };
        var dock = new DockPanel();
        var top = K.Col(0, header, K.Margin(seg, 12, 0, 12, 8));
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        dock.Children.Add(body);
        // 设备列表页打开期间让内核做局域网发现（protocol.md 第 5 节）
        dock.Loaded += (_, _) => app.SetLanDiscovery(true);
        dock.Unloaded += (_, _) => app.SetLanDiscovery(false);
        return dock;
    }

    private static string? CurrentFolder(AppModel app)
    {
        if (app.Selection == null || app.Entry(app.Selection) is not { } e) return null;
        return e.Data is VaultData.Folder ? e.Id : e.FolderId;
    }

    private static FrameworkElement HostsTab(AppModel app)
    {
        var search = K.SearchField(L.T("hosts.search"), app.SearchText, v => { app.SearchText = v; app.Nav.Notify(); }, out _);
        var tags = new Region(() =>
        {
            var all = app.AllTags;
            if (all.Count == 0) return null;
            var row = new StackPanel { Orientation = Orientation.Horizontal };
            foreach (var t in all)
                row.Children.Add(K.Margin(K.TagChip(t, app.TagFilter == t, () => { app.TagFilter = app.TagFilter == t ? null : t; app.Nav.Notify(); }), 0, 0, 5, 0));
            return K.Margin(new ScrollViewer { Content = row, HorizontalScrollBarVisibility = ScrollBarVisibility.Hidden, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled, Focusable = false }, 12, 0, 12, 6);
        }).On(app.Data, app.Nav);
        var list = new Region(() => HostListContent(app)).On(app.Data, app.Nav);
        var scroll = K.Scroll(K.Margin(list, 6, 6, 6, 6));
        var rootTarget = new Border { Background = Brushes.Transparent, Child = scroll };
        Dnd.Target(rootTarget, Dnd.HostFormat, id => app.Move(id, null), h => rootTarget.Background = h ? Th.B(Th.P.Accent, 0.06) : Brushes.Transparent);
        var dock = new DockPanel();
        var top = K.Col(0, K.Margin(search, 12, 0, 12, 6), tags, K.HLine());
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        dock.Children.Add(rootTarget);
        return dock;
    }

    private static FrameworkElement HostListContent(AppModel app)
    {
        var items = new List<UIElement?>();
        if (app.SearchText.Length == 0 && app.TagFilter == null)
        {
            var favs = app.Favorites();
            if (favs.Count > 0)
            {
                items.Add(K.Margin(K.SectionLabel(L.T("hosts.favorites")), 8, 6, 0, 4));
                foreach (var n in favs) items.Add(Row(app, n, 0));
                items.Add(K.Margin(K.SectionLabel(L.T("hosts.all")), 8, 12, 0, 4));
            }
            void Branch(HostNode n, int depth)
            {
                items.Add(Row(app, n, depth));
                if (n.Kind == NodeKind.Folder && n.Expanded) foreach (var c in n.Children) Branch(c, depth + 1);
            }
            foreach (var n in app.Tree()) Branch(n, 0);
        }
        else
        {
            var list = app.Filtered();
            if (list.Count == 0) items.Add(K.Margin(K.Text(L.T("hosts.noMatch"), 12.5, Th.Tertiary), 10, 10, 10, 10));
            foreach (var n in list) items.Add(Row(app, n, 0));
        }
        return K.Col(1, items);
    }

    public static FrameworkElement NodeIcon(HostNode n, double size = 28)
    {
        switch (n.Kind)
        {
            case NodeKind.Folder:
                var c = n.Color != null ? Th.Hex(n.Color) : Th.Secondary;
                return K.Width(K.Center(K.Icon(n.Expanded ? "folder" : "folderFill", 15, c, 1.8)), size);
            case NodeKind.Device:
                var g = new Grid { Width = size, Height = size };
                g.Children.Add(new Border { CornerRadius = new CornerRadius(6), Background = Th.Elevated, BorderBrush = Th.Border, BorderThickness = new Thickness(1), Child = K.Center(K.PlatformGlyph(n.Device?.Platform ?? Platform.Linux, size * 0.5, Th.Secondary)) });
                if (n.Device != null)
                {
                    var dot = K.StatusDot(K.DotOf(n.Device), 7);
                    dot.HorizontalAlignment = HorizontalAlignment.Right;
                    dot.VerticalAlignment = VerticalAlignment.Bottom;
                    dot.Margin = new Thickness(0, 0, -3, -3);
                    g.Children.Add(dot);
                }
                return g;
            case NodeKind.Ssh:
                return K.ProtocolBadge("SSH", Th.P.Accent, size);
            default:
                return K.ProtocolBadge("RDP", Th.P.Info, size);
        }
    }

    private static FrameworkElement Row(AppModel app, HostNode n, int depth)
    {
        var selected = app.Selection == n.Id && app.ActiveTabId == null;
        FrameworkElement chevron;
        if (n.Kind == NodeKind.Folder)
        {
            var cb = K.IconButton(n.Expanded ? "chevronDown" : "chevronRight", null, () => app.ToggleFolder(n.Id), size: 11, w: 16, h: 18);
            chevron = cb;
        }
        else chevron = K.Gap(16);
        var title = K.Text(n.Title, 13, Th.Text, n.Kind == NodeKind.Folder ? FontWeights.SemiBold : FontWeights.Medium);
        UIElement text = n.Kind == NodeKind.Folder ? title : K.Col(1, title, K.Text(n.Subtitle, 11.5, Th.Tertiary));
        var trailing = new List<UIElement?>();
        if (n.Kind == NodeKind.Folder) trailing.Add(K.Text(n.Children.Count.ToString(), 11.5, Th.Tertiary));
        else
        {
            if (n.Tags.Length > 0) trailing.Add(K.TagChip(n.Tags[0]));
            if (n.Favorite) trailing.Add(K.Icon("star", 11, Th.Warning));
            if (n.Device?.IsSelf == true) trailing.Add(K.TagChip(L.T("device.thisMac")));
        }
        var row = K.Row(7, new UIElement?[] { chevron, NodeIcon(n), K.Grow(text) }.Concat(trailing).ToArray());
        var b = new Btn
        {
            Height = n.Kind == NodeKind.Folder ? 30 : 42, CornerRadius = new CornerRadius(6), Padding = new Thickness(4 + depth * 16, 0, 8, 0),
            Normal = selected ? Th.Selection : Brushes.Transparent, HoverBrush = selected ? Th.Selection : Th.Hover,
            Child = row,
        };
        b.UpdateLook();
        b.Click += () =>
        {
            app.Selection = n.Id;
            app.ActiveTabId = null;
            if (n.Kind == NodeKind.Folder) app.Editing = null;
            app.Nav.Notify();
        };
        b.DoubleClick += () => app.Open(n);
        K.ContextMenuFor(b, () => app.ContextMenu(n));
        Dnd.Source(b, Dnd.HostFormat, () => n.Id);
        if (n.Kind == NodeKind.Folder)
            Dnd.Target(b, Dnd.HostFormat, id => app.Move(id, n.Id), h => b.Background = h ? Th.B(Th.P.Accent, 0.14) : selected ? Th.Selection : Brushes.Transparent);
        return b;
    }

    private static FrameworkElement CredentialsList(AppModel app) => K.Scroll(new Region(() =>
    {
        var items = new List<UIElement?>();
        foreach (var e in app.Credentials)
        {
            if (e.Data is not VaultData.Credential c) continue;
            var selected = app.Selection == e.Id;
            var isKey = c.PrivateKey != null;
            var b = new Btn
            {
                Height = 44, CornerRadius = new CornerRadius(6), Padding = new Thickness(8, 0, 8, 0),
                Normal = selected ? Th.Selection : Brushes.Transparent, HoverBrush = selected ? Th.Selection : Th.Hover,
                Child = K.Row(9,
                    new Border { Width = 28, Height = 28, CornerRadius = new CornerRadius(6), Background = Th.Elevated, Child = K.Center(K.Icon(isKey ? "key" : "lock", 14, Th.Secondary)) },
                    K.Grow(K.Col(1, K.Text(c.Label, 13, Th.Text, FontWeights.Medium), K.Text(c.Username + " · " + (isKey ? L.T("cred.kind.key") : L.T("cred.kind.password")), 11.5, Th.Tertiary)))),
            };
            b.UpdateLook();
            var id = e.Id;
            b.Click += () => { app.Selection = id; app.Editing = new EditTarget.Vault(id); app.ActiveTabId = null; app.Nav.Notify(); };
            K.ContextMenuFor(b, () => new List<MenuEntry>
            {
                new(L.T("hosts.edit"), () => { app.Selection = id; app.Editing = new EditTarget.Vault(id); app.ActiveTabId = null; app.Nav.Notify(); }),
                new(L.T("common.delete"), () => app.Confirm(L.T("hosts.delete.title"), L.T("hosts.delete.message", c.Label), L.T("common.delete"), () => Ui.Run(() => app.Delete(id))), Destructive: true),
            });
            items.Add(b);
        }
        if (items.Count == 0) items.Add(K.Margin(K.Text(L.T("cred.empty"), 12.5, Th.Tertiary), 10, 10, 10, 10));
        return K.Margin(K.Col(1, items), 6);
    }).On(app.Data, app.Nav));

    private static FrameworkElement KnownHostsList(AppModel app) => K.Scroll(new Region(() =>
    {
        var items = new List<UIElement?>();
        foreach (var e in app.KnownHosts)
        {
            if (e.Data is not VaultData.KnownHost k) continue;
            var fp = K.Text(k.Fingerprint, 11, Th.Secondary, mono: true);
            fp.TextTrimming = TextTrimming.CharacterEllipsis;
            var b = new Btn
            {
                CornerRadius = new CornerRadius(6), Padding = new Thickness(10, 7, 10, 7), HoverBrush = Th.Hover,
                Child = K.Col(3, K.Row(6, K.Grow(K.Text(k.Port == 22 ? k.Host : $"{k.Host}:{k.Port}", 13, Th.Text, FontWeights.Medium)), K.Text(k.KeyType, 11, Th.Tertiary)), fp),
            };
            b.UpdateLook();
            var id = e.Id;
            K.ContextMenuFor(b, () => new List<MenuEntry>
            {
                new(L.T("known.copy"), () => { SysClipboard.SetText(k.Fingerprint); app.ShowToast(L.T("common.copied")); }),
                new(L.T("common.delete"), () => app.Confirm(L.T("known.delete.title"), L.T("known.delete.message", k.Host), L.T("common.delete"), () => Ui.Run(() => app.Delete(id))), Destructive: true),
            });
            items.Add(b);
            items.Add(K.Margin(K.HLine(), 10, 0, 10, 0));
        }
        return K.Margin(K.Col(1, items), 6);
    }).On(app.Data));

    // ---------- 首页 ----------

    public static FrameworkElement Home(AppModel app)
    {
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var detail = new Region(() => Detail(app)).On(app.Data, app.Nav);
        grid.Children.Add(detail);
        // 编辑面板只在编辑目标变化时重建（输入框要保持焦点与内容）
        var editor = new Region(() =>
        {
            if (app.Editing is null or EditTarget.NewSnippet or EditTarget.Forward) return null;
            var panel = Editors.Panel(app, app.Editing);
            return K.Row(0, K.VLine(), new Border { Width = 360, Background = Th.List, Child = panel });
        }).Keyed(() => app.Editing).On(app.Nav);
        Grid.SetColumn(editor, 1);
        grid.Children.Add(editor);
        return grid;
    }

    private static FrameworkElement Detail(AppModel app)
    {
        var s = app.Selection;
        if (s == null) return Overview(app, null);
        if (s.StartsWith("dev:", StringComparison.Ordinal) && app.Device(s.Substring(4)) is { } d) return DeviceDetail(app, d);
        return app.Entry(s) switch
        {
            { Data: VaultData.SshHost } e => SshDetail(app, e),
            { Data: VaultData.RdpHost } e => RdpDetail(app, e),
            { Data: VaultData.Folder } e => Overview(app, e.Id),
            _ => Overview(app, null),
        };
    }

    private static FrameworkElement Overview(AppModel app, string? folderId)
    {
        var col = new List<UIElement?>();
        var headerItems = new List<UIElement?>();
        if (folderId != null && app.Entry(folderId)?.Data is VaultData.Folder f) headerItems.Add(K.Semi(f.Name, 21));
        else
        {
            headerItems.Add(K.Semi(L.T("nav.hosts"), 21));
            var online = app.Devices.Count(d => d.Online && !d.IsSelf && d.Kind != DeviceKind.Mobile);
            headerItems.Add(K.Margin(K.Text(L.T("hosts.onlineCount", online), 13, Th.Secondary), 0, 5, 0, 0));
        }
        headerItems.Add(K.Spacer());
        headerItems.Add(K.SecondaryButton(L.T("hosts.new.ssh"), () => { app.Editing = new EditTarget.NewSsh(folderId); app.Nav.Notify(); }, icon: "plus"));
        col.Add(K.Row(10, headerItems));
        var nodes = app.Nodes();
        if (folderId != null)
        {
            var inFolder = nodes.Where(n => n.FolderId == folderId).ToList();
            inFolder.Sort(HostTree.Order);
            col.Add(CardGrid(app, inFolder));
        }
        else
        {
            var favs = app.Favorites();
            if (favs.Count > 0) col.Add(Group(app, L.T("hosts.favorites"), favs));
            var devices = nodes.Where(n => n.Kind == NodeKind.Device && n.Device is { IsSelf: false } dd && dd.Kind != DeviceKind.Mobile)
                .OrderBy(n => n.Device!.Online ? 0 : 1).ThenBy(n => n.Title, StringComparer.CurrentCultureIgnoreCase).ToList();
            col.Add(Group(app, L.T("hosts.devices"), devices));
            var hosts = nodes.Where(n => n.Kind is NodeKind.Ssh or NodeKind.Rdp).ToList();
            hosts.Sort(HostTree.Order);
            col.Add(Group(app, L.T("hosts.directHosts"), hosts));
        }
        return K.PageScroll(K.Col(22, col), 1100);
    }

    private static FrameworkElement Group(AppModel app, string title, List<HostNode> nodes) => K.Col(10, K.SectionLabel(title), CardGrid(app, nodes));

    private static FrameworkElement CardGrid(AppModel app, List<HostNode> nodes)
    {
        var g = new AdaptiveGrid { MinItem = 230, Spacing = 12 };
        foreach (var n in nodes) g.Children.Add(Card(app, n));
        return g;
    }

    private static FrameworkElement Card(AppModel app, HostNode n)
    {
        FrameworkElement icon = n.Kind switch
        {
            NodeKind.Device => new Border { Width = 38, Height = 38, CornerRadius = new CornerRadius(8), Background = Th.B(Th.P.Accent, 0.12), Child = K.Center(K.PlatformGlyph(n.Device?.Platform ?? Platform.Linux, 18, Th.Accent)) },
            NodeKind.Ssh => K.ProtocolBadge("SSH", Th.P.Accent, 38, 11),
            NodeKind.Rdp => K.ProtocolBadge("RDP", Th.P.Info, 38, 11),
            _ => new Border { Width = 38, Height = 38, CornerRadius = new CornerRadius(8), Background = Th.Elevated, Child = K.Center(K.Icon("folderFill", 17, n.Color != null ? Th.Hex(n.Color) : Th.Secondary)) },
        };
        var titleRow = K.Row(6, K.Grow(K.Semi(n.Title, 13.5)), n.Device != null ? K.StatusDot(K.DotOf(n.Device), 7, Th.Card) : null);
        var b = new Btn
        {
            Padding = new Thickness(12), CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(1),
            Normal = Th.Card, HoverBrush = Th.Elevated, NormalBorder = Th.Border, HoverBorder = Th.B(Th.P.Accent, 0.5),
            Child = K.Row(11, icon, K.Grow(K.Col(3, titleRow, K.Text(n.Subtitle, 12, Th.Secondary)))),
        };
        b.UpdateLook();
        b.Click += () => { app.Selection = n.Id; app.Nav.Notify(); };
        b.DoubleClick += () => app.Open(n);
        K.ContextMenuFor(b, () => app.ContextMenu(n));
        return b;
    }

    // ---------- 详情 ----------

    private static FrameworkElement DetailHeader(string title, string subtitle, UIElement icon, UIElement? actions)
        => K.Row(14, icon, K.Grow(K.Col(4, K.Semi(title, 21), K.Text(subtitle, 13, Th.Secondary))), actions);

    public static FrameworkElement ActionTile(string icon, string title, bool enabled, Action a)
    {
        var b = new Btn
        {
            Height = 80, CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(1), Normal = Th.Card, NormalBorder = Th.Border,
            HoverBrush = Th.B(K.Mix(Th.P.Card, Th.P.Accent, 0.06)), HoverBorder = Th.B(Th.P.Accent, 0.6),
            Child = K.Center(K.Col(8, K.Center(K.Icon(icon, 22, enabled ? Th.Text : Th.Tertiary, 1.5)), K.Center(K.Text(title, 12.5, enabled ? Th.Text : Th.Tertiary, FontWeights.Medium)))),
        };
        b.UpdateLook();
        b.IsEnabled = enabled;
        b.Click += a;
        return K.Grow(b);
    }

    public static FrameworkElement InfoGrid(IEnumerable<(string label, string value)> rows)
    {
        var list = rows.ToList();
        var items = new List<UIElement?>();
        for (var i = 0; i < list.Count; i++)
        {
            var v = K.SelectableText(list[i].value, 13, Th.Text, wrap: true);
            items.Add(new Border
            {
                Padding = new Thickness(14, 9, 14, 9),
                Child = K.Row(0, K.Width(K.Text(list[i].label, 13, Th.Secondary), 140), K.Grow(v)),
            });
            if (i < list.Count - 1) items.Add(K.Margin(K.HLine(), 14, 0, 0, 0));
        }
        return K.Card(K.Col(0, items), 0);
    }

    private static FrameworkElement Tags(string[] tags)
    {
        if (tags.Length == 0) return K.Gap(0);
        var p = new FlowPanel { Spacing = 5 };
        foreach (var t in tags) p.Children.Add(K.TagChip(t));
        return p;
    }

    private static FrameworkElement? Note(string note) => note.Length == 0 ? null : K.Col(6, K.SectionLabel(L.T("hosts.note")), K.Text(note, 13, Th.Text, wrap: true));

    private static FrameworkElement DeviceDetail(AppModel app, DeviceInfo d)
    {
        var id = d.Id;
        var gui = d.Platform is Platform.Windows or Platform.Macos or Platform.Linux;
        var connectable = !d.IsSelf && d.Kind != DeviceKind.Mobile && (d.Online || d.LanSeen);
        var meta = app.MetaFor(id)?.Data as VaultData.DeviceMeta;
        var icon = new Grid { Width = 54, Height = 54 };
        icon.Children.Add(new Border { CornerRadius = new CornerRadius(12), Background = Th.B(Th.P.Accent, 0.12), Child = K.Center(K.PlatformGlyph(d.Platform, 26, Th.Accent)) });
        var dot = K.StatusDot(K.DotOf(d), 10, Th.Content);
        dot.HorizontalAlignment = HorizontalAlignment.Right;
        dot.VerticalAlignment = VerticalAlignment.Bottom;
        dot.Margin = new Thickness(0, 0, -3, -3);
        icon.Children.Add(dot);
        var col = new List<UIElement?>
        {
            DetailHeader(app.DeviceName(id), $"{d.Hostname} · {d.OsVersion} · {d.Arch}", icon,
                K.SecondaryButton(L.T("hosts.edit"), () => { app.Editing = new EditTarget.Device(id); app.Nav.Notify(); }, icon: "pencil")),
            meta != null && meta.Tags.Length > 0 ? Tags(meta.Tags) : null,
        };
        if (d.IsSelf) col.Add(K.Text(L.T("device.selfHint.win"), 13, Th.Secondary, wrap: true));
        else if (d.Kind == DeviceKind.Mobile) col.Add(K.Text(L.T("device.mobileHint"), 13, Th.Secondary, wrap: true));
        else
        {
            // 离线且能远程开机：连接方式上面放一条远程开机
            if (app.CanWake(id)) col.Add(WakeCard(app, d));
            col.Add(K.Row(12,
                ActionTile("display", L.T("connect.desktop"), connectable && gui, () => app.OpenRemoteDesktop(id)),
                ActionTile("terminal", L.T("connect.terminal"), connectable, () => app.OpenTerminalAsking(id)),
                ActionTile("folder", L.T("connect.files"), connectable, () => app.OpenSftp(id)),
                ActionTile("sparkles", L.T("connect.ai"), connectable && app.AiEnabled, () => app.OpenAi(new AgentTarget(AgentTargetKind.Device, id)))));
        }
        string status;
        if (d.Online) status = d.LanSeen ? L.T("device.onlineLan") : L.T("device.online");
        else if (d.LanSeen) status = L.T("device.lanOnly");
        else status = L.T("device.offlineSince", Fmt.Relative(d.LastSeenAt));
        var kindKey = d.Kind switch { DeviceKind.Control => "control", DeviceKind.Worker => "worker", _ => "mobile" };
        col.Add(InfoGrid(new[]
        {
            (L.T("device.status"), status), (L.T("device.kind"), L.T("device.kind." + kindKey)), (L.T("device.os"), d.OsVersion),
            (L.T("device.arch"), d.Arch), (L.T("device.appVersion"), d.AppVersion), (L.T("device.lastSeen"), Fmt.Date(d.LastSeenAt)), (L.T("device.id"), d.Id),
        }));
        col.Add(Note(meta?.Note ?? ""));
        if (!d.IsSelf)
            col.Add(K.Row(0, K.Spacer(), K.SecondaryButton(L.T("device.remove"), () =>
                app.Confirm(L.T("device.remove.title"), L.T("device.remove.message", app.DeviceName(id)), L.T("device.remove"), () => Ui.Run(async () =>
                {
                    try { await app.Client.RemoveDevice(id); app.Selection = null; app.ReloadDevices(); app.Nav.Notify(); }
                    catch (Exception e) { app.ShowError(e); }
                })), destructive: true)));
        return K.PageScroll(K.Col(20, col), 900);
    }

    /// <summary>离线设备的远程开机：状态 + 按钮，网络唤醒的前提放在 i 图标里</summary>
    private static FrameworkElement WakeCard(AppModel app, DeviceInfo d)
    {
        var busy = app.IsWaking(d.Id);
        var button = K.PrimaryButton(busy ? L.T("wake.sending") : L.T("wake.action"), () => app.WakeDevice(d.Id), icon: "power", enabled: !busy);
        return K.Card(K.Row(12,
            new Border { Width = 36, Height = 36, CornerRadius = new CornerRadius(9), Background = Th.Elevated, Child = K.Center(K.Icon("power", 17, Th.Secondary)) },
            K.Grow(K.Col(3, K.Row(5, K.Text(L.T("device.offline"), 13.5, Th.Text, FontWeights.Medium), K.Info(L.T("wake.info"))),
                K.Text(L.T("device.lastSeen") + " " + Fmt.Relative(d.LastSeenAt), 12, Th.Secondary))),
            button), 14);
    }

    private static FrameworkElement SshDetail(AppModel app, VaultEntry e)
    {
        if (e.Data is not VaultData.SshHost h) return K.Gap(0);
        string AuthText() => h.Auth switch
        {
            SshAuth.Password => L.T("ssh.auth.password"),
            SshAuth.PrivateKey pk => app.Entry(pk.CredentialId)?.Data is VaultData.Credential c ? L.T("ssh.auth.key") + " · " + c.Label : L.T("ssh.auth.key"),
            SshAuth.Agent => L.T("ssh.auth.agent"),
            _ => L.T("ssh.auth.interactive"),
        };
        var jump = h.JumpDeviceId != null ? L.T("ssh.via.device", app.DeviceName(h.JumpDeviceId)) : h.JumpHostId != null ? L.T("ssh.via.host", app.SshLabel(h.JumpHostId)) : L.T("ssh.via.direct");
        var col = new List<UIElement?>
        {
            DetailHeader(h.Label, $"{h.Username}@{h.Host}:{h.Port}", K.ProtocolBadge("SSH", Th.P.Accent, 54, 14),
                K.SecondaryButton(L.T("hosts.edit"), () => { app.Editing = new EditTarget.Vault(e.Id); app.Nav.Notify(); }, icon: "pencil")),
            h.Tags.Length > 0 ? Tags(h.Tags) : null,
            K.Row(12,
                ActionTile("terminal", L.T("connect.terminal"), true, () => app.OpenSshTerminal(e.Id)),
                ActionTile("folder", L.T("connect.sftp"), true, () => app.OpenSftp(sshHostId: e.Id)),
                ActionTile("sparkles", L.T("connect.ai"), app.AiEnabled, () => app.OpenAi(new AgentTarget(AgentTargetKind.SshHost, e.Id)))),
            h.Auth is SshAuth.Agent ? K.Row(6, K.Icon("info", 14, Th.Warning), K.Grow(K.Text(L.T("ssh.agent.unsupported"), 12.5, Th.Secondary, wrap: true))) : null,
            InfoGrid(new[]
            {
                (L.T("ssh.address"), $"{h.Host}:{h.Port}"), (L.T("ssh.username"), h.Username), (L.T("ssh.auth"), AuthText()),
                (L.T("ssh.jumpVia"), jump), (L.T("ssh.theme"), h.TerminalTheme != null ? TerminalTheme.Named(h.TerminalTheme).Name : L.T("ssh.theme.default")),
            }),
            Note(h.Note),
        };
        return K.PageScroll(K.Col(20, col), 900);
    }

    private static FrameworkElement RdpDetail(AppModel app, VaultEntry e)
    {
        if (e.Data is not VaultData.RdpHost r) return K.Gap(0);
        var col = new List<UIElement?>
        {
            DetailHeader(r.Label, $"{r.Host}:{r.Port}", K.ProtocolBadge("RDP", Th.P.Info, 54, 14),
                K.SecondaryButton(L.T("hosts.edit"), () => { app.Editing = new EditTarget.Vault(e.Id); app.Nav.Notify(); }, icon: "pencil")),
            r.Tags.Length > 0 ? Tags(r.Tags) : null,
            K.Row(8, K.PrimaryButton(L.T("connect.rdp"), () => app.OpenRdp(e.Id), icon: "display"), K.Info(L.T("rdp.info"))),
            InfoGrid(new[]
            {
                (L.T("ssh.address"), $"{r.Host}:{r.Port}"), (L.T("ssh.username"), (r.Domain is { Length: > 0 } ? r.Domain + "\\" : "") + r.Username),
                (L.T("ssh.jump"), r.JumpDeviceId != null ? app.DeviceName(r.JumpDeviceId) : L.T("ssh.jump.none")),
            }),
            Note(r.Note),
        };
        return K.PageScroll(K.Col(20, col), 900);
    }
}
