namespace LemonGalaxy.UI;

internal enum NodeKind { Folder, Device, Ssh, Rdp }

/// <summary>设备、SSH 主机、RDP 主机与文件夹组成的一棵树（文件夹结构存在端到端加密库里）。</summary>
internal sealed class HostNode
{
    /// <summary>文件夹 / SSH / RDP：加密库条目 id；设备："dev:" + 设备 id</summary>
    public string Id = "";
    public NodeKind Kind;
    public string Title = "";
    public string Subtitle = "";
    public string[] Tags = Array.Empty<string>();
    public bool Favorite;
    public string? FolderId;
    public long Sort;
    public List<HostNode> Children = new();
    public DeviceInfo? Device;
    public string? Color;
    public bool Expanded = true;

    public string? DeviceId => Kind == NodeKind.Device ? Id.Substring(4) : null;
    public string Searchable => string.Join(" ", new[] { Title, Subtitle }.Concat(Tags)).ToLowerInvariant();
}

internal static class HostTree
{
    public static List<HostNode> Nodes(this AppModel app)
    {
        var nodes = new List<HostNode>();
        var metaByDevice = new Dictionary<string, VaultEntry>();
        foreach (var e in app.Vault)
        {
            switch (e.Data)
            {
                case VaultData.Folder f:
                    nodes.Add(new HostNode { Id = e.Id, Kind = NodeKind.Folder, Title = f.Name, FolderId = e.FolderId, Sort = e.Sort, Color = f.Color, Expanded = f.Expanded });
                    break;
                case VaultData.DeviceMeta m:
                    metaByDevice[m.DeviceId] = e;
                    break;
                case VaultData.SshHost h:
                    var sub = $"{h.Username}@{h.Host}" + (h.Port == 22 ? "" : $":{h.Port}");
                    if (h.JumpDeviceId != null) sub += " · " + L.T("hosts.via", app.DeviceName(h.JumpDeviceId));
                    else if (h.JumpHostId != null) sub += " · " + L.T("hosts.via", app.SshLabel(h.JumpHostId));
                    nodes.Add(new HostNode { Id = e.Id, Kind = NodeKind.Ssh, Title = h.Label, Subtitle = sub, Tags = h.Tags, Favorite = h.Favorite, FolderId = e.FolderId, Sort = e.Sort });
                    break;
                case VaultData.RdpHost r:
                    nodes.Add(new HostNode
                    {
                        Id = e.Id, Kind = NodeKind.Rdp, Title = r.Label, Subtitle = $"{r.Username}@{r.Host}" + (r.Port == 3389 ? "" : $":{r.Port}"),
                        Tags = r.Tags, Favorite = r.Favorite, FolderId = e.FolderId, Sort = e.Sort,
                    });
                    break;
            }
        }
        foreach (var d in app.Devices)
        {
            var n = new HostNode { Id = "dev:" + d.Id, Kind = NodeKind.Device, Title = d.Hostname, Subtitle = d.OsVersion, Sort = 100, Device = d };
            if (metaByDevice.TryGetValue(d.Id, out var me) && me.Data is VaultData.DeviceMeta m)
            {
                if (!string.IsNullOrEmpty(m.Alias)) { n.Title = m.Alias!; n.Subtitle = d.Hostname + " · " + d.OsVersion; }
                n.Tags = m.Tags;
                n.Favorite = m.Favorite;
                n.FolderId = me.FolderId;
                n.Sort = me.Sort;
            }
            nodes.Add(n);
        }
        return nodes;
    }

    public static int Order(HostNode a, HostNode b)
    {
        if ((a.Kind == NodeKind.Folder) != (b.Kind == NodeKind.Folder)) return a.Kind == NodeKind.Folder ? -1 : 1;
        if (a.Sort != b.Sort) return a.Sort.CompareTo(b.Sort);
        return string.Compare(a.Title, b.Title, StringComparison.CurrentCultureIgnoreCase);
    }

    /// <summary>组装成树；文件夹在前，其余按排序值、名称</summary>
    public static List<HostNode> Tree(this AppModel app)
    {
        var all = app.Nodes();
        var folderIds = new HashSet<string>(all.Where(n => n.Kind == NodeKind.Folder).Select(n => n.Id));
        // depth 防止加密库里的文件夹意外成环时无限递归
        List<HostNode> Children(string? parent, int depth)
        {
            var list = all.Where(n => (n.FolderId != null && folderIds.Contains(n.FolderId) ? n.FolderId : null) == parent).ToList();
            foreach (var n in list.Where(n => n.Kind == NodeKind.Folder)) n.Children = depth > 32 ? new List<HostNode>() : Children(n.Id, depth + 1);
            list.Sort(Order);
            return list;
        }
        return Children(null, 0);
    }

    public static List<HostNode> Favorites(this AppModel app)
        => app.Nodes().Where(n => n.Favorite && n.Kind != NodeKind.Folder).OrderBy(n => n.Title, StringComparer.CurrentCultureIgnoreCase).ToList();

    public static List<HostNode> Filtered(this AppModel app)
    {
        var q = app.SearchText.Trim().ToLowerInvariant();
        return app.Nodes().Where(n => n.Kind != NodeKind.Folder && (app.TagFilter == null || n.Tags.Contains(app.TagFilter)) && (q.Length == 0 || n.Searchable.Contains(q)))
            .OrderBy(n => n.Favorite ? 0 : 1).ThenBy(n => n.Title, StringComparer.CurrentCultureIgnoreCase).ToList();
    }

    // ---------- 树操作 ----------

    public static void ToggleFolder(this AppModel app, string id)
    {
        if (app.Entry(id) is not { Data: VaultData.Folder f } e) return;
        Ui.Run(async () => await app.Save(e with { Data = f with { Expanded = !f.Expanded } }));
    }

    public static bool IsAncestor(this AppModel app, string folder, string? target)
    {
        var cur = target;
        var guard = 0;
        while (cur != null && guard++ < 64)
        {
            if (cur == folder) return true;
            cur = app.Entry(cur)?.FolderId;
        }
        return false;
    }

    public static void Move(this AppModel app, string itemId, string? folderId)
    {
        if (itemId == folderId) return;
        if (itemId.StartsWith("dev:", StringComparison.Ordinal))
        {
            var did = itemId.Substring(4);
            Ui.Run(() => app.UpdateDeviceMeta(did, (m, _) => (m, folderId)));
            return;
        }
        if (app.Entry(itemId) is not { } e) return;
        if (e.Data is VaultData.Folder && folderId != null && app.IsAncestor(itemId, folderId)) return;
        var sort = app.Vault.Where(v => v.FolderId == folderId).Select(v => v.Sort).DefaultIfEmpty(0).Max() + 1;
        Ui.Run(async () => await app.Save(e with { FolderId = folderId, Sort = sort }));
    }

    public static void Rename(this AppModel app, string itemId, string name)
    {
        name = name.Trim();
        if (itemId.StartsWith("dev:", StringComparison.Ordinal))
        {
            var did = itemId.Substring(4);
            Ui.Run(() => app.UpdateDeviceMeta(did, (m, f) => (m with { Alias = name.Length == 0 ? null : name }, f)));
            return;
        }
        if (name.Length == 0 || app.Entry(itemId) is not { } e) return;
        VaultData? d = e.Data switch
        {
            VaultData.Folder f => f with { Name = name },
            VaultData.SshHost h => h with { Label = name },
            VaultData.RdpHost r => r with { Label = name },
            VaultData.Credential c => c with { Label = name },
            VaultData.Snippet s => s with { Label = name },
            _ => null,
        };
        if (d != null) Ui.Run(async () => await app.Save(e with { Data = d }));
    }

    public static void ToggleFavorite(this AppModel app, string itemId)
    {
        if (itemId.StartsWith("dev:", StringComparison.Ordinal))
        {
            var did = itemId.Substring(4);
            Ui.Run(() => app.UpdateDeviceMeta(did, (m, f) => (m with { Favorite = !m.Favorite }, f)));
            return;
        }
        if (app.Entry(itemId) is not { } e) return;
        VaultData? d = e.Data switch
        {
            VaultData.SshHost h => h with { Favorite = !h.Favorite },
            VaultData.RdpHost r => r with { Favorite = !r.Favorite },
            _ => null,
        };
        if (d != null) Ui.Run(async () => await app.Save(e with { Data = d }));
    }

    public static void NewFolder(this AppModel app, string? parent)
    {
        Modals.Prompt(app, L.T("hosts.new.folder"), L.T("hosts.newFolderName"), L.T("common.create"), name =>
            Ui.Run(async () =>
            {
                var saved = await app.Save(new VaultEntry("", parent, 0, new VaultData.Folder(name, null, true), 0));
                if (saved != null) { app.Selection = saved.Id; app.Nav.Notify(); }
            }));
    }

    public static void Open(this AppModel app, HostNode n)
    {
        switch (n.Kind)
        {
            case NodeKind.Device:
                if (n.DeviceId != null) app.ActivateDevice(n.DeviceId);
                break;
            case NodeKind.Ssh:
                app.OpenSshTerminal(n.Id);
                break;
            case NodeKind.Rdp:
                app.OpenRdp(n.Id);
                break;
            case NodeKind.Folder:
                app.ToggleFolder(n.Id);
                break;
        }
    }

    public static void AskDelete(this AppModel app, HostNode n)
    {
        var msg = n.Kind == NodeKind.Folder ? L.T("hosts.delete.folderMessage", n.Title) : L.T("hosts.delete.message", n.Title);
        app.Confirm(L.T("hosts.delete.title"), msg, L.T("common.delete"), () => Ui.Run(() => app.Delete(n.Id)));
    }

    public static void AskRename(this AppModel app, HostNode n)
        => Modals.Prompt(app, L.T("hosts.rename"), n.Title, L.T("common.save"), v => app.Rename(n.Id, v));

    /// <summary>右键菜单（列表行与概览卡片共用）</summary>
    public static List<MenuEntry> ContextMenu(this AppModel app, HostNode n)
    {
        var list = new List<MenuEntry>();
        void Edit(EditTarget t) { app.Selection = n.Id; app.Editing = t; app.ActiveTabId = null; app.Nav.Notify(); }
        switch (n.Kind)
        {
            case NodeKind.Device:
                if (n.Device is { } d && !d.IsSelf && d.Kind != DeviceKind.Mobile && n.DeviceId is { } id)
                {
                    if (d.Platform is Platform.Windows or Platform.Macos) list.Add(new(L.T("connect.desktop"), () => app.OpenRemoteDesktop(id)));
                    list.Add(new(L.T("connect.terminal"), () => app.OpenTerminalAsking(id)));
                    list.Add(new(L.T("connect.files"), () => app.OpenSftp(id)));
                    list.Add(new(L.T("connect.ai"), () => app.OpenAi(new AgentTarget(AgentTargetKind.Device, id)), app.AiEnabled));
                    list.Add(MenuEntry.Sep);
                }
                list.Add(new(L.T("hosts.edit"), () => Edit(new EditTarget.Device(n.DeviceId ?? ""))));
                list.Add(new(L.T("hosts.rename"), () => app.AskRename(n)));
                list.Add(new(n.Favorite ? L.T("hosts.unfavorite") : L.T("hosts.favorite"), () => app.ToggleFavorite(n.Id)));
                break;
            case NodeKind.Ssh:
                list.Add(new(L.T("connect.terminal"), () => app.OpenSshTerminal(n.Id)));
                list.Add(new(L.T("connect.sftp"), () => app.OpenSftp(sshHostId: n.Id)));
                list.Add(new(L.T("connect.ai"), () => app.OpenAi(new AgentTarget(AgentTargetKind.SshHost, n.Id)), app.AiEnabled));
                list.Add(MenuEntry.Sep);
                list.Add(new(L.T("hosts.edit"), () => Edit(new EditTarget.Vault(n.Id))));
                list.Add(new(L.T("hosts.rename"), () => app.AskRename(n)));
                list.Add(new(n.Favorite ? L.T("hosts.unfavorite") : L.T("hosts.favorite"), () => app.ToggleFavorite(n.Id)));
                list.Add(MenuEntry.Sep);
                list.Add(new(L.T("common.delete"), () => app.AskDelete(n), Destructive: true));
                break;
            case NodeKind.Rdp:
                list.Add(new(L.T("connect.rdp"), () => app.OpenRdp(n.Id)));
                list.Add(MenuEntry.Sep);
                list.Add(new(L.T("hosts.edit"), () => Edit(new EditTarget.Vault(n.Id))));
                list.Add(new(L.T("hosts.rename"), () => app.AskRename(n)));
                list.Add(new(n.Favorite ? L.T("hosts.unfavorite") : L.T("hosts.favorite"), () => app.ToggleFavorite(n.Id)));
                list.Add(MenuEntry.Sep);
                list.Add(new(L.T("common.delete"), () => app.AskDelete(n), Destructive: true));
                break;
            case NodeKind.Folder:
                list.Add(new(L.T("hosts.new.folder"), () => app.NewFolder(n.Id)));
                list.Add(new(L.T("hosts.new.ssh"), () => Edit(new EditTarget.NewSsh(n.Id))));
                list.Add(MenuEntry.Sep);
                list.Add(new(L.T("hosts.rename"), () => app.AskRename(n)));
                list.Add(new(L.T("hosts.edit"), () => Edit(new EditTarget.Vault(n.Id))));
                list.Add(MenuEntry.Sep);
                list.Add(new(L.T("common.delete"), () => app.AskDelete(n), Destructive: true));
                break;
        }
        if (n.Kind != NodeKind.Folder)
        {
            var sub = new List<MenuEntry> { new(L.T("hosts.root"), () => app.Move(n.Id, null), Checked: n.FolderId == null) };
            sub.AddRange(app.Nodes().Where(f => f.Kind == NodeKind.Folder).Select(f => new MenuEntry(f.Title, () => app.Move(n.Id, f.Id), Checked: n.FolderId == f.Id)));
            list.Add(new(L.T("hosts.moveTo"), Sub: sub));
        }
        return list;
    }
}
