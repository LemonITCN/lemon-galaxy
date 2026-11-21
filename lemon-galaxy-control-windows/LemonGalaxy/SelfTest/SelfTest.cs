using System.Globalization;
using System.IO;
using System.Text;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using LemonGalaxy.Core;
using LemonGalaxy.Core.Mock;
using LemonGalaxy.UI;

namespace LemonGalaxy;

/// <summary>
/// --selftest &lt;输出目录&gt; [--scale 1.5] [--only 名称片段] [--lang zh-Hans|en] [--theme light|dark]
/// 用模拟内核把主要界面离屏渲染成 PNG：浅色 / 深色 × 中文 / 英文。不显示窗口（元素不挂到任何窗口上，直接 RenderTargetBitmap），
/// 不连网络、不读写用户设置与系统安全存储。退出码：0 全部成功；1 有问题（见输出目录里的 report.txt）。
/// </summary>
internal static class SelfTest
{
    private static readonly Size WindowSize = new(1280, 800);
    private static double scale = 1;
    private static string dir = "selftest";
    private static string? only;
    private static int count;
    private static readonly List<string> Problems = new();

    public static int Run(string outputDir, string[] args)
    {
        string? Arg(string name)
        {
            var i = Array.IndexOf(args, name);
            return i >= 0 && i + 1 < args.Length ? args[i + 1] : null;
        }
        try
        {
            scale = double.TryParse(Arg("--scale"), NumberStyles.Float, CultureInfo.InvariantCulture, out var sc) && sc > 0 ? sc : 1;
            only = Arg("--only");
            dir = IOPath.GetFullPath(outputDir);
            Directory.CreateDirectory(dir);
            Log.FilePath = IOPath.Combine(dir, "ui.log");
            if (File.Exists(Log.FilePath)) File.Delete(Log.FilePath);

            var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
            L.LoadPacks();
            if (L.Packs.Count == 0) Problems.Add("没有找到语言包（Resources\\Locales）");
            if (TerminalTheme.All.Count < 2) Problems.Add("终端主题少于 2 个（Resources\\TerminalThemes）");
            var settings = AppSettings.Shared;
            settings.ResetForSelfTest();
            Th.Apply(settings);
            Styles.Install(app);
            if (Styles.LoadError != null) Problems.Add("控件模板解析失败：" + Styles.LoadError);
            CheckIcons();
            Clock.FrozenMs = 1_795_225_692_000;      // 2026-11-21 09:48:12（北京时间）
            var fakeHome = MakeFakeHome();
            LocalFileSource.HomeOverride = fakeHome;

            var langs = Arg("--lang") is { } l ? new[] { l } : new[] { "zh-Hans", "en" };
            var themes = Arg("--theme") is { } t ? new[] { t } : new[] { "light", "dark" };
            foreach (var lang in langs)
                foreach (var theme in themes)
                {
                    try { RenderAll(settings, lang, theme); }
                    catch (Exception e) { Problems.Add($"渲染 {lang}-{theme} 出错：{e}"); }
                }
            IconSheet();
            try { Directory.Delete(IOPath.GetDirectoryName(fakeHome)!, true); } catch { }
        }
        catch (Exception e) { Problems.Add("自测出错：" + e); }

        var missing = L.MissingKeys;
        if (missing.Length > 0) Problems.Add("缺失的语言键：" + string.Join(", ", missing));
        if (Log.ErrorCount > 0) Problems.Add($"界面日志里有 {Log.ErrorCount} 条错误（见 ui.log）");
        var report = new StringBuilder();
        report.AppendLine("LemonGalaxy Windows selftest");
        report.AppendLine($"输出 {count} 张，尺寸 {WindowSize.Width}×{WindowSize.Height} @{scale.ToString(CultureInfo.InvariantCulture)}x");
        report.AppendLine($"系统 {AppInfo.OsVersion}，.NET {Environment.Version}");
        report.AppendLine();
        report.AppendLine($"问题（{Problems.Count}）：");
        foreach (var p in Problems) report.AppendLine(p);
        try { File.WriteAllText(IOPath.Combine(dir, "report.txt"), report.ToString(), new UTF8Encoding(false)); } catch { }
        Console.WriteLine($"selftest: 输出 {count} 张到 {dir}；问题 {Problems.Count} 个");
        foreach (var p in Problems) Console.WriteLine("问题：" + p);
        return Problems.Count == 0 && count > 0 ? 0 : 1;
    }

    private static void CheckIcons()
    {
        foreach (var n in Icons.AllNames)
        {
            try { _ = Icons.Get(n); }
            catch (Exception e) { Problems.Add($"图标 {n} 解析失败：{e.Message}"); }
        }
    }

    /// <summary>本机一侧用的假主目录（文件名、时间都固定）</summary>
    private static string MakeFakeHome()
    {
        var baseDir = IOPath.Combine(IOPath.GetTempPath(), "lemongalaxy-selftest-" + Environment.TickCount);
        var home = IOPath.Combine(baseDir, "alice");
        Directory.CreateDirectory(home);
        var t = DateTimeOffset.FromUnixTimeMilliseconds(1_795_225_692_000).LocalDateTime;
        var items = new (string name, bool dir, int size, double daysAgo)[]
        {
            ("Desktop", true, 0, 2), ("Documents", true, 0, 5), ("Downloads", true, 0, 0.2), ("Projects", true, 0, 9), (".ssh", true, 0, 40), ("Pictures", true, 0, 60),
            ("deploy-notes.md", false, 2430, 1), ("galaxy-server.toml", false, 812, 3), ("report-2026Q3.pdf", false, 1_284_112, 12),
            ("screenshot.png", false, 482_330, 0.5), ("backup-2026-11-01.tar.gz", false, 3_412_000, 20),
        };
        foreach (var (name, isDir, size, days) in items)
        {
            var p = IOPath.Combine(home, name);
            if (isDir)
            {
                Directory.CreateDirectory(p);
                Directory.SetLastWriteTime(p, t.AddDays(-days));
            }
            else
            {
                File.WriteAllBytes(p, new byte[size]);
                File.SetLastWriteTime(p, t.AddDays(-days));
            }
        }
        return home;
    }

    // ---------- 渲染 ----------

    private static void Settle(FrameworkElement root, Size size)
    {
        for (var i = 0; i < 4; i++)
        {
            Ui.DoEvents();
            root.Measure(size);
            root.Arrange(new Rect(size));
            root.UpdateLayout();
        }
        Ui.Pump(40);
        root.Measure(size);
        root.Arrange(new Rect(size));
        root.UpdateLayout();
    }

    private static void Capture(string name, FrameworkElement root, Size? sizeOpt = null)
    {
        if (only != null && !name.Contains(only)) return;
        var size = sizeOpt ?? WindowSize;
        try
        {
            Settle(root, size);
            var rtb = new RenderTargetBitmap((int)Math.Round(size.Width * scale), (int)Math.Round(size.Height * scale), 96 * scale, 96 * scale, PixelFormats.Pbgra32);
            var bg = new DrawingVisual();
            using (var dc = bg.RenderOpen()) dc.DrawRectangle(Th.Content, null, new Rect(size));
            rtb.Render(bg);
            rtb.Render(root);
            var enc = new PngBitmapEncoder();
            enc.Frames.Add(BitmapFrame.Create(rtb));
            using var fs = File.Create(IOPath.Combine(dir, name + ".png"));
            enc.Save(fs);
            count++;
        }
        catch (Exception e) { Problems.Add($"渲染 {name} 失败：{e.Message}"); }
    }

    private static bool Wait(string what, Func<bool> cond, int timeoutMs = 8000)
    {
        if (Ui.WaitFor(cond, timeoutMs)) return true;
        Problems.Add("等待超时：" + what);
        return false;
    }

    private static void RenderAll(AppSettings settings, string lang, string theme)
    {
        settings.ResetForSelfTest();
        settings.Language = lang;
        settings.Appearance = theme;
        settings.Palette = "galaxy";
        settings.TerminalTheme = theme == "dark" ? "galaxy-dark" : "galaxy-light";
        L.Apply(lang);
        Th.Apply(settings);
        var prefix = (lang == "en" ? "en" : "zh") + "-" + theme;
        var zh = lang != "en";
        MockTerminalRegistry.Shared.Reset();
        var app = new AppModel(settings, selfTest: true, forceMock: true, new MockOptions { LatencyScale = 0, LiveSimulation = false });
        app.Worker.Fake(WorkerState.Running);
        const string server = "galaxy.lemonit.cn:7443#sha256-Q2F0Y2hNZUlmWW91Q2FuX0xlbW9uR2FsYXh5X1Rlc3Q";

        // 1. 登录三步
        Capture($"{prefix}-01-login", LoginView.Build(app, m => { m.Server = server; m.Username = "alice"; m.Password = "password"; }));
        Capture($"{prefix}-02-login-totp", LoginView.Build(app, m => { m.Server = server; m.Username = "alice-totp"; m.Password = "x"; m.Step = LoginStep.Totp; m.Totp = "48213"; }));
        Capture($"{prefix}-03-login-change-password", LoginView.Build(app, m => { m.Server = server; m.Username = "new-user"; m.Step = LoginStep.ChangePassword; m.NewPassword = "Galaxy-2026"; m.ConfirmPassword = "Galaxy-202"; }));
        Capture($"{prefix}-04-login-invalid-server", LoginView.Build(app, m => { m.Server = "galaxy.lemonit.cn:7443"; m.Username = "alice"; m.Error = zh ? "用户名或密码错误" : "Incorrect username or password"; }));

        // 登录（模拟内核立即返回）
        var loggedIn = false;
        Ui.Run(async () =>
        {
            await app.Client.Login(server, "alice", "pw", null);
            app.DidLogin();
            loggedIn = true;
        });
        Wait("登录", () => loggedIn);
        var root = Shell.Root(app);

        // 2. 设备与主机
        app.SelectHome(NavSection.Hosts);
        Capture($"{prefix}-10-hosts", root);
        app.Selection = "dev:dev-office";
        app.Nav.Notify();
        Capture($"{prefix}-11-hosts-device", root);
        app.Selection = "s-dbp";
        app.Editing = new EditTarget.Vault("s-dbp");
        app.Nav.Notify();
        Capture($"{prefix}-12-hosts-ssh-editor", root);
        app.Editing = null;
        app.Selection = "dev:dev-office";
        app.Nav.Notify();
        app.ActivateDevice("dev-office");
        Capture($"{prefix}-13-hosts-connect-chooser", root);
        app.CloseModal();
        app.HostsTab = HostsListTab.Credentials;
        app.Selection = "c-deploy";
        app.Editing = new EditTarget.Vault("c-deploy");
        app.Nav.Notify();
        Capture($"{prefix}-14-hosts-credential", root);
        app.Editing = null;
        app.HostsTab = HostsListTab.Hosts;
        app.Selection = null;
        app.SearchText = "web";
        app.Nav.Notify();
        Capture($"{prefix}-15-hosts-search", root);
        app.SearchText = "";
        app.Nav.Notify();
        app.ShowTerminalSessions("dev-build", MockTerminalRegistry.Shared.Sessions("dev-build").Where(s => !s.Attached).ToArray());
        Capture($"{prefix}-16-terminal-reattach", root);
        app.ShowModal(new ModalSpec { Width = 460, Build = () => Modals.HostKeyCard(app, "10.0.2.10", 22, "ssh-ed25519", "SHA256:Lm9uR2FsYXh5TW9ja0hvc3RLZXlGaW5nZXJwcmludA", true, _ => { }) });
        Capture($"{prefix}-17-ssh-hostkey-changed", root);
        app.ShowModal(new ModalSpec { Width = 420, Build = () => Modals.AssistConnectCard(app) });
        Capture($"{prefix}-18-assist-connect", root);
        app.CloseModal();

        // 3. 终端
        app.OpenTerminal("dev-build");
        if (app.ActiveTab is TerminalTab term)
        {
            term.Term.ForceFocusedLook = true;
            Wait("终端打开", () => term.Phase == TerminalPhase.Running);
            Ui.Pump(80);
            foreach (var cmd in new[] { "ls -la\r", "df -h\r", "colors\r" })
            {
                term.SendText(cmd);
                Ui.Pump(80);
            }
        }
        Capture($"{prefix}-20-terminal", root);
        app.OpenSshTerminal("s-web1");
        if (app.ActiveTab is TerminalTab ssh)
        {
            ssh.Term.ForceFocusedLook = true;
            Wait("SSH 终端打开", () => ssh.Phase == TerminalPhase.Running);
            ssh.SendText("top\r");
            Ui.Pump(120);
        }
        Capture($"{prefix}-21-terminal-ssh", root);

        // 4. 文件
        app.OpenSftp("dev-build");
        if (app.ActiveTab is SftpTab sftp)
        {
            Wait("文件远端列表", () => sftp.RightPane.Items.Count > 0 && !sftp.RightPane.Loading);
            Wait("文件本机列表", () => !sftp.LeftPane.Loading);
            // 演示传输队列：一个完成、一个失败
            if (sftp.RightPane.Source is RemoteFileSource r)
            {
                var tmp = IOPath.Combine(IOPath.GetTempPath(), "lemongalaxy-selftest-dl-" + Environment.TickCount);
                Directory.CreateDirectory(tmp);
                app.Transfers.Start("config.yaml", false, r.Client, IOPath.Combine(tmp, "config.yaml"), r.Join(sftp.RightPane.Path, "app/config.yaml"), sftp.RightPane.EndpointName);
                app.Transfers.Start("backup-fail.tar.gz", false, r.Client, IOPath.Combine(tmp, "b.tgz"), r.Join(sftp.RightPane.Path, "backup-fail.tar.gz"), sftp.RightPane.EndpointName);
                Wait("传输完成", () => app.Transfers.Items.All(i => i.State is not (TransferState.Running or TransferState.Queued)));
                try { Directory.Delete(tmp, true); } catch { }
            }
            var deploy = sftp.RightPane.Items.FirstOrDefault(i => i.Name == "deploy.sh");
            if (deploy != null) { sftp.RightPane.Selection = new HashSet<string> { deploy.Path }; sftp.RightPane.Notify(); }
        }
        Capture($"{prefix}-30-sftp", root);
        if (app.ActiveTab is SftpTab sftp2 && sftp2.RightPane.Items.FirstOrDefault(i => i.Name == "deploy.sh") is { } item)
        {
            SftpViews.ShowChmod(app, sftp2.RightPane, item);
            Capture($"{prefix}-31-sftp-chmod", root);
            app.CloseModal();
        }

        // 5. 远程桌面
        app.OpenRemoteDesktop("dev-office");
        if (app.ActiveTab is RemoteTab remote)
        {
            Wait("远程画面解码", () => remote.FramesDecoded > 0, 10000);
            // 声音默认开：画面打开后自动开始
            if (!remote.AudioSupported) Problems.Add("远程桌面：对端能力里没有 audio，工具栏不显示声音开关");
            else Wait("远端声音开始播放", () => remote.AudioOn, 5000);
            remote.MeasuredFps = 30;
            remote.MeasuredKbps = 8400;
            remote.Notify();
        }
        Capture($"{prefix}-40-remote-desktop", root);
        if (app.ActiveTab is RemoteTab remote2 && remote2.AudioOn)
        {
            // 关掉声音：记进设置、停掉播放（模拟声音少一路），再打开恢复
            var playing = MockAudio.Playing;
            remote2.ToggleAudio();
            if (remote2.AudioOn || app.Settings.RemoteAudio || MockAudio.Playing != playing - 1)
                Problems.Add($"远程桌面：关掉声音后状态不对（AudioOn={remote2.AudioOn}，设置={app.Settings.RemoteAudio}，播放路数 {playing} → {MockAudio.Playing}）");
            Capture($"{prefix}-41-remote-sound-off", root);
            remote2.ToggleAudio();
            Wait("远端声音重新开始", () => remote2.AudioOn, 5000);
        }

        // 6. AI 智能体
        app.SelectHome(NavSection.Ai);
        app.Ai.LoadModels();
        Wait("AI 模型列表", () => app.Ai.Models != null);
        app.Ai.DraftTargets = new List<AgentTarget> { new(AgentTargetKind.Device, "dev-build"), new(AgentTargetKind.SshHost, "s-dbp") };
        app.Ai.DraftPrompt = zh ? "build-server-01 的 /data 快满了，帮我看看是什么占的空间，清理一下旧备份。" : "/data on build-server-01 is almost full. Find out what uses the space and clean up old backups.";
        app.Ai.Changed.Notify();
        Capture($"{prefix}-50-ai-new", root);
        app.Ai.DraftTargets = new List<AgentTarget> { new(AgentTargetKind.Device, "dev-build") };
        var conv = app.Ai.Start();
        if (conv != null) Wait("AI 确认卡片", () => conv.HasPendingApproval);
        Capture($"{prefix}-51-ai-approval", root);
        if (conv != null && conv.Items.OfType<ToolCard>().FirstOrDefault(c => c.Decision == ToolDecision.Pending) is { } card)
        {
            conv.Decide(card, new ApprovalDecision.AllowOnce());
            if (Wait("AI 提问", () => conv.Items.OfType<AskItem>().Any()))
            {
                var ask = conv.Items.OfType<AskItem>().First();
                conv.Answer(ask, ask.Options[0]);
                Wait("AI 请求密码", () => conv.ActiveSecret != null);
            }
        }
        Capture($"{prefix}-52-ai-secret", root);
        if (conv?.ActiveSecret is { } sec) conv.CancelSecret(sec);
        // 图形界面目标：截图 + 点击标注图，每步确认档
        app.SelectHome(NavSection.Ai);
        app.Ai.DraftTargets = new List<AgentTarget> { new(AgentTargetKind.Device, "dev-office") };
        app.Ai.DraftMode = AiMode.ConfirmAll;
        app.Ai.DraftModel = "1:qwen3-vl-plus";
        app.Ai.DraftPrompt = zh ? "把记事本里的内容保存一下" : "Save the Notepad content";
        if (app.Ai.Start() is { } gui) Wait("AI 图形界面确认", () => gui.HasPendingApproval);
        Capture($"{prefix}-53-ai-gui", root);

        // 7. 片段
        app.SelectedSnippet = "n-docker";
        app.SelectHome(NavSection.Snippets);
        Capture($"{prefix}-60-snippets", root);

        // 8. 本机被控（Windows 被控端系统服务）
        app.SelectHome(NavSection.Local);
        app.Worker.Fake(WorkerState.Running);
        Capture($"{prefix}-70-local-running", root);
        app.Worker.Fake(WorkerState.NotInstalled);
        Capture($"{prefix}-71-local-not-installed", root);

        // 9. 设置
        var pages = new[] { SettingsPage.Account, SettingsPage.Appearance, SettingsPage.Terminal, SettingsPage.Connection, SettingsPage.Ai, SettingsPage.About };
        for (var i = 0; i < pages.Length; i++)
        {
            app.SettingsPage = pages[i];
            app.SelectHome(NavSection.Settings);
            Capture($"{prefix}-8{i}-settings-{pages[i].ToString().ToLowerInvariant()}", root);
        }
        var qr = new Border
        {
            Background = Th.Content, Padding = new Thickness(24),
            Child = K.Row(18, new Border { Background = Brushes.White, Padding = new Thickness(4), CornerRadius = new CornerRadius(8), Child = QrImage.Image("otpauth://totp/LemonGalaxy:alice?secret=JBSWY3DPEHPK3PXPLGALAXY&issuer=LemonGalaxy", 180) },
                K.Col(8, K.Semi(L.T("account.totp"), 15), K.Text(L.T("account.totp.scan"), 13, Th.Secondary, wrap: true).Also(t => t.Width = 360), K.SelectableText("JBSWY3DPEHPK3PXPLGALAXY", 14, Th.Text, mono: true))),
        };
        Capture($"{prefix}-86-settings-totp-qr", qr, new Size(680, 260));

        // 10. 文本编辑器、紧凑侧栏
        if (app.Tabs.OfType<SftpTab>().FirstOrDefault()?.RightPane.Source is { } src)
        {
            var model = new TextEditorModel(src, "/home/deploy/app/config.yaml");
            Ui.Run(model.Load);
            Wait("编辑器读入", () => !model.Loading);
            model.Text += "  timeout: 30s\n";
            model.Notify();
            Capture($"{prefix}-90-text-editor", TextEditorWindow.Build(app, model), new Size(760, 480));
        }
        settings.SidebarCompact = true;
        settings.AutoHideList = false;
        app.Section = NavSection.Hosts;
        if (app.Tabs.OfType<TerminalTab>().FirstOrDefault() is { } t2) app.ActivateTab(t2.Id);
        Capture($"{prefix}-91-compact-terminal", root);
        settings.SidebarCompact = false;
        settings.AutoHideList = true;

        app.CloseAllTabs();
        Ui.Pump(50);
    }

    /// <summary>全部矢量图标一览（核对图标画得对不对）</summary>
    private static void IconSheet()
    {
        var names = Icons.AllNames.Distinct().OrderBy(n => n, StringComparer.Ordinal).ToList();
        var wrap = new WrapPanel { Width = 1000, Margin = new Thickness(16) };
        foreach (var n in names)
            wrap.Children.Add(new Border
            {
                Width = 118, Height = 72, Margin = new Thickness(4), Background = Th.Card, BorderBrush = Th.Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8),
                Child = K.Center(K.Col(6, K.Center(K.Icon(n, 22, Th.Text)), K.Center(K.Text(n, 11, Th.Secondary)))),
            });
        var root = new Border { Background = Th.Content, Child = wrap };
        var rows = (names.Count + 7) / 8;
        Capture("99-icons", root, new Size(1032, rows * 80 + 40));
    }
}
