using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using LemonGalaxy.Core;

namespace LemonGalaxy.UI;

/// <summary>输入框「+」菜单里的选择器：设备或主机、文件或文件夹、命令片段、端口映射（都画成模态卡片）</summary>
internal static class AiPickers
{
    /// <summary>引用的图标：设备按平台，SSH 主机、文件、文件夹、片段、端口映射、连接记录各一个</summary>
    public static string RefIcon(AppModel app, ContextRef r) => r.Kind switch
    {
        ContextRefKind.Device => r.Id == AiHub.LocalId ? "display" : app.Device(r.Id) is { } d ? K.PlatformIcon(d.Platform) : "display",
        ContextRefKind.SshHost => "server",
        ContextRefKind.File => "fileText",
        ContextRefKind.Folder => "folder",
        ContextRefKind.Snippet => "code",
        ContextRefKind.PortForward => "forward",
        _ => "clock",
    };

    /// <summary>悬停提示：文件与文件夹显示完整路径与所在设备</summary>
    public static string RefTip(AppModel app, ContextRef r) => r.Kind is ContextRefKind.File or ContextRefKind.Folder
        ? r.Id + "\n" + app.Ai.WhereName(r.TargetId)
        : r.Label;

    // ---------- 多选列表 ----------

    private sealed record Option(ContextRef Ref, string Title, string? Subtitle, string Icon, DotState? Dot, bool Enabled = true);

    private static void MultiPick(AppModel app, string title, string empty, List<Option> options, ComposerDraft draft)
    {
        var chosen = new HashSet<ContextRef>(draft.Refs.Where(r => options.Any(o => o.Ref == r)));
        var filter = "";
        app.ShowModal(new ModalSpec
        {
            Width = 460,
            Build = () =>
            {
                Btn? add = null;
                void UpdateAdd()
                {
                    if (add == null) return;
                    add.IsEnabled = chosen.Count > 0;
                }
                Region? list = null;
                list = new Region(() =>
                {
                    var rows = new List<UIElement?>();
                    var q = filter.Trim();
                    foreach (var o in options.Where(o => q.Length == 0 || o.Title.IndexOf(q, StringComparison.CurrentCultureIgnoreCase) >= 0
                                                         || (o.Subtitle?.IndexOf(q, StringComparison.CurrentCultureIgnoreCase) ?? -1) >= 0))
                    {
                        var on = chosen.Contains(o.Ref);
                        var box = new Border
                        {
                            Width = 16, Height = 16, CornerRadius = new CornerRadius(4), BorderThickness = new Thickness(1),
                            Background = on ? Th.Accent : Th.Elevated, BorderBrush = on ? Th.Accent : Th.B(Th.P.SecondaryText, 0.6),
                            Child = on ? K.Icon("check", 12, Th.AccentText, 3) : null,
                        };
                        var text = K.Col(1, K.Text(o.Title, 13, o.Enabled ? Th.Text : Th.Tertiary), o.Subtitle == null ? null : K.Text(o.Subtitle, 11.5, Th.Tertiary, mono: o.Ref.Kind == ContextRefKind.Snippet));
                        var b = new Btn
                        {
                            CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 7, 10, 7), HoverBrush = Th.Hover,
                            Child = K.Row(10, box, K.Icon(o.Icon, 14, o.Enabled ? Th.Secondary : Th.Tertiary, 1.8), K.Grow(text), o.Dot is { } dot ? K.StatusDot(dot, 7, Th.Card) : null),
                        };
                        b.UpdateLook();
                        b.IsEnabled = o.Enabled;
                        var opt = o;
                        b.Click += () =>
                        {
                            if (!chosen.Remove(opt.Ref)) chosen.Add(opt.Ref);
                            UpdateAdd();
                            list!.Force();
                        };
                        rows.Add(b);
                    }
                    if (rows.Count == 0) rows.Add(K.Margin(K.Text(options.Count == 0 ? empty : L.T("ai.pick.noMatch"), 13, Th.Tertiary, wrap: true), 10, 14, 10, 14));
                    return K.Col(1, rows);
                });
                var search = K.SearchField(L.T("ai.pick.search"), "", v => { filter = v; list.Force(); }, out _);
                add = K.PrimaryButton(L.T("ai.pick.add"), () =>
                {
                    foreach (var r in options.Select(o => o.Ref).Where(chosen.Contains)) draft.AddRef(r);
                    app.CloseModal();
                });
                UpdateAdd();
                var scroll = K.Scroll(list);
                scroll.MaxHeight = 360;
                return K.Col(14,
                    Modals.Header(app, title),
                    options.Count > 6 ? search : null,
                    scroll,
                    Modals.Buttons(K.SecondaryButton(L.T("common.cancel"), () => app.CloseModal(true)), add));
            },
        });
    }

    private static List<Option> TargetOptions(AppModel app, bool onlyReachable)
    {
        // 本机排第一，总是在线
        var local = app.Ai.LocalRef;
        var list = new List<Option> { new(local, local.Label, null, "display", DotState.Online) };
        foreach (var d in app.Devices.Where(d => !d.IsSelf && d.Kind != DeviceKind.Mobile).OrderByDescending(d => d.Online || d.LanSeen).ThenBy(d => app.DeviceName(d.Id), StringComparer.CurrentCultureIgnoreCase))
        {
            var reachable = d.Online || d.LanSeen;
            list.Add(new Option(AiHub.DeviceRef(app, d.Id), app.DeviceName(d.Id), d.OsVersion, K.PlatformIcon(d.Platform), K.DotOf(d), !onlyReachable || reachable));
        }
        foreach (var e in app.SshHosts)
        {
            var h = e.Data as VaultData.SshHost;
            list.Add(new Option(AiHub.SshRef(app, e.Id), app.SshLabel(e.Id), h == null ? null : $"{h.Username}@{h.Host}" + (h.Port != 22 ? ":" + h.Port : ""), "server", null));
        }
        return list;
    }

    public static void PickTargets(AppModel app, ComposerDraft draft)
        => MultiPick(app, L.T("ai.plus.targets"), L.T("ai.pick.noTargets"), TargetOptions(app, false), draft);

    public static void PickSnippets(AppModel app, ComposerDraft draft)
    {
        var options = app.Snippets.Where(e => e.Data is VaultData.Snippet).Select(e =>
        {
            var s = (VaultData.Snippet)e.Data;
            return new Option(new ContextRef(ContextRefKind.Snippet, "", e.Id, s.Label), s.Label, s.Command.Replace("\n", " "), "code", null);
        }).ToList();
        MultiPick(app, L.T("ai.plus.snippets"), L.T("ai.pick.noSnippets"), options, draft);
    }

    public static void PickForwards(AppModel app, ComposerDraft draft)
    {
        var options = app.PortForwards.Where(e => e.Data is VaultData.PortForward).Select(e =>
        {
            var f = (VaultData.PortForward)e.Data;
            var via = f.ViaDeviceId != null ? app.DeviceName(f.ViaDeviceId) : f.ViaSshHostId != null ? app.SshLabel(f.ViaSshHostId) : "";
            var local = f.LocalPort == 0 ? L.T("ai.pick.autoPort") : f.LocalPort.ToString();
            return new Option(new ContextRef(ContextRefKind.PortForward, "", e.Id, f.Label), f.Label, $"{local} → {f.RemoteHost}:{f.RemotePort} · {via}", "forward", null);
        }).ToList();
        MultiPick(app, L.T("ai.plus.forwards"), L.T("ai.pick.noForwards"), options, draft);
    }

    public static ContextRef ActivityRef() => new(ContextRefKind.Activity, "", "", L.T("ai.ref.activity"));

    // ---------- 文件或文件夹 ----------

    /// <summary>先选设备或主机，再浏览目录（设备走端到端文件通道，SSH 主机走 SFTP，与文件页同一套）</summary>
    public static void PickFile(AppModel app, ComposerDraft draft) => new FilePick(app, draft).Show();

    private sealed class FilePick : Observable
    {
        private readonly AppModel app;
        private readonly ComposerDraft draft;
        private readonly ModalSpec spec;
        private string? targetId;
        private bool isDevice;
        private IFileSource? source;
        private string path = "";
        private List<FileItem> items = new();
        private bool loading;
        private string? error;
        private string? selected;

        public FilePick(AppModel app, ComposerDraft draft)
        {
            this.app = app;
            this.draft = draft;
            spec = new ModalSpec { Width = 560, Build = Build };
        }

        public void Show() => app.ShowModal(spec);

        public bool Listed => source != null && !loading;

        private FrameworkElement Build()
        {
            var body = new Region(() => targetId == null ? Targets() : Browser()).On(this);
            return K.Col(14, new Region(() => Modals.Header(app, targetId == null ? L.T("ai.plus.files") : app.Ai.WhereName(targetId))).On(this), body);
        }

        private FrameworkElement Targets()
        {
            var rows = new List<UIElement?>();
            foreach (var o in TargetOptions(app, true))
            {
                var b = new Btn
                {
                    CornerRadius = new CornerRadius(7), Padding = new Thickness(10, 8, 10, 8), HoverBrush = Th.Hover,
                    Child = K.Row(10, K.Icon(o.Icon, 15, o.Enabled ? Th.Secondary : Th.Tertiary, 1.8),
                        K.Grow(K.Col(1, K.Text(o.Title, 13, o.Enabled ? Th.Text : Th.Tertiary), o.Subtitle == null ? null : K.Text(o.Subtitle, 11.5, Th.Tertiary))),
                        o.Dot is { } dot ? K.StatusDot(dot, 7, Th.Card) : null, K.Icon("chevronRight", 12, Th.Tertiary, 2.2)),
                };
                b.UpdateLook();
                b.IsEnabled = o.Enabled;
                var r = o.Ref;
                b.Click += () => Choose(r.Id, r.Kind == ContextRefKind.Device);
                rows.Add(b);
            }
            if (rows.Count == 0) rows.Add(K.Margin(K.Text(L.T("ai.pick.noTargets"), 13, Th.Tertiary), 10, 14, 10, 14));
            var scroll = K.Scroll(K.Col(1, rows));
            scroll.MaxHeight = 380;
            return K.Col(14, scroll, Modals.Buttons(K.SecondaryButton(L.T("common.cancel"), () => app.CloseModal(true))));
        }

        public void Choose(string id, bool device)
        {
            targetId = id;
            isDevice = device;
            source = null;
            path = "";
            items = new List<FileItem>();
            selected = null;
            error = null;
            loading = true;
            Notify();
            Ui.Run(async () =>
            {
                try
                {
                    if (id == AiHub.LocalId)
                    {
                        // 本机：直接浏览这台电脑的文件，选中后路径换成内核的格式（/C:/Users/x）
                        source = new LocalFileSource();
                    }
                    else if (device)
                    {
                        var p = await app.Peer(id);
                        source = new RemoteFileSource(FileEndpoint.ForDevice(id), p.Session.Files());
                    }
                    else
                    {
                        var s = await app.Ssh(id);
                        source = new RemoteFileSource(FileEndpoint.ForSsh(id), s.Session.Files());
                        // SSH 登录时问过密码（会换掉这张卡片），连上后把选择器放回来
                        if (!ReferenceEquals(app.Modal, spec)) app.ShowModal(spec);
                    }
                    if (targetId != id) return;
                    var home = await source.Home();
                    await Load(home);
                }
                catch (Exception e)
                {
                    if (targetId != id) return;
                    loading = false;
                    error = Fmt.Error(e);
                    Notify();
                }
            });
        }

        public async Task Load(string dir)
        {
            if (source == null) return;
            loading = true;
            error = null;
            selected = null;
            path = dir;
            Notify();
            try
            {
                items = (await source.List(dir))
                    .OrderByDescending(i => i.IsDir).ThenBy(i => i.Name, StringComparer.CurrentCultureIgnoreCase).ToList();
            }
            catch (Exception e)
            {
                items = new List<FileItem>();
                error = Fmt.Error(e);
            }
            loading = false;
            Notify();
        }

        public void SelectFile(string p)
        {
            selected = p;
            Notify();
        }

        private void Finish(ContextRef r)
        {
            draft.AddRef(r);
            app.CloseModal();
        }

        private string NameOf(string p)
        {
            var t = p.TrimEnd('/', '\\');
            var i = Math.Max(t.LastIndexOf('/'), t.LastIndexOf('\\'));
            return i >= 0 && i < t.Length - 1 ? t.Substring(i + 1) : p;
        }

        public void ChooseFile()
        {
            if (selected == null || targetId == null) return;
            Finish(new ContextRef(ContextRefKind.File, targetId, Wire(selected), NameOf(selected)));
        }

        public void ChooseFolder()
        {
            if (targetId == null || path.Length == 0) return;
            Finish(new ContextRef(ContextRefKind.Folder, targetId, Wire(path), NameOf(path)));
        }

        /// <summary>本机路径换成内核的格式（与 galaxy-host files.rs native_to_wire 一致）；远端路径本来就是</summary>
        private string Wire(string p) => targetId == AiHub.LocalId ? WirePath(p) : p;

        private FrameworkElement Browser()
        {
            var up = K.IconButton("arrowUp", L.T("ai.pick.parent"), () => { if (source != null) Ui.Run(() => Load(source.Parent(path))); });
            up.IsEnabled = source != null && !loading && source.Parent(path) != path && path.Length > 0;
            var back = K.IconButton("chevronLeft", L.T("common.back"), () => { targetId = null; Notify(); });
            var pathBar = new Border
            {
                Background = Th.CodeBg, CornerRadius = new CornerRadius(6), Padding = new Thickness(6, 2, 10, 2),
                Child = K.Row(6, back, up, K.Grow(K.Text(path, 12, Th.Secondary, mono: true))),
            };
            UIElement content;
            if (loading) content = K.Center(K.Spinner(18));
            else if (error != null) content = K.Center(K.Col(10, K.Center(K.Text(error, 12.5, Th.Danger, wrap: true)), K.Center(K.SecondaryButton(L.T("common.retry"), () =>
            {
                if (source == null && targetId != null) Choose(targetId, isDevice);
                else Ui.Run(() => Load(path));
            }))));
            else if (items.Count == 0) content = K.Center(K.Text(L.T("ai.pick.emptyFolder"), 13, Th.Tertiary));
            else
            {
                var rows = new List<UIElement?>();
                foreach (var it in items)
                {
                    var sel = selected == it.Path;
                    var b = new Btn
                    {
                        CornerRadius = new CornerRadius(6), Padding = new Thickness(8, 5, 10, 5), Normal = sel ? Th.Selection : Brushes.Transparent, HoverBrush = sel ? Th.Selection : Th.Hover,
                        Child = K.Row(9, K.Icon(it.IsDir ? "folder" : "file", 14, it.IsDir ? Th.Accent : Th.Secondary, 1.8), K.Grow(K.Text(it.Name, 13)),
                            it.IsDir ? K.Icon("chevronRight", 11, Th.Tertiary, 2.2) : K.Text(Fmt.Bytes(it.Size), 11.5, Th.Tertiary)),
                    };
                    b.UpdateLook();
                    var item = it;
                    if (it.IsDir) b.Click += () => Ui.Run(() => Load(item.Path));
                    else
                    {
                        b.Click += () => SelectFile(item.Path);
                        b.DoubleClick += () => { SelectFile(item.Path); ChooseFile(); };
                    }
                    rows.Add(b);
                }
                content = K.Scroll(K.Col(1, rows));
            }
            var area = new Border { Height = 340, Child = content };
            var pickFolder = K.SecondaryButton(L.T("ai.pick.thisFolder"), ChooseFolder, icon: "folder");
            pickFolder.IsEnabled = !loading && error == null && path.Length > 0;
            var pickFile = K.PrimaryButton(L.T("ai.pick.thisFile"), ChooseFile, icon: "fileText");
            pickFile.IsEnabled = selected != null;
            return K.Col(12, pathBar, area, K.Row(8, K.SecondaryButton(L.T("common.cancel"), () => app.CloseModal(true)), K.Spacer(), pickFolder, pickFile));
        }
    }

    /// <summary>Windows 本机路径 → 内核路径：C:\Users\x → /C:/Users/x，C:\ → /C:/，\\server\share → //server/share</summary>
    public static string WirePath(string p)
    {
        if (p.StartsWith(@"\\?\UNC\", StringComparison.Ordinal)) p = @"\\" + p.Substring(8);
        else if (p.StartsWith(@"\\?\", StringComparison.Ordinal)) p = p.Substring(4);
        if (p.StartsWith(@"\\", StringComparison.Ordinal)) return "//" + p.Substring(2).Replace('\\', '/');
        if (p.Length >= 2 && p[1] == ':')
        {
            var rest = p.Substring(2).Replace('\\', '/');
            return "/" + char.ToUpperInvariant(p[0]) + ":" + (rest.Length == 0 ? "/" : rest);
        }
        return p.Replace('\\', '/');
    }

    /// <summary>selftest：打开文件选择器并直接进入某台设备的主目录；返回「列表已加载」的判断</summary>
    public static Func<bool> PickFileOn(AppModel app, ComposerDraft draft, string deviceId)
    {
        var p = new FilePick(app, draft);
        p.Show();
        p.Choose(deviceId, true);
        return () => p.Listed;
    }
}
