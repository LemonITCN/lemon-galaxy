using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>文件管理（Termius SFTP 式）：分区列表、双栏、传输队列、对话卡片</summary>
internal static class SftpViews
{
    private const string PaneFormat = "LemonGalaxy.Files";

    // ---------- 分区 ----------

    public static FrameworkElement Middle(AppModel app)
    {
        var list = new Region(() =>
        {
            var items = new List<UIElement?> { K.Margin(K.SectionLabel(L.T("hosts.devices")), 10, 8, 0, 4) };
            foreach (var d in app.Devices.Where(x => !x.IsSelf && x.Kind != DeviceKind.Mobile))
            {
                var id = d.Id;
                items.Add(TargetRow(K.PlatformGlyph(d.Platform, 14), app.DeviceName(d.Id), d.Hostname, K.StatusDot(K.DotOf(d), 7), d.Online || d.LanSeen, () => app.OpenSftp(id)));
            }
            items.Add(K.Margin(K.SectionLabel(L.T("hosts.sshHosts")), 10, 12, 0, 4));
            foreach (var e in app.SshHosts)
            {
                if (e.Data is not VaultData.SshHost h) continue;
                var id = e.Id;
                items.Add(TargetRow(K.ProtocolBadge("SSH", Th.P.Accent, 22, 7), h.Label, $"{h.Username}@{h.Host}", null, true, () => app.OpenSftp(sshHostId: id)));
            }
            return K.Margin(K.Col(1, items), 6);
        }).On(app.Data);
        var dock = new DockPanel();
        var top = K.Col(0, Shell.ColumnHeader(L.T("nav.sftp"), K.IconButton("columns", L.T("sftp.newLocal"), () => app.OpenSftp(), size: 15)), K.HLine());
        DockPanel.SetDock(top, Dock.Top);
        dock.Children.Add(top);
        dock.Children.Add(K.Scroll(list));
        return dock;
    }

    private static FrameworkElement TargetRow(UIElement icon, string title, string subtitle, UIElement? state, bool enabled, Action a)
    {
        var b = new Btn
        {
            Height = 42, CornerRadius = new CornerRadius(6), Padding = new Thickness(10, 0, 10, 0), HoverBrush = Th.Hover,
            Child = K.Row(9, K.Width(K.Center(icon), 24), K.Grow(K.Col(1, K.Text(title, 13, Th.Text, FontWeights.Medium), K.Text(subtitle, 11.5, Th.Tertiary))), state),
        };
        b.UpdateLook();
        b.IsEnabled = enabled;
        b.Click += a;
        return b;
    }

    public static FrameworkElement Home(AppModel app)
    {
        var dock = new DockPanel();
        var queue = QueueView(app, () => true, null);
        DockPanel.SetDock(queue, Dock.Bottom);
        dock.Children.Add(queue);
        dock.Children.Add(K.EmptyState("columns", L.T("sftp.home.title"), (L.T("sftp.newLocal"), () => app.OpenSftp())));
        return dock;
    }

    // ---------- 标签 ----------

    public static FrameworkElement TabView(AppModel app, SftpTab tab)
    {
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var left = PaneView(app, tab, tab.LeftPane);
        var right = PaneView(app, tab, tab.RightPane);
        var line = K.VLine();
        Grid.SetColumn(line, 1);
        Grid.SetColumn(right, 2);
        grid.Children.Add(left);
        grid.Children.Add(line);
        grid.Children.Add(right);
        var dock = new DockPanel { Background = Th.Content };
        var queue = QueueView(app, () => tab.QueueOpen, v => { tab.QueueOpen = v; tab.Notify(); }, tab);
        DockPanel.SetDock(queue, Dock.Bottom);
        dock.Children.Add(queue);
        dock.Children.Add(grid);
        return dock;
    }

    private static FrameworkElement PaneView(AppModel app, SftpTab tab, FilePane pane)
    {
        var dock = new DockPanel();
        var header = new Region(() => PaneHeader(app, tab, pane)).On(pane, app.Data);
        DockPanel.SetDock(header, Dock.Top);
        dock.Children.Add(header);
        var footer = new Region(() =>
        {
            var vis = pane.Visible;
            var items = new List<UIElement?> { K.Text(L.T("sftp.count", vis.Count), 11.5, Th.Tertiary) };
            if (pane.Selection.Count > 0)
            {
                var size = pane.Items.Where(i => pane.Selection.Contains(i.Path) && !i.IsDir).Aggregate(0UL, (a, i) => a + i.Size);
                items.Add(K.Text(L.T("sftp.selected", pane.Selection.Count, Fmt.Bytes(size)), 11.5, Th.Tertiary));
            }
            items.Add(K.Spacer());
            if (pane.Loading) items.Add(K.Spinner(12));
            return new Border { Height = 26, Padding = new Thickness(10, 0, 10, 0), BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 1, 0, 0), Child = K.Row(10, items) };
        }).On(pane);
        DockPanel.SetDock(footer, Dock.Bottom);
        dock.Children.Add(footer);

        var columns = ColumnHeaders(pane);
        DockPanel.SetDock(columns, Dock.Top);
        dock.Children.Add(columns);

        var list = new VList(o => FileRow(pane, (FileItem)o));
        var syncing = false;
        void Sync()
        {
            syncing = true;
            var vis = pane.Visible;
            list.ItemsSource = vis;
            list.SelectedItems.Clear();
            foreach (var i in vis.Where(i => pane.Selection.Contains(i.Path))) list.SelectedItems.Add(i);
            syncing = false;
        }
        Sync();
        pane.Watch(list, () => Ui.Coalesce(list, Sync));
        list.SelectionChanged += (_, _) =>
        {
            if (syncing) return;
            pane.Selection = new HashSet<string>(list.SelectedItems.Cast<FileItem>().Select(i => i.Path));
            Ui.Coalesce(footer, footer.Refresh);
        };
        list.MouseDoubleClick += (_, e) =>
        {
            if (VList.ItemAt(e.OriginalSource as DependencyObject) is FileItem i) pane.Open(i);
        };
        list.KeyDown += (_, e) =>
        {
            var sel = list.SelectedItems.Cast<FileItem>().ToList();
            if (e.Key == Key.Enter && sel.Count == 1) { pane.Open(sel[0]); e.Handled = true; }
            else if (e.Key == Key.Back) { pane.GoUp(); e.Handled = true; }
            else if (e.Key == Key.Delete && sel.Count > 0) { AskDelete(app, pane, sel); e.Handled = true; }
            else if (e.Key == Key.F2 && sel.Count == 1) { AskRename(app, pane, sel[0]); e.Handled = true; }
            else if (e.Key == Key.F5) { pane.Refresh(); e.Handled = true; }
        };
        list.ContextMenu = new ContextMenu();
        list.ContextMenuOpening += (_, e) =>
        {
            if (VList.ItemAt(e.OriginalSource as DependencyObject) is FileItem hit && !list.SelectedItems.Contains(hit))
            {
                list.SelectedItems.Clear();
                list.SelectedItems.Add(hit);
            }
            list.ContextMenu = K.Menu(ContextMenu(app, tab, pane, list.SelectedItems.Cast<FileItem>().ToList()));
        };
        // 拖到另一栏 = 传输；从资源管理器拖进来 = 上传
        Point? dragStart = null;
        list.PreviewMouseLeftButtonDown += (_, e) => dragStart = VList.ItemAt(e.OriginalSource as DependencyObject) != null ? e.GetPosition(list) : null;
        list.PreviewMouseMove += (_, e) =>
        {
            if (dragStart == null || e.LeftButton != MouseButtonState.Pressed) return;
            var p = e.GetPosition(list);
            if (Math.Abs(p.X - dragStart.Value.X) < 6 && Math.Abs(p.Y - dragStart.Value.Y) < 6) return;
            dragStart = null;
            var side = ReferenceEquals(pane, tab.LeftPane) ? "L" : "R";
            try { DragDrop.DoDragDrop(list, new DataObject(PaneFormat, side), DragDropEffects.Copy); } catch { }
        };
        var area = new Grid { AllowDrop = true };
        area.Children.Add(list);
        var dropRing = new Border { BorderBrush = Th.Accent, BorderThickness = new Thickness(2), CornerRadius = new CornerRadius(4), Margin = new Thickness(2), Visibility = Visibility.Collapsed, IsHitTestVisible = false };
        area.Children.Add(dropRing);
        area.Children.Add(new Region(() =>
        {
            if (pane.Loading && pane.Items.Count == 0) return K.Center(K.Spinner(18));
            if (pane.Error is { } err && pane.Items.Count == 0)
                return K.Center(K.Col(10, K.Center(K.Icon("warning", 26, Th.Warning, 1.6)), K.Center(K.Text(err, 13, Th.Secondary, wrap: true).Also(t => t.TextAlignment = TextAlignment.Center)),
                    K.Center(K.SecondaryButton(L.T("common.retry"), () => Ui.Run(() => pane.Connect(pane.Endpoint))))).Also(c => c.MaxWidth = 320));
            return null;
        }).On(pane));
        void DragOver(DragEventArgs e)
        {
            var ok = e.Data.GetDataPresent(DataFormats.FileDrop) || e.Data.GetData(PaneFormat) is string s && s != (ReferenceEquals(pane, tab.LeftPane) ? "L" : "R");
            e.Effects = ok ? DragDropEffects.Copy : DragDropEffects.None;
            dropRing.Visibility = ok ? Visibility.Visible : Visibility.Collapsed;
            e.Handled = true;
        }
        area.DragEnter += (_, e) => DragOver(e);
        area.DragOver += (_, e) => DragOver(e);
        area.DragLeave += (_, _) => dropRing.Visibility = Visibility.Collapsed;
        area.Drop += (_, e) =>
        {
            dropRing.Visibility = Visibility.Collapsed;
            if (e.Data.GetData(DataFormats.FileDrop) is string[] files) tab.UploadFiles(files, pane);
            else if (e.Data.GetData(PaneFormat) is string side)
            {
                var from = side == "L" ? tab.LeftPane : tab.RightPane;
                if (ReferenceEquals(from, pane)) return;
                tab.Transfer(from.Items.Where(i => from.Selection.Contains(i.Path)).ToList(), from);
            }
            e.Handled = true;
        };
        dock.Children.Add(area);
        return dock;
    }

    private static FrameworkElement PaneHeader(AppModel app, SftpTab tab, FilePane pane)
    {
        var endpoint = K.MenuButton(K.Row(6, K.Icon(pane.IsLocal ? "laptop" : "server", 14, Th.Text), K.Text(pane.EndpointName, 13, Th.Text, FontWeights.SemiBold), K.Icon("chevronDown", 11, Th.Tertiary)), () =>
        {
            var list = new List<MenuEntry> { new(L.T("sftp.local"), () => Ui.Run(() => pane.Connect(FileEndpoint.Local)), Checked: pane.IsLocal) };
            var devs = app.Devices.Where(d => !d.IsSelf && d.Kind != DeviceKind.Mobile && (d.Online || d.LanSeen)).ToList();
            if (devs.Count > 0)
                list.Add(new(L.T("hosts.devices"), Sub: devs.Select(d => new MenuEntry(app.DeviceName(d.Id), () => Ui.Run(() => pane.Connect(FileEndpoint.ForDevice(d.Id))),
                    Checked: pane.Endpoint == FileEndpoint.ForDevice(d.Id))).ToList()));
            var hosts = app.SshHosts;
            if (hosts.Count > 0)
                list.Add(new(L.T("hosts.sshHosts"), Sub: hosts.Select(e => new MenuEntry(app.SshLabel(e.Id), () => Ui.Run(() => pane.Connect(FileEndpoint.ForSsh(e.Id))),
                    Checked: pane.Endpoint == FileEndpoint.ForSsh(e.Id))).ToList()));
            return list;
        });
        var path = PathField(pane);
        var row1 = K.Row(4, endpoint, K.Spacer(),
            K.IconButton("eye", L.T("sftp.hidden"), () => { pane.ShowHidden = !pane.ShowHidden; pane.Notify(); }, pane.ShowHidden, 14),
            K.IconButton("folderPlus", L.T("sftp.newFolder"), () => AskNewFolder(app, pane), size: 15),
            K.IconButton("refresh", L.T("common.refresh"), pane.Refresh, size: 13));
        var back = K.IconButton("chevronLeft", L.T("sftp.back"), pane.GoBack, size: 13);
        back.IsEnabled = pane.Back.Count > 0;
        var row2 = K.Row(3, back, K.IconButton("arrowUp", L.T("sftp.up"), pane.GoUp, size: 13), K.IconButton("home", L.T("sftp.home"), pane.GoHome, size: 13), K.Grow(path));
        return new Border { Padding = new Thickness(10, 8, 10, 8), Child = K.Col(6, row1, row2) };
    }

    /// <summary>路径栏：点一下变成可输入</summary>
    private static FrameworkElement PathField(FilePane pane)
    {
        var display = pane.Path;
        if (pane.IsLocal && pane.Path.Length == 0) display = L.T("sftp.thisPc");
        var f = K.TextField("", pane.Path, mono: true, fontSize: 12);
        f.Height = 26;
        f.MinHeight = 26;
        f.Visibility = Visibility.Collapsed;
        var label = K.Text(display, 12, Th.Secondary, mono: true);
        label.TextTrimming = TextTrimming.CharacterEllipsis;
        var b = new Btn
        {
            Height = 26, CornerRadius = new CornerRadius(6), Background = Th.Elevated, Normal = Th.Elevated, BorderBrush = Th.Border, BorderThickness = new Thickness(1),
            Padding = new Thickness(8, 0, 8, 0), Child = label, Cursor = Cursors.IBeam,
        };
        b.UpdateLook();
        b.Click += () =>
        {
            b.Visibility = Visibility.Collapsed;
            f.Visibility = Visibility.Visible;
            f.FocusInput();
        };
        f.Submitted += () => { var p = f.Text.Trim(); Ui.Run(() => pane.Load(p, true)); };
        f.Escaped += () => { f.Visibility = Visibility.Collapsed; b.Visibility = Visibility.Visible; };
        f.Input.LostKeyboardFocus += (_, _) => { f.Visibility = Visibility.Collapsed; b.Visibility = Visibility.Visible; };
        return new Grid { Children = { b, f } };
    }

    private static FrameworkElement ColumnHeaders(FilePane pane) => new Region(() =>
    {
        FrameworkElement H(string key, string title, double width = double.NaN)
        {
            var active = pane.SortKey == key;
            var content = K.Row(4, K.Text(title, 12, active ? Th.Text : Th.Secondary, FontWeights.Medium), active ? K.Icon(pane.Ascending ? "chevronUp" : "chevronDown", 10, Th.Secondary, 2.4) : null);
            var b = new Btn { Height = 26, Padding = new Thickness(6, 0, 6, 0), HoverBrush = Th.Hover, Child = content, CornerRadius = new CornerRadius(4) };
            b.UpdateLook();
            if (!double.IsNaN(width)) b.Width = width;
            b.Click += () => { if (pane.SortKey == key) pane.Ascending = !pane.Ascending; else { pane.SortKey = key; pane.Ascending = true; } pane.Notify(); };
            return b;
        }
        var perms = K.Margin(K.Text(L.T("sftp.perms"), 12, Th.Secondary, FontWeights.Medium), 6, 0, 0, 0);
        perms.Width = 92;
        return new Border
        {
            Padding = new Thickness(8, 0, 14, 0), BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 1, 0, 1), Background = Th.List,
            Child = K.Row(0, K.Grow(H("name", L.T("sftp.name"))), H("size", L.T("sftp.size"), 84), H("modified", L.T("sftp.modified"), 104), pane.IsLocal ? null : perms),
        };
    }).On(pane);

    private static string FileIcon(FileItem i)
    {
        if (i.IsDrive) return "server";
        if (i.IsDir) return "folderFill";
        if (i.Kind == FileKind.Symlink) return "link";
        var ext = IOPath.GetExtension(i.Name).TrimStart('.').ToLowerInvariant();
        return ext switch
        {
            "png" or "jpg" or "jpeg" or "gif" or "webp" or "heic" or "bmp" => "image",
            "zip" or "gz" or "tgz" or "xz" or "bz2" or "7z" or "tar" or "rar" => "archive",
            "sh" or "py" or "js" or "ts" or "rs" or "go" or "swift" or "c" or "cpp" or "cs" or "ps1" => "fileCode",
            "exe" or "bin" or "msi" or "dll" or "app" or "dmg" => "gear",
            _ => i.LooksLikeText ? "fileText" : "file",
        };
    }

    private static UIElement FileRow(FilePane pane, FileItem i)
    {
        var name = K.Text(pane.NameOf(i), 13);
        var nameRow = K.Row(7, K.Width(K.Center(K.Icon(FileIcon(i), 14, i.IsDir ? Th.Accent : Th.Secondary, 1.8)), 18), K.Grow(name),
            i.LinkTarget != null ? K.Text("→ " + i.LinkTarget, 11.5, Th.Tertiary) : null);
        var size = K.Text(i.IsDir && !i.IsDrive ? "—" : Fmt.Bytes(i.Size), 12, Th.Secondary);
        size.Width = 84;
        size.Padding = new Thickness(6, 0, 0, 0);
        var mod = K.Text(i.IsDrive ? "" : Fmt.ShortDate(i.Modified), 12, Th.Secondary);
        mod.Width = 104;
        mod.Padding = new Thickness(6, 0, 0, 0);
        var row = K.Row(0, K.Grow(nameRow), size, mod);
        if (!pane.IsLocal)
        {
            var perms = K.Text(i.ModeString, 11.5, Th.Secondary, mono: true);
            perms.Width = 92;
            perms.Padding = new Thickness(6, 0, 0, 0);
            row.Children.Add(perms);
        }
        return new Border { Height = 28, Padding = new Thickness(8, 0, 6, 0), Background = Brushes.Transparent, Child = row };
    }

    private static List<MenuEntry> ContextMenu(AppModel app, SftpTab tab, FilePane pane, List<FileItem> items)
    {
        var list = new List<MenuEntry>();
        var other = tab.Other(pane);
        if (items.Count == 1)
        {
            var one = items[0];
            list.Add(new(one.IsDir ? L.T("sftp.open") : L.T("sftp.edit"), () => pane.Open(one), one.IsDir || one.LooksLikeText));
        }
        if (items.Count > 0)
        {
            list.Add(new(pane.IsLocal ? L.T("sftp.uploadTo", other.EndpointName) : L.T("sftp.transferTo", other.EndpointName), () => tab.Transfer(items, pane)));
            list.Add(MenuEntry.Sep);
        }
        if (items.Count == 1 && !items[0].IsDrive)
        {
            var one = items[0];
            list.Add(new(L.T("sftp.rename") + "\tF2", () => AskRename(app, pane, one)));
            if (pane.Source?.SupportsChmod == true) list.Add(new(L.T("sftp.chmod"), () => ShowChmod(app, pane, one)));
            list.Add(new(L.T("sftp.copyPath"), () => SysClipboard.SetText(one.Path)));
            if (pane.IsLocal) list.Add(new(L.T("sftp.reveal"), () => LocalFileSource.Reveal(one.Path)));
        }
        if (items.Count > 0 && !items.Any(i => i.IsDrive))
        {
            list.Add(new(L.T("common.delete") + "\tDel", () => AskDelete(app, pane, items), Destructive: true));
            list.Add(MenuEntry.Sep);
        }
        list.Add(new(L.T("sftp.newFolder"), () => AskNewFolder(app, pane), !(pane.IsLocal && pane.Path.Length == 0)));
        list.Add(new(L.T("common.refresh") + "\tF5", pane.Refresh));
        return list;
    }

    private static void Run(AppModel app, FilePane pane, Func<IFileSource, Task> op)
    {
        if (pane.Source is not { } s) return;
        Ui.Run(async () =>
        {
            try { await op(s); }
            catch (Exception e) { app.ShowError(e); }
            pane.Refresh();
        });
    }

    private static void AskNewFolder(AppModel app, FilePane pane)
        => Modals.Prompt(app, L.T("sftp.newFolder"), "", L.T("common.create"), name => Run(app, pane, s => s.Mkdir(s.Join(pane.Path, name))), L.T("sftp.folderName"));

    private static void AskRename(AppModel app, FilePane pane, FileItem item)
        => Modals.Prompt(app, L.T("sftp.rename"), pane.NameOf(item), L.T("sftp.rename"), name => Run(app, pane, s => s.Rename(item.Path, s.Join(s.Parent(item.Path), name))));

    private static void AskDelete(AppModel app, FilePane pane, List<FileItem> items)
    {
        var msg = items.Count == 1 ? L.T("sftp.delete.one", pane.NameOf(items[0])) : L.T("sftp.delete.many", items.Count);
        if (!pane.IsLocal) msg += "\n" + L.T("sftp.delete.irreversible");
        else msg += "\n" + L.T("sftp.delete.recycle");
        app.Confirm(L.T("sftp.delete.title"), msg, L.T("common.delete"), () => Run(app, pane, async s => { foreach (var i in items) await s.Remove(i.Path, true); }));
    }

    /// <summary>权限勾选：所有者 / 组 / 其他 × 读 / 写 / 执行</summary>
    public static void ShowChmod(AppModel app, FilePane pane, FileItem item)
    {
        var mode = item.Mode & 0x1FF;
        app.ShowModal(new ModalSpec
        {
            Width = 380,
            Build = () =>
            {
                var octal = K.TextField("", Convert.ToString(mode, 8), mono: true, width: 76);
                var text = K.Text(Fmt.Mode(mode), 13, Th.Secondary, mono: true);
                Grid? grid = null;
                Region? checks = null;
                void Update(bool fromOctal)
                {
                    text.Text = Fmt.Mode(mode);
                    if (!fromOctal) octal.Text = Convert.ToString(mode, 8);
                    checks?.Refresh();
                }
                checks = new Region(() =>
                {
                    grid = new Grid();
                    for (var c = 0; c < 4; c++) grid.ColumnDefinitions.Add(new ColumnDefinition { Width = c == 0 ? new GridLength(90) : new GridLength(56) });
                    for (var r = 0; r < 4; r++) grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(28) });
                    var headers = new[] { "sftp.chmod.read", "sftp.chmod.write", "sftp.chmod.exec" };
                    for (var c = 0; c < 3; c++) Place(grid, K.Text(L.T(headers[c]), 12, Th.Secondary), 0, c + 1);
                    var who = new[] { "sftp.chmod.owner", "sftp.chmod.group", "sftp.chmod.other" };
                    for (var w = 0; w < 3; w++)
                    {
                        Place(grid, K.Text(L.T(who[w]), 13), w + 1, 0);
                        for (var bit = 0; bit < 3; bit++)
                        {
                            var mask = 1u << ((2 - w) * 3 + (2 - bit));
                            Place(grid, K.CheckBox("", (mode & mask) != 0, on => { mode = on ? mode | mask : mode & ~mask; Update(false); }), w + 1, bit + 1);
                        }
                    }
                    return grid;
                });
                octal.Changed += v =>
                {
                    try
                    {
                        var m = Convert.ToUInt32(v.Trim(), 8);
                        if (m <= 0x1FF && v.Trim().Length > 0) { mode = m; Update(true); }
                    }
                    catch { }
                };
                return K.Col(14,
                    K.Semi(L.T("sftp.chmod.title", pane.NameOf(item)), 15),
                    checks,
                    K.Row(8, K.Text(L.T("sftp.chmod.octal"), 12.5, Th.Secondary), octal, text),
                    Modals.Buttons(K.SecondaryButton(L.T("common.cancel"), () => app.CloseModal()),
                        K.PrimaryButton(L.T("common.apply"), () => { app.CloseModal(); var m = mode; Run(app, pane, s => s.Chmod(item.Path, m)); })));
            },
        });
    }

    private static void Place(Grid g, UIElement e, int row, int col)
    {
        Grid.SetRow(e, row);
        Grid.SetColumn(e, col);
        if (e is FrameworkElement fe) fe.VerticalAlignment = VerticalAlignment.Center;
        g.Children.Add(e);
    }

    // ---------- 传输队列 ----------

    public static FrameworkElement QueueView(AppModel app, Func<bool> isOpen, Action<bool>? setOpen, Observable? openSource = null)
    {
        var q = app.Transfers;
        var open = isOpen();
        var region = new Region(() =>
        {
            open = isOpen();
            var count = q.ActiveCount;
            var head = new Btn
            {
                HoverBrush = Th.Hover, Padding = new Thickness(6, 0, 6, 0), CornerRadius = new CornerRadius(5), Height = 24,
                Child = K.Row(6, K.Icon(open ? "chevronDown" : "chevronRight", 11, Th.Secondary, 2.4), K.Semi(L.T("sftp.queue"), 12.5),
                    count > 0 ? new Border { Background = Th.Accent, CornerRadius = new CornerRadius(8), Padding = new Thickness(6, 0, 6, 1), Child = K.Text(count.ToString(), 11, Th.AccentText, FontWeights.SemiBold) } : null),
            };
            head.UpdateLook();
            head.Click += () => setOpen?.Invoke(!open);
            var bar = K.Row(8, head, K.Spacer(), open && q.Items.Count > 0 ? K.LinkButton(L.T("sftp.queue.clear"), q.ClearFinished) : null);
            var col = new List<UIElement?> { new Border { Height = 32, Padding = new Thickness(8, 0, 8, 0), Child = bar } };
            if (open)
            {
                col.Add(K.HLine());
                var rows = new List<UIElement?>();
                foreach (var item in q.Items) rows.Add(TransferRow(q, item));
                if (rows.Count == 0) rows.Add(K.Margin(K.Text(L.T("sftp.queue.empty"), 12.5, Th.Tertiary), 14, 12, 14, 12));
                col.Add(new Border { Height = 136, Child = K.Scroll(K.Col(0, rows)) });
            }
            return new Border { Background = Th.List, BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 1, 0, 0), Child = K.Col(0, col) };
        }).On(q);
        if (openSource != null) openSource.Watch(region, region.Invalidate);
        return region;
    }

    private static FrameworkElement TransferRow(TransferQueue q, TransferItem item) => new Region(() =>
    {
        var color = item.State switch
        {
            TransferState.Failed => Th.P.Danger,
            TransferState.Done => Th.P.Success,
            TransferState.Paused or TransferState.Cancelled => Th.P.TertiaryText,
            _ => Th.P.Accent,
        };
        var status = item.State switch
        {
            TransferState.Queued => L.T("sftp.state.queued"),
            TransferState.Running => $"{Fmt.Bytes(item.Done)} / {Fmt.Bytes(item.Total)}" + (item.Speed > 0 ? $" · {Fmt.Bytes((ulong)item.Speed)}/s" : ""),
            TransferState.Paused => L.T("sftp.state.paused"),
            TransferState.Done => L.T("sftp.state.done"),
            TransferState.Failed => item.Error ?? "",
            _ => L.T("sftp.state.cancelled"),
        };
        var actions = new List<UIElement?>();
        switch (item.State)
        {
            case TransferState.Running:
            case TransferState.Queued:
                actions.Add(K.IconButton("pause", L.T("sftp.pause"), () => q.Pause(item), size: 11, w: 24, h: 22));
                actions.Add(K.IconButton("close", L.T("common.cancel"), () => q.Cancel(item), size: 11, w: 24, h: 22));
                break;
            case TransferState.Paused:
                actions.Add(K.IconButton("play", L.T("sftp.resume"), () => q.Resume(item), size: 11, w: 24, h: 22));
                actions.Add(K.IconButton("close", L.T("common.cancel"), () => q.Cancel(item), size: 11, w: 24, h: 22));
                break;
            case TransferState.Failed:
            case TransferState.Cancelled:
                actions.Add(K.IconButton("refresh", L.T("common.retry"), () => q.Retry(item), size: 11, w: 24, h: 22));
                break;
        }
        var statusText = K.Text(status, 11.5, Th.B(color));
        var top = K.Row(8, K.Text(item.Name, 12.5, Th.Text, FontWeights.Medium), K.Grow(K.Text(item.Upload ? L.T("sftp.to", item.EndpointName) : L.T("sftp.from", item.EndpointName), 11.5, Th.Tertiary)), statusText);
        return new Border
        {
            Padding = new Thickness(12, 6, 12, 6),
            Child = K.Row(10, K.Icon(item.Upload ? "upCircle" : "downCircle", 16, Th.B(color), 1.8), K.Grow(K.Col(5, top, K.Progress(item.Fraction, Th.B(color)))),
                K.Width(K.Row(2, actions).Also(r => r.HorizontalAlignment = HorizontalAlignment.Right), 56)),
        };
    }).On(item);
}
