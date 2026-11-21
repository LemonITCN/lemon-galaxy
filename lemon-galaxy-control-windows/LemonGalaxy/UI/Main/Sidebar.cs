using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>左侧导航：标志、七个分区、底部账号</summary>
internal static class Sidebar
{
    private static readonly NavSection[] Sections = { NavSection.Hosts, NavSection.Sftp, NavSection.Ai, NavSection.Forwards, NavSection.Snippets, NavSection.Local, NavSection.Settings };

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
        }).On(app.Nav, app.Data, s, app.Worker.Changed, app.Forwards.Changed);
    }

    private static FrameworkElement Item(AppModel app, NavSection sec, bool compact)
    {
        var selected = app.Section == sec && app.ActiveTabId == null;
        var current = app.Section == sec;
        var fg = selected ? Th.Accent : Th.SidebarText;
        var content = new List<UIElement?> { K.Width(K.Center(K.Icon(Shell.SectionIcon(sec), 16, fg, 1.9)), 22) };
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
        if (sec == NavSection.Forwards)
        {
            // 正在运行的端口映射数
            var n = app.Forwards.RunningCount;
            return n > 0 ? K.Text(n.ToString(), 11.5, Th.Success, FontWeights.Medium) : null;
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
