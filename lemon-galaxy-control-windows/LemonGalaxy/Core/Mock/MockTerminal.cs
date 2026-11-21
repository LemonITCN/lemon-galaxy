using System.Text;

namespace LemonGalaxy.Core.Mock;

/// <summary>被控端保留的终端会话（模拟）：断线后可接续。每台设备预置两条「已断开但仍在运行」的会话。</summary>
internal sealed class MockTerminalRegistry
{
    public static readonly MockTerminalRegistry Shared = new();
    private readonly object lk = new();
    private readonly Dictionary<string, (string deviceId, TerminalSessionInfo info, string screen)> items = new();
    private readonly HashSet<string> seeded = new();

    public TerminalSessionInfo[] Sessions(string deviceId)
    {
        lock (lk)
        {
            if (seeded.Add(deviceId))
            {
                var t = MockData.Now;
                var a = new TerminalSessionInfo($"t-{deviceId}-1", "htop", t - 3_600_000L * 5, false);
                var b = new TerminalSessionInfo($"t-{deviceId}-2", "tail -f /var/log/syslog", t - 60_000L * 42, false);
                items[a.Id] = (deviceId, a, "htop");
                items[b.Id] = (deviceId, b, "tail");
            }
            return items.Values.Where(v => v.deviceId == deviceId).Select(v => v.info).OrderBy(i => i.CreatedAt).ToArray();
        }
    }

    /// <summary>selftest 每轮从同样的初始状态开始</summary>
    public void Reset() { lock (lk) { items.Clear(); seeded.Clear(); } }

    public void Register(TerminalSessionInfo info, string deviceId) { lock (lk) items[info.Id] = (deviceId, info, ""); }

    public void SetAttached(string id, bool attached)
    {
        lock (lk)
            if (items.TryGetValue(id, out var v)) items[id] = (v.deviceId, v.info with { Attached = attached }, v.screen);
    }

    public string? Screen(string id) { lock (lk) return items.TryGetValue(id, out var v) ? v.screen : null; }
    public void Remove(string id) { lock (lk) items.Remove(id); }
}

/// <summary>模拟远端 shell：本地回显、行编辑、几条常用命令的假输出。全部走 TerminalSink 的原始字节，和真内核一样。</summary>
internal sealed class MockTerminalChannel : ICoreTerminal
{
    private readonly MockClient client;
    private readonly DeviceInfo device;
    private readonly TerminalSink sink;
    private readonly string id;
    private readonly string user;
    private string cwd = "~";
    private string line = "";
    private readonly List<string> history = new();
    private int historyIndex;
    private bool passwordMode;
    private readonly List<byte> esc = new();
    private bool closed;
    private readonly SerialQueue queue = new();
    private readonly bool registered;

    public MockTerminalChannel(MockClient client, DeviceInfo device, TerminalOptions options, TerminalSink sink, string? usernameOverride = null, bool register = true)
    {
        this.client = client;
        this.device = device;
        this.sink = sink;
        user = usernameOverride ?? MockData.Username(device);
        registered = register;
        id = options.AttachSession ?? $"t-{device.Id}-{Guid.NewGuid().ToString("N").Substring(0, 6)}";
        if (register)
        {
            if (options.AttachSession == null)
                MockTerminalRegistry.Shared.Register(new TerminalSessionInfo(id, device.Platform == Platform.Windows ? "powershell" : "bash", MockData.Now, true), device.Id);
            else MockTerminalRegistry.Shared.SetAttached(id, true);
        }
        var attachScreen = options.AttachSession == null ? null : MockTerminalRegistry.Shared.Screen(options.AttachSession);
        queue.Post(() => { if (attachScreen != null) EmitAttached(attachScreen); else EmitBanner(); });
    }

    private bool IsWindows => device.Platform == Platform.Windows;
    private bool Zh => client.Zh;

    private void Out(string s) => sink.OnOutput(Encoding.UTF8.GetBytes(s.Replace("\r\n", "\n").Replace("\n", "\r\n")));

    private string Prompt
    {
        get
        {
            if (IsWindows) return $"PS C:\\Users\\{user}{(cwd == "~" ? "" : "\\" + cwd.Replace("~/", "").Replace('/', '\\'))}> ";
            var host = device.Hostname.Split('.')[0];
            return $"\x1b[1;32m{user}@{host}\x1b[0m:\x1b[1;34m{cwd}\x1b[0m$ ";
        }
    }

    private void EmitBanner()
    {
        if (IsWindows) Out("Windows PowerShell\nCopyright (C) Microsoft Corporation. All rights reserved.\n\n");
        else if (device.Platform == Platform.Macos) Out("Last login: Fri Nov 21 09:12:44 on ttys002\n");
        else
        {
            Out($"Welcome to {device.OsVersion} (GNU/Linux 6.8.0-49-generic {device.Arch})\n\n");
            Out(" * Documentation:  https://help.ubuntu.com\n");
            Out("\n  System load:  0.42               Processes:             231\n");
            Out("  Usage of /:   37.8% of 196.70GB   Users logged in:       1\n");
            Out("  Memory usage: 41%                 IPv4 address for eth0: 10.0.1.5\n\n");
            Out("Last login: Fri Nov 21 09:12:44 2026 from 10.0.0.12\n");
        }
        Out(Prompt);
    }

    private void EmitAttached(string kind)
    {
        Out("\x1b[2J\x1b[H");
        Out(Zh ? "\x1b[2m[已接续被控端保留的会话]\x1b[0m\n" : "\x1b[2m[Re-attached to the session kept on the remote device]\x1b[0m\n");
        if (kind == "tail")
            foreach (var l in MockShell.SyslogLines.Skip(MockShell.SyslogLines.Length - 8)) Out(l + "\n");
        else
        {
            Out(MockShell.TopOutput());
            Out("\n");
            Out(Prompt);
        }
    }

    public string SessionId() => id;
    public string Title() => IsWindows ? "powershell" : "bash";
    public void Write(byte[] data) => queue.Post(() => Handle(data));

    private void Handle(byte[] bytes)
    {
        if (closed) return;
        var i = 0;
        while (i < bytes.Length)
        {
            var b = bytes[i];
            if (esc.Count > 0 || b == 0x1b)
            {
                esc.Add(b);
                if (esc.Count >= 3 && esc[1] == 0x5b)
                {
                    var fin = esc[esc.Count - 1];
                    if (fin >= 0x40 && fin <= 0x7e)
                    {
                        if (fin == 0x41) HistoryUp();
                        else if (fin == 0x42) HistoryDown();
                        esc.Clear();
                    }
                }
                else if (esc.Count >= 2 && esc[1] != 0x5b && esc[1] != 0x4f) esc.Clear();
                else if (esc.Count >= 3 && esc[1] == 0x4f)
                {
                    if (esc[2] == 0x41) HistoryUp();
                    else if (esc[2] == 0x42) HistoryDown();
                    esc.Clear();
                }
                i++;
                continue;
            }
            switch (b)
            {
                case 0x0d:
                case 0x0a:
                    Out("\n");
                    var cmd = line;
                    line = "";
                    if (passwordMode)
                    {
                        passwordMode = false;
                        Out(Zh ? "sudo：已模拟（不会真的执行）\n" : "sudo: simulated (nothing was run)\n");
                        Out(Prompt);
                    }
                    else Run(cmd);
                    break;
                case 0x7f:
                case 0x08:
                    if (line.Length > 0)
                    {
                        line = line.Substring(0, line.Length - 1);
                        if (!passwordMode) Out("\b \b");
                    }
                    break;
                case 0x03:
                    Out("^C\n");
                    line = "";
                    passwordMode = false;
                    Out(Prompt);
                    break;
                case 0x04:
                    if (line.Length == 0) { Finish(0); return; }
                    break;
                case 0x0c:
                    Out("\x1b[2J\x1b[H" + Prompt + line);
                    break;
                default:
                    if (b >= 0x20)
                    {
                        var end = i + 1;
                        while (end < bytes.Length && (bytes[end] >= 0x80 || bytes[end] >= 0x20 && bytes[end] != 0x7f)) end++;
                        var chunk = Encoding.UTF8.GetString(bytes, i, end - i);
                        // 括号粘贴的包裹序列已在上面按转义序列吃掉
                        line += chunk;
                        if (!passwordMode) Out(chunk);
                        i = end;
                        continue;
                    }
                    break;
            }
            i++;
        }
    }

    private void HistoryUp()
    {
        if (history.Count == 0 || historyIndex <= 0) return;
        historyIndex--;
        ReplaceLine(history[historyIndex]);
    }

    private void HistoryDown()
    {
        if (historyIndex >= history.Count) return;
        historyIndex++;
        ReplaceLine(historyIndex < history.Count ? history[historyIndex] : "");
    }

    private void ReplaceLine(string s)
    {
        var sb = new StringBuilder();
        for (var k = 0; k < line.Length; k++) sb.Append("\b \b");
        Out(sb.ToString());
        line = s;
        Out(s);
    }

    private void Run(string raw)
    {
        var cmd = raw.Trim();
        if (cmd.Length > 0) history.Add(cmd);
        historyIndex = history.Count;
        if (cmd.Length == 0) { Out(Prompt); return; }
        var parts = cmd.Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
        switch (parts[0])
        {
            case "exit":
            case "logout":
                Finish(0);
                return;
            case "clear":
            case "cls":
                Out("\x1b[2J\x1b[H");
                break;
            case "cd":
                var target = parts.Length > 1 ? parts[1] : "~";
                if (target == "~" || target == "/") cwd = target;
                else if (target == "..") cwd = cwd.Contains("/") ? cwd.Substring(0, cwd.LastIndexOf('/')) : "~";
                else if (new[] { "projects", "logs", "Downloads", "Documents", "app" }.Contains(target)) cwd = (cwd == "~" ? "~/" : cwd + "/") + target;
                else Out($"bash: cd: {target}: No such file or directory\n");
                break;
            case "sudo":
                Out($"[sudo] password for {user}: ");
                passwordMode = true;
                return;
            default:
                Out(MockShell.Run(cmd, device, cwd, Zh).text);
                break;
        }
        Out(Prompt);
    }

    private void Finish(int code)
    {
        if (closed) return;
        closed = true;
        Out("logout\n");
        if (registered) MockTerminalRegistry.Shared.Remove(id);
        sink.OnExit(code);
    }

    public void Resize(ushort cols, ushort rows) { }

    public void Detach()
    {
        closed = true;
        if (registered) MockTerminalRegistry.Shared.SetAttached(id, false);
    }

    public void Kill()
    {
        closed = true;
        if (registered) MockTerminalRegistry.Shared.Remove(id);
    }
}

/// <summary>假命令输出，终端和 AI 的 run_command 共用。</summary>
internal static class MockShell
{
    public static readonly string[] SyslogLines =
    {
        "Nov 21 09:40:01 build-server-01 CRON[48211]: (root) CMD (command -v debian-sa1 > /dev/null && debian-sa1 1 1)",
        "Nov 21 09:41:12 build-server-01 systemd[1]: Started docker-compose@ci.service - CI stack.",
        "Nov 21 09:41:13 build-server-01 dockerd[912]: time=\"2026-11-21T09:41:13\" level=info msg=\"Container started\"",
        "Nov 21 09:42:55 build-server-01 kernel: [812345.1234] EXT4-fs (sda1): re-mounted. Quota mode: none.",
        "Nov 21 09:44:02 build-server-01 sshd[49870]: Accepted publickey for deploy from 10.0.0.12 port 53122",
        "Nov 21 09:45:30 build-server-01 systemd[1]: Starting apt-daily.service - Daily apt download activities...",
        "Nov 21 09:45:41 build-server-01 systemd[1]: apt-daily.service: Deactivated successfully.",
        "Nov 21 09:47:18 build-server-01 runner[2231]: Job ci-1842 finished in 3m12s (success)",
        "Nov 21 09:48:02 build-server-01 runner[2231]: Job ci-1843 started: build linux-x86_64",
    };

    public static string TopOutput() =>
        "top - 09:48:12 up 41 days,  3:02,  1 user,  load average: 0.42, 0.51, 0.47\n" +
        "Tasks: 231 total,   1 running, 230 sleeping,   0 stopped,   0 zombie\n" +
        "%Cpu(s):  4.1 us,  1.2 sy,  0.0 ni, 94.3 id,  0.3 wa,  0.0 hi,  0.1 si\n" +
        "MiB Mem :  31842.6 total,   9120.4 free,  13044.8 used,   9677.4 buff/cache\n" +
        "\x1b[7m    PID USER      PR  NI    VIRT    RES  %CPU  %MEM     TIME+ COMMAND        \x1b[0m\n" +
        "   2231 runner    20   0 4123456 812340  12.3   2.5 812:41.22 runner\n" +
        "    912 root      20   0 2985612 142380   3.1   0.4 201:13.05 dockerd\n" +
        "  41022 deploy    20   0 1203344 401220   2.7   1.2  12:03.88 node\n" +
        "   1189 postgres  20   0  412332  98120   1.0   0.3  88:41.10 postgres";

    public static (string text, int code) Run(string command, DeviceInfo device, string cwd, bool zh)
    {
        var parts = command.Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
        if (parts.Length == 0) return ("", 0);
        var head = parts[0];
        var windows = device.Platform == Platform.Windows;
        var u = MockData.Username(device);
        switch (head)
        {
            case "ls":
            case "dir":
            case "Get-ChildItem":
                if (windows)
                    return ("\n    Directory: C:\\Users\\alice\n\nMode                 LastWriteTime         Length Name\n----                 -------------         ------ ----\n" +
                            "d-r---        2026/11/18     10:21                Desktop\nd-r---        2026/11/20     17:02                Documents\n" +
                            "d-r---        2026/11/21      9:15                Downloads\n-a----        2026/11/19     14:40          18244 report.xlsx\n\n", 0);
                if (parts.Contains("-la") || parts.Contains("-l") || parts.Contains("-al"))
                    return ("total 48\n" +
                            "drwxr-x--- 7 deploy deploy 4096 Nov 21 09:12 \x1b[1;34m.\x1b[0m\n" +
                            "drwxr-xr-x 4 root   root   4096 Oct  2 11:40 \x1b[1;34m..\x1b[0m\n" +
                            "-rw------- 1 deploy deploy 8214 Nov 21 09:12 .bash_history\n" +
                            "drwx------ 2 deploy deploy 4096 Oct  2 11:41 \x1b[1;34m.ssh\x1b[0m\n" +
                            "drwxr-xr-x 5 deploy deploy 4096 Nov 20 18:22 \x1b[1;34mapp\x1b[0m\n" +
                            "-rw-r--r-- 1 deploy deploy 1532 Nov 19 10:04 docker-compose.yml\n" +
                            "drwxr-xr-x 2 deploy deploy 4096 Nov 21 09:40 \x1b[1;34mlogs\x1b[0m\n" +
                            "-rwxr-xr-x 1 deploy deploy  642 Nov 12 16:30 \x1b[1;32mdeploy.sh\x1b[0m\n" +
                            "drwxr-xr-x 6 deploy deploy 4096 Nov 18 21:03 \x1b[1;34mprojects\x1b[0m\n" +
                            "-rw-r--r-- 1 deploy deploy  220 Oct  2 11:40 README.md\n", 0);
                return ("\x1b[1;34mapp\x1b[0m  \x1b[1;32mdeploy.sh\x1b[0m  docker-compose.yml  \x1b[1;34mlogs\x1b[0m  \x1b[1;34mprojects\x1b[0m  README.md\n", 0);
            case "pwd":
                return ((cwd == "~" ? $"/home/{u}" : cwd.Replace("~", $"/home/{u}")) + "\n", 0);
            case "whoami": return (u + "\n", 0);
            case "hostname": return (device.Hostname + "\n", 0);
            case "uname": return ($"Linux {device.Hostname} 6.8.0-49-generic #49-Ubuntu SMP PREEMPT_DYNAMIC x86_64 GNU/Linux\n", 0);
            case "date": return ("Fri Nov 21 09:48:12 CST 2026\n", 0);
            case "uptime": return (" 09:48:12 up 41 days,  3:02,  1 user,  load average: 0.42, 0.51, 0.47\n", 0);
            case "echo": return (string.Join(" ", parts.Skip(1)) + "\n", 0);
            case "df":
                return ("Filesystem      Size  Used Avail Use% Mounted on\n/dev/sda1       197G   75G  113G  40% /\ntmpfs            16G     0   16G   0% /dev/shm\n" +
                        "/dev/sdb1       1.8T  1.6T  142G  92% /data\n/dev/sda15      105M  6.1M   99M   6% /boot/efi\n", 0);
            case "free":
                return ("               total        used        free      shared  buff/cache   available\nMem:            31Gi        12Gi       8.9Gi       412Mi       9.4Gi        18Gi\nSwap:          4.0Gi       128Mi       3.9Gi\n", 0);
            case "du":
                return ("1.1T\t/data/backups\n312G\t/data/docker\n98G\t/data/datasets\n41G\t/data/logs\n", 0);
            case "top":
            case "htop":
                return (TopOutput() + "\n", 0);
            case "journalctl":
            case "tail":
                return (string.Join("\n", SyslogLines) + "\n", 0);
            case "docker":
                return ("NAMES          STATUS                   PORTS\nci-runner      Up 6 days\npostgres       Up 6 days (healthy)      5432/tcp\n" +
                        "nginx          Up 2 hours               0.0.0.0:80->80/tcp, 0.0.0.0:443->443/tcp\nredis          Exited (137) 3 hours ago\n", 0);
            case "cat":
                if (parts.Length > 1 && parts[1].EndsWith("README.md", StringComparison.Ordinal)) return ("# build-server-01\n\nCI runner and artifact cache.\n", 0);
                if (parts.Length > 1 && parts[1].EndsWith("docker-compose.yml", StringComparison.Ordinal))
                    return ("services:\n  nginx:\n    image: nginx:1.27\n    ports: [\"80:80\", \"443:443\"]\n  redis:\n    image: redis:7\n", 0);
                return ($"cat: {(parts.Length > 1 ? parts[1] : "")}: No such file or directory\n", 1);
            case "colors":
                var s = new StringBuilder();
                for (var c = 0; c < 8; c++) s.Append($"\x1b[{30 + c}m\u2588\u2588\u2588 \x1b[0m");
                s.Append('\n');
                for (var c = 0; c < 8; c++) s.Append($"\x1b[{90 + c}m\u2588\u2588\u2588 \x1b[0m");
                s.Append('\n');
                for (var c = 0; c < 24; c++) s.Append($"\x1b[48;5;{232 + c}m \x1b[0m");
                s.Append("  \x1b[38;2;245;217;10mLemon\x1b[38;2;123;211;90mGalaxy\x1b[0m  \x1b[1m粗体\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4m下划线\x1b[0m\n");
                return (s.ToString(), 0);
            case "help":
                return ((zh ? "模拟终端可用命令：" : "Commands available in the mock terminal: ") +
                        "ls, ls -la, cd, pwd, whoami, hostname, uname, date, uptime, df, free, du, top, journalctl, docker, cat, echo, colors, clear, sudo, exit\n", 0);
            case "systemctl":
                return ("\u25cf nginx.service - A high performance web server\n     Loaded: loaded (/lib/systemd/system/nginx.service; enabled)\n     Active: \x1b[1;32mactive (running)\x1b[0m since Fri 2026-11-21 07:41:02 CST; 2h 7min ago\n", 0);
            case "rm":
            case "find":
                return ("", 0);
            default:
                if (windows) return ($"{head} : The term '{head}' is not recognized as the name of a cmdlet.\n", 1);
                return ($"bash: {head}: command not found\n", 127);
        }
    }
}
