using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>左侧导航：标志、八个分区、底部账号</summary>
internal static class Sidebar
{
    private static readonly NavSection[] Sections = { NavSection.Hosts, NavSection.Sftp, NavSection.Ai, NavSection.Mcp, NavSection.Forwards, NavSection.Snippets, NavSection.Local, NavSection.Settings };

    public static FrameworkElement Build(AppModel app)
    {
        var s = app.Settings;
        return new Region(() =>
        {
            var compact = s.SidebarCompact;
            var items = new List<UIElement?>();
            // 顶部：标志 + 收起按钮（这一条也是窗口的拖动区）
            if (compact)
                items.Add(new Border { Height = Shell.BarHeight, Child = K.Center(K.AppMark(22)) });
            else
                items.Add(new Border
                {
                    Height = Shell.BarHeight, Padding = new Thickness(4, 0, 0, 0),
                    Child = K.Row(8, K.AppMark(20), K.Grow(K.Semi("LemonGalaxy", 13.5)), K.IconButton("sidebar", L.T("nav.collapse"), () => s.SidebarCompact = true, size: 14)),
                });
            if (compact)
                items.Add(K.Margin(K.Center(K.IconButton("sidebar", L.T("nav.expand"), () => s.SidebarCompact = false, size: 14)), 0, 0, 0, 4));
            items.Add(K.Gap(4));
            foreach (var sec in Sections) items.Add(Item(app, sec, compact));
            items.Add(K.Spacer());
            items.Add(AccountChip(app, compact));
            var col = K.Col(2, items);
            return new Border
            {
                Width = compact ? 58 : 200, Background = Th.Sidebar, Padding = new Thickness(compact ? 7 : 10, 0, compact ? 7 : 10, 10), Child = col,
            };
        }).On(app.Nav, app.Data, s, app.Worker.Changed, app.Forwards.Changed, app.Mcp.Changed, app.Updater);
    }

    private static FrameworkElement Item(AppModel app, NavSection sec, bool compact)
    {
        var selected = app.Section == sec && app.ActiveTabId == null;
        var current = app.Section == sec;
        var fg = selected ? Th.Accent : Th.SidebarText;
        var icon = K.Width(K.Center(K.Icon(Shell.SectionIcon(sec), 16, fg, 1.9)), 22);
        // 收起的侧栏没有地方放数字：MCP 有待批准时在图标右上角叠一个小角标
        if (compact && sec == NavSection.Mcp && app.Mcp.PendingCount > 0)
        {
            var bubble = CountBubble(app.Mcp.PendingCount, 9.5);
            bubble.HorizontalAlignment = HorizontalAlignment.Right;
            bubble.VerticalAlignment = VerticalAlignment.Top;
            bubble.Margin = new Thickness(0, -6, -9, 0);
            icon = new Grid { Width = 22, Children = { icon, bubble } };
        }
        // 收起的侧栏：有新版本时「设置」图标右上角一个小点
        if (compact && sec == NavSection.Settings && app.Updater.HasUpdate)
        {
            var dot = UpdateDot();
            dot.HorizontalAlignment = HorizontalAlignment.Right;
            dot.VerticalAlignment = VerticalAlignment.Top;
            dot.Margin = new Thickness(0, -3, -5, 0);
            icon = new Grid { Width = 22, Children = { icon, dot } };
        }
        var content = new List<UIElement?> { icon };
        if (!compact)
        {
            content.Add(K.Grow(K.Text(L.T(Shell.SectionKey(sec)), 13.5, selected ? Th.Accent : Th.SidebarText, selected ? FontWeights.SemiBold : FontWeights.Normal)));
            content.Add(Badge(app, sec));
        }
        var b = new Btn
        {
            Height = 34, CornerRadius = new CornerRadius(7), Padding = new Thickness(compact ? 0 : 10, 0, compact ? 0 : 8, 0),
            Normal = selected ? Th.SidebarSelection : current ? Th.Hover : Brushes.Transparent,
            HoverBrush = selected ? Th.SidebarSelection : Th.Hover2,
            Child = compact ? K.Center(content[0]!) : K.Row(10, content),
        };
        b.UpdateLook();
        K.Tip(b, L.T(Shell.SectionKey(sec)));
        b.Click += () => app.SelectHome(sec);
        return b;
    }

    private static FrameworkElement? Badge(AppModel app, NavSection sec)
    {
        if (sec == NavSection.Hosts)
        {
            var n = app.Devices.Count(d => d.Online && !d.IsSelf && d.Kind != DeviceKind.Mobile);
            return n > 0 ? K.Text(n.ToString(), 11.5, Th.Tertiary, FontWeights.Medium) : null;
        }
        if (sec == NavSection.Mcp)
        {
            // 等用户批准的步骤（含要输入密码的）
            var n = app.Mcp.PendingCount;
            if (n > 0) return CountBubble(n, 10.5);
            return app.Mcp.Running ? K.StatusDot(DotState.Online, 7, Th.Sidebar) : null;
        }
        if (sec == NavSection.Forwards)
        {
            // 正在运行的端口映射数
            var n = app.Forwards.RunningCount;
            return n > 0 ? K.Text(n.ToString(), 11.5, Th.Success, FontWeights.Medium) : null;
        }
        if (sec == NavSection.Settings)
        {
            // 有新版本（后台已检查到或已下载好）
            return app.Updater.HasUpdate ? UpdateDot() : null;
        }
        if (sec == NavSection.Local)
        {
            return app.Worker.State switch
            {
                WorkerState.Running => K.StatusDot(DotState.Online, 7, Th.Sidebar),
                WorkerState.Stopped or WorkerState.Starting => K.StatusDot(DotState.Busy, 7, Th.Sidebar),
                _ => null,
            };
        }
        return null;
    }

    /// <summary>有新版本的小圆点</summary>
    public static FrameworkElement UpdateDot() => new System.Windows.Shapes.Ellipse { Width = 8, Height = 8, Fill = Th.Accent, IsHitTestVisible = false };

    /// <summary>醒目的数字角标（待批准）</summary>
    private static Border CountBubble(int n, double size)
    {
        var t = K.Text(n > 99 ? "99+" : n.ToString(), size, Brushes.White, FontWeights.SemiBold);
        t.TextAlignment = TextAlignment.Center;
        return new Border
        {
            MinWidth = size + 7, Height = size + 7, CornerRadius = new CornerRadius((size + 7) / 2), Padding = new Thickness(4, 0, 4, 0),
            Background = Th.Danger, Child = K.Center(t), IsHitTestVisible = false,
        };
    }

    private static FrameworkElement AccountChip(AppModel app, bool compact)
    {
        var name = app.User?.DisplayName ?? "";
        var avatar = new Grid { Width = 28, Height = 28 };
        avatar.Children.Add(K.Avatar(name, 28));
        var dot = K.StatusDot(app.ClientState == ClientState.Online ? DotState.Online : app.ClientState == ClientState.Offline ? DotState.Busy : DotState.Offline, 7, Th.Sidebar);
        dot.HorizontalAlignment = HorizontalAlignment.Right;
        dot.VerticalAlignment = VerticalAlignment.Bottom;
        dot.Margin = new Thickness(0, 0, -2, -2);
        avatar.Children.Add(dot);
        var state = app.ClientState switch
        {
            ClientState.Online => app.Server?.Name ?? L.T("state.online"),
            ClientState.Offline => L.T("state.offline"),
            ClientState.Connecting => L.T("state.connecting"),
            _ => L.T("state.loggedOut"),
        };
        var b = new Btn
        {
            CornerRadius = new CornerRadius(8), Padding = new Thickness(compact ? 0 : 6, 6, compact ? 0 : 6, 6), HoverBrush = Th.Hover2,
            Child = compact ? K.Center(avatar) : K.Row(9, avatar, K.Grow(K.Col(1, K.Text(name, 12.5, Th.Text, FontWeights.Medium), K.Text(state, 11, Th.Tertiary)))),
        };
        b.UpdateLook();
        K.Tip(b, L.T("account.settings"));
        b.Click += () => { app.SettingsPage = SettingsPage.Account; app.SelectHome(NavSection.Settings); };
        return b;
    }
}
