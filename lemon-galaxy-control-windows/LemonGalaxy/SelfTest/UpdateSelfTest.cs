using System.IO;
using System.IO.Compression;
using System.Text;
#if NETFRAMEWORK
using System.Security.AccessControl;
using System.Security.Principal;
#endif

namespace LemonGalaxy;

/// <summary>
/// 在线更新安装部分（App/UpdateInstall.cs）的自测：在临时目录里摆出「旧程序目录」与「新程序包」，走一遍
/// 原地替换、去掉新版本不要的文件、新版本打不开时换回、文件被占用时换回、换到一半断电后恢复、改装到用户目录
/// （旧位置不动、快捷方式改到新位置、记下新位置、旧位置转去新位置）、安装位置的选择、参数拼接、启动与等待进程。
/// 不碰用户目录、不启动真的 LemonGalaxy（辅助程序启动新程序的那一步换成假的）。
/// </summary>
internal static class UpdateSelfTest
{
    private static List<string> problems = new();
    private static bool Windows => Environment.OSVersion.Platform == PlatformID.Win32NT;

    private static void Expect(bool ok, string what)
    {
        if (!ok) problems.Add("更新：" + what);
    }

    /// <summary>返回跑过的检查组数</summary>
    public static int Run(List<string> output)
    {
        problems = output;
        var root = IOPath.Combine(IOPath.GetTempPath(), "lemongalaxy-update-selftest-" + Guid.NewGuid().ToString("N").Substring(0, 8));
        var attempts = FileSwap.Attempts;
        FileSwap.Attempts = 3;
        var groups = 0;
        try
        {
            Directory.CreateDirectory(root);
            var env = MakeEnv(root, "a");
            var pkg = MakePackage(IOPath.Combine(root, "pkg"), "new", withAdded: true);
            void Group(string name, Action a)
            {
                groups++;
                try { a(); }
                catch (Exception e) { problems.Add($"更新：{name} 出错：{e}"); }
            }
            Group("更新包", () => CheckPackage(root, pkg));
            Group("原地替换", () => CheckInPlace(root, env, pkg));
            Group("新版本打不开", () => CheckStartupFailure(root, env, pkg));
            Group("换到一半恢复", () => CheckRecover(root, pkg));
            if (Windows) Group("文件被占用", () => CheckLocked(root, env, pkg));
            Group("安装位置", () => CheckTarget(root, env));
            Group("改装到用户目录", () => CheckMigrate(root, pkg));
            Group("版本与参数", CheckStrings);
            if (Windows) Group("进程与事件", () => CheckProcesses(env));
        }
        catch (Exception e) { problems.Add("更新自测出错：" + e); }
        finally
        {
            FileSwap.Attempts = attempts;
            UpdateFiles.TryDeleteDir(root);
        }
        return groups;
    }

    private static UpdateEnv MakeEnv(string root, string name)
    {
        var u = IOPath.Combine(root, "user-" + name);
        var env = new UpdateEnv
        {
            UpdatesDir = IOPath.Combine(u, "updates"),
            UserInstallDir = IOPath.Combine(u, "Programs", "LemonGalaxy"),
            PointerFile = IOPath.Combine(u, "install.json"),
            TempDir = IOPath.Combine(root, "tmp"),
            StartMenuDir = IOPath.Combine(u, "start"),
            MutexName = @"Local\LemonGalaxy.SelfTest." + Guid.NewGuid().ToString("N"),
            EventPrefix = @"Local\LemonGalaxy.SelfTestStarted.",
        };
        env.ShortcutRoots.Add(new KeyValuePair<string, bool>(env.StartMenuDir, true));
        env.ShortcutRoots.Add(new KeyValuePair<string, bool>(IOPath.Combine(u, "desktop"), false));
        return env;
    }

    // ---------- 测试用的程序目录与更新包 ----------

    /// <summary>LemonGalaxy.exe 用正在运行的这一份（要是真的 .NET 程序），其余文件内容带上标记便于核对是哪一版</summary>
    private static void Put(string dir, string rel, string tag)
    {
        var p = IOPath.Combine(dir, rel);
        Directory.CreateDirectory(IOPath.GetDirectoryName(p)!);
        if (rel == UpdatePackage.CoreName)
        {
            var b = new byte[2048];
            b[0] = (byte)'M';
            b[1] = (byte)'Z';
            var t = Encoding.ASCII.GetBytes(tag);
            Array.Copy(t, 0, b, 2, t.Length);
            File.WriteAllBytes(p, b);
        }
        else File.WriteAllText(p, tag);
    }

    private static string MakeInstall(string dir, string tag)
    {
        Directory.CreateDirectory(dir);
        File.Copy(UpdateEnv.CurrentExe, IOPath.Combine(dir, UpdatePackage.ExeName), true);
        Put(dir, UpdatePackage.CoreName, tag);
        Put(dir, IOPath.Combine("Resources", "Locales", "en.json"), tag);
        Put(dir, "LICENSE.txt", tag);
        return dir;
    }

    private static string MakePackage(string dir, string tag, bool withAdded)
    {
        MakeInstall(dir, tag);
        if (withAdded) Put(dir, IOPath.Combine("Resources", "added.json"), tag);
        return dir;
    }

    /// <summary>核心文件里的标记</summary>
    private static string Tag(string dir)
    {
        try
        {
            var b = File.ReadAllBytes(IOPath.Combine(dir, UpdatePackage.CoreName));
            var n = Array.IndexOf(b, (byte)0, 2);
            return Encoding.ASCII.GetString(b, 2, (n < 0 ? b.Length : n) - 2);
        }
        catch { return "(无)"; }
    }

    private static string Text(string dir, string rel)
    {
        try { return File.ReadAllText(IOPath.Combine(dir, rel)); }
        catch { return "(无)"; }
    }

    private static int WorkDirs(string dir) => Directory.Exists(dir) ? Directory.GetDirectories(dir, FileSwap.Prefix + "*").Length : 0;

    private static UpdatePlan Plan(string source, string target, string oldExe, bool migrate, string token) => new()
    {
        Source = source, Target = target, OldExe = oldExe, Migrate = migrate, Token = token,
        OldVersion = "1.0.0", NewVersion = "9.9.9", Sha256 = "sha-" + token,
    };

    private sealed class FakeLauncher : IUpdateLauncher
    {
        public StartOutcome Outcome = StartOutcome.Started;
        public int ExitCode;
        public Action<string>? OnStart;
        public readonly List<string> Launched = new();
        public readonly List<string> Opened = new();

        public void WaitForExit(int pid, long startTicks, UpdateLog log) { }

        public StartOutcome StartAndWait(string exe, string args, string cwd, string eventName, int timeoutMs, UpdateLog log, out int exitCode)
        {
            Launched.Add(exe + " " + args);
            OnStart?.Invoke(exe);
            exitCode = ExitCode;
            return Outcome;
        }

        public bool Start(string exe, string args, string cwd, UpdateLog log)
        {
            Opened.Add(exe + " " + args);
            return true;
        }
    }

    // ---------- 检查 ----------

    private static void CheckPackage(string root, string pkg)
    {
        Expect(UpdatePackage.Problem(pkg) == null, "完整的更新包没通过核对：" + UpdatePackage.Problem(pkg));
        var noCore = MakePackage(IOPath.Combine(root, "pkg-nocore"), "x", false);
        File.Delete(IOPath.Combine(noCore, UpdatePackage.CoreName));
        Expect(UpdatePackage.Problem(noCore)?.Contains(UpdatePackage.CoreName) == true, "缺 galaxy_ffi.dll 没发现");
        var badExe = MakePackage(IOPath.Combine(root, "pkg-badexe"), "x", false);
        Put(badExe, UpdatePackage.ExeName, new string('x', 4096));
        Expect(UpdatePackage.Problem(badExe) != null, "不是程序的 LemonGalaxy.exe 没发现");
        var noRes = MakePackage(IOPath.Combine(root, "pkg-nores"), "x", false);
        Directory.Delete(IOPath.Combine(noRes, "Resources"), true);
        Expect(UpdatePackage.Problem(noRes) != null, "缺 Resources 没发现");

        // 压缩包里是同名文件夹（与发布包一样）
        var folder = IOPath.Combine(root, "zipsrc", "LemonGalaxy-9.9.9-windows-x64");
        MakePackage(folder, "zip", true);
        var zip = IOPath.Combine(root, "LemonGalaxy-9.9.9-windows-x64.zip");
        ZipFile.CreateFromDirectory(folder, zip, CompressionLevel.Fastest, true);
        var staging = IOPath.Combine(root, "staging");
        Directory.CreateDirectory(staging);
        File.WriteAllText(IOPath.Combine(staging, "leftover.txt"), "上次的");
        var got = UpdatePackage.Extract(zip, staging);
        Expect(IOPath.GetFileName(got) == "LemonGalaxy-9.9.9-windows-x64" && Tag(got) == "zip" && UpdatePackage.Problem(got) == null, "解压后找到的包根目录不对：" + got);
        Expect(!File.Exists(IOPath.Combine(staging, "leftover.txt")), "解压前没清掉上次的暂存目录");
        Expect(UpdatePackage.ExeVersion(IOPath.Combine(got, UpdatePackage.ExeName)) is { Length: > 0 }, "读不出新程序的版本");
        Expect(UpdatePackage.Files(got).Count == 5, "包里文件数不对：" + string.Join(", ", UpdatePackage.Files(got)));
    }

    private static void CheckInPlace(string root, UpdateEnv env, string pkg)
    {
        var dir = MakeInstall(IOPath.Combine(root, "apps", "in place"), "old");
        File.WriteAllText(IOPath.Combine(dir, "user-notes.txt"), "用户自己放的");
        var exe = IOPath.Combine(dir, UpdatePackage.ExeName);
        var seenAtStart = "";
        var l = new FakeLauncher { OnStart = _ => seenAtStart = Tag(dir) };
        var log = new UpdateLog(null);
        var rc = UpdateHelper.Execute(Plan(pkg, dir, exe, false, "t1"), env, log, l);
        Expect(rc == 0, "原地替换返回 " + rc + "：" + string.Join(" | ", log.Lines));
        Expect(seenAtStart == "new", "启动新程序时文件还没换好：" + seenAtStart);
        Expect(Tag(dir) == "new" && Text(dir, IOPath.Combine("Resources", "Locales", "en.json")) == "new", "原地替换后文件不是新的");
        Expect(Text(dir, IOPath.Combine("Resources", "added.json")) == "new", "新版本新增的文件没装上");
        Expect(Text(dir, "user-notes.txt") == "用户自己放的", "用户自己放的文件被动了");
        Expect(File.Exists(IOPath.Combine(dir, FileSwap.ManifestName)), "没写文件清单");
        Expect(WorkDirs(dir) == 0, "替换完工作目录没删掉");
        Expect(l.Launched.Count == 1 && l.Launched[0].StartsWith(exe, StringComparison.Ordinal) && l.Launched[0].Contains("--update-token t1") && l.Launched[0].Contains("--updated-from"),
            "启动新程序的命令不对：" + string.Join(" | ", l.Launched));
        Expect(l.Opened.Count == 0, "成功了还打开了旧程序");
        Expect(UpdateJson.Load<UpdateResult>(env.ResultFile) is { Ok: true, Version: "9.9.9" }, "结果没记成功");
        Expect(!File.Exists(env.PointerFile), "原地替换不该记新位置");

        // 再更新一版：新版本不要了的文件（上一版清单里有）删掉，用户的文件留着
        var pkg3 = MakePackage(IOPath.Combine(root, "pkg3"), "v3", withAdded: false);
        rc = UpdateHelper.Execute(Plan(pkg3, dir, exe, false, "t1b"), env, new UpdateLog(null), new FakeLauncher());
        Expect(rc == 0 && Tag(dir) == "v3", "第二次原地替换没成功");
        Expect(!File.Exists(IOPath.Combine(dir, "Resources", "added.json")), "新版本不要了的文件没删");
        Expect(Text(dir, "user-notes.txt") == "用户自己放的", "第二次替换动了用户自己放的文件");
    }

    private static void CheckStartupFailure(string root, UpdateEnv env, string pkg)
    {
        var dir = MakeInstall(IOPath.Combine(root, "apps", "startup-fail"), "old");
        var exe = IOPath.Combine(dir, UpdatePackage.ExeName);
        Directory.CreateDirectory(env.UpdatesDir);
        File.WriteAllText(env.StartupFailureFile("t2"), "内核加载失败");
        var l = new FakeLauncher { Outcome = StartOutcome.Exited, ExitCode = 3 };
        var rc = UpdateHelper.Execute(Plan(pkg, dir, exe, false, "t2"), env, new UpdateLog(null), l);
        Expect(rc == 1, "新版本打不开时应返回失败");
        Expect(Tag(dir) == "old" && Text(dir, IOPath.Combine("Resources", "Locales", "en.json")) == "old", "新版本打不开后没换回旧文件");
        Expect(!File.Exists(IOPath.Combine(dir, "Resources", "added.json")), "换回后新版本新增的文件还在");
        Expect(WorkDirs(dir) == 0, "换回后工作目录没删掉");
        var r = UpdateJson.Load<UpdateResult>(env.ResultFile);
        Expect(r is { Ok: false, Code: "startup" } && r.Detail.Contains("内核加载失败") && r.Sha256 == "sha-t2", "失败结果不对：" + r?.Code + " " + r?.Detail);
        Expect(l.Opened.Count == 1 && l.Opened[0].StartsWith(exe, StringComparison.Ordinal) && l.Opened[0].Contains("--update-failed"), "换回后没重新打开旧程序：" + string.Join(" | ", l.Opened));
        Expect(!File.Exists(env.StartupFailureFile("t2")), "启动失败原因文件没删");

        // 更新包不齐全：一个文件都不动
        var bad = MakePackage(IOPath.Combine(root, "pkg-bad"), "bad", false);
        File.Delete(IOPath.Combine(bad, UpdatePackage.CoreName));
        l = new FakeLauncher();
        rc = UpdateHelper.Execute(Plan(bad, dir, exe, false, "t2b"), env, new UpdateLog(null), l);
        Expect(rc == 1 && Tag(dir) == "old" && l.Launched.Count == 0 && l.Opened.Count == 1, "更新包不齐全时不该动程序目录");
        Expect(UpdateJson.Load<UpdateResult>(env.ResultFile)?.Code == "package", "更新包不齐全的失败原因不对");
    }

    private static void CheckRecover(string root, string pkg)
    {
        var dir = MakeInstall(IOPath.Combine(root, "apps", "crash"), "old");
        var log = new UpdateLog(null);
        var swap = FileSwap.Begin(dir, log);
        swap.Prepare(pkg, "9.9.9");
        swap.StopAfter = 2;
        try
        {
            swap.Swap();
            problems.Add("更新：模拟中断没生效");
        }
        catch (FileSwap.SimulatedCrash) { }
        Expect(Tag(dir) == "new" || Text(dir, UpdatePackage.ExeName).Length > 0, "模拟中断前应已换了一部分");
        Expect(FileSwap.RecoverAll(dir, log), "没发现换到一半的替换");
        Expect(Tag(dir) == "old" && Text(dir, IOPath.Combine("Resources", "Locales", "en.json")) == "old" && File.Exists(IOPath.Combine(dir, UpdatePackage.ExeName)), "恢复后不是旧文件");
        Expect(!File.Exists(IOPath.Combine(dir, "Resources", "added.json")) && !File.Exists(IOPath.Combine(dir, FileSwap.ManifestName)), "恢复后新增的文件还在");
        Expect(WorkDirs(dir) == 0, "恢复后工作目录没删");
        Expect(!FileSwap.RecoverAll(dir, log), "恢复过的又恢复了一次");

        // 复制阶段就中断（还没开始挪旧文件）：直接清掉，不动程序文件
        var swap2 = FileSwap.Begin(dir, log);
        swap2.Prepare(pkg, "9.9.9");
        Expect(!FileSwap.RecoverAll(dir, log) && Tag(dir) == "old" && WorkDirs(dir) == 0, "复制阶段中断的工作目录处理不对");
    }

    /// <summary>Windows：后面的文件被别的进程占着（不允许改名）→ 前面换过的也要换回</summary>
    private static void CheckLocked(string root, UpdateEnv env, string pkg)
    {
        var dir = MakeInstall(IOPath.Combine(root, "apps", "locked"), "old");
        var exe = IOPath.Combine(dir, UpdatePackage.ExeName);
        var locked = IOPath.Combine(dir, "Resources", "Locales", "en.json");
        var l = new FakeLauncher();
        int rc;
        using (new FileStream(locked, FileMode.Open, FileAccess.Read, FileShare.Read))
            rc = UpdateHelper.Execute(Plan(pkg, dir, exe, false, "t3"), env, new UpdateLog(null), l);
        Expect(rc == 1 && l.Launched.Count == 0, "文件被占用时应失败且不启动新程序");
        Expect(Tag(dir) == "old" && Text(dir, IOPath.Combine("Resources", "Locales", "en.json")) == "old", "文件被占用失败后没换回旧文件");
        Expect(!File.Exists(IOPath.Combine(dir, "Resources", "added.json")), "文件被占用失败后新增的文件还在");
        Expect(UpdateJson.Load<UpdateResult>(env.ResultFile)?.Code == "locked", "文件被占用的失败原因不对：" + UpdateJson.Load<UpdateResult>(env.ResultFile)?.Code);
        Expect(l.Opened.Count == 1 && l.Opened[0].Contains("--update-failed"), "文件被占用失败后没重新打开旧程序");
        // 正在运行的程序（只开着、允许删除共享）不影响：改名替换照样成功
        l = new FakeLauncher();
        using (new FileStream(locked, FileMode.Open, FileAccess.Read, FileShare.Read | FileShare.Delete))
            rc = UpdateHelper.Execute(Plan(pkg, dir, exe, false, "t3b"), env, new UpdateLog(null), l);
        Expect(rc == 0 && Tag(dir) == "new", "文件开着但允许改名时替换应成功");
    }

    private static void CheckTarget(string root, UpdateEnv env)
    {
        var w = IOPath.Combine(root, "apps", "writable");
        Directory.CreateDirectory(w);
        var c = InstallTarget.Choose(w, env, null);
        Expect(!c.Migrate && UpdateFiles.Same(c.Dir, w), "可写目录应原地替换");
        Expect(Directory.GetFiles(w).Length == 0, "写入探测留下了文件");
        var t = IOPath.Combine(env.TempDir, "Temp1_LemonGalaxy.zip", "LemonGalaxy");
        Directory.CreateDirectory(t);
        c = InstallTarget.Choose(t, env, null);
        Expect(c.Migrate && UpdateFiles.Same(c.Dir, env.UserInstallDir), "临时目录里运行时应改装到用户目录");
        c = InstallTarget.Choose(env.UserInstallDir, env, null);
        Expect(!c.Migrate && UpdateFiles.Same(c.Dir, env.UserInstallDir), "已在用户目录时应原地替换");
        c = InstallTarget.Choose(w, env, new UpdateResult { Ok = false, Denied = true, Target = UpdateFiles.Norm(w) });
        Expect(c.Migrate, "上次在原目录没有权限时应改装到用户目录");
        c = InstallTarget.Choose(w, env, new UpdateResult { Ok = false, Denied = true, Target = IOPath.Combine(root, "other") });
        Expect(!c.Migrate, "别的目录没有权限不该影响这里");
#if NETFRAMEWORK
        if (Windows)
        {
            // 当前用户不能在目录里建文件（像 Program Files）
            var ro = IOPath.Combine(root, "apps", "readonly");
            var di = Directory.CreateDirectory(ro);
            var sid = WindowsIdentity.GetCurrent().User!;
            var rule = new FileSystemAccessRule(sid, FileSystemRights.CreateFiles | FileSystemRights.CreateDirectories | FileSystemRights.WriteData | FileSystemRights.AppendData | FileSystemRights.DeleteSubdirectoriesAndFiles,
                InheritanceFlags.None, PropagationFlags.None, AccessControlType.Deny);
            var sec = di.GetAccessControl();
            sec.AddAccessRule(rule);
            di.SetAccessControl(sec);
            try
            {
                c = InstallTarget.Choose(ro, env, null);
                Expect(c.Migrate && UpdateFiles.Same(c.Dir, env.UserInstallDir), "不可写的目录应改装到用户目录：" + c.Why);
            }
            finally
            {
                sec = di.GetAccessControl();
                sec.RemoveAccessRule(rule);
                di.SetAccessControl(sec);
            }
        }
#endif
    }

    private static void CheckMigrate(string root, string pkg)
    {
        var env = MakeEnv(root, "m");
        var oldDir = MakeInstall(IOPath.Combine(root, "apps", "Program Files", "LemonGalaxy"), "old");
        var oldExe = IOPath.Combine(oldDir, UpdatePackage.ExeName);
        var newExe = IOPath.Combine(env.UserInstallDir, UpdatePackage.ExeName);
        var desktop = env.ShortcutRoots[1].Key;
        var otherExe = IOPath.Combine(root, "apps", "other.exe");
        string? startLnk = null, deskLnk = null, otherLnk = null;
        if (Windows)
        {
            startLnk = IOPath.Combine(env.StartMenuDir, "Tools", "LemonGalaxy 远程控制.lnk");
            deskLnk = IOPath.Combine(desktop, "LemonGalaxy.lnk");
            otherLnk = IOPath.Combine(desktop, "Other.lnk");
            Directory.CreateDirectory(IOPath.GetDirectoryName(startLnk)!);
            Directory.CreateDirectory(desktop);
            ShellLink.Create(startLnk, oldExe, "LemonGalaxy");
            ShellLink.Create(deskLnk, oldExe, "LemonGalaxy");
            ShellLink.Create(otherLnk, otherExe, "Other");
        }
        var l = new FakeLauncher();
        var log = new UpdateLog(null);
        var rc = UpdateHelper.Execute(Plan(pkg, env.UserInstallDir, oldExe, true, "t4"), env, log, l);
        Expect(rc == 0, "改装到用户目录返回 " + rc + "：" + string.Join(" | ", log.Lines));
        Expect(Tag(env.UserInstallDir) == "new" && File.Exists(newExe), "用户目录里没装上新程序");
        Expect(Tag(oldDir) == "old" && WorkDirs(oldDir) == 0, "旧位置被动了");
        Expect(l.Launched.Count == 1 && l.Launched[0].StartsWith(newExe, StringComparison.Ordinal), "启动的不是用户目录里的新程序：" + string.Join(" | ", l.Launched));
        var ptr = InstallPointer.Read(env);
        Expect(ptr != null && UpdateFiles.Same(ptr.Exe, newExe) && ptr.Version == "9.9.9" && UpdateFiles.Same(ptr.From, oldDir), "没记下新位置");
        if (Windows)
        {
            Expect(UpdateFiles.Same(ShellLink.ReadTarget(startLnk!), newExe), "开始菜单快捷方式没改到新位置");
            Expect(UpdateFiles.Same(ShellLink.ReadTarget(deskLnk!), newExe), "桌面快捷方式没改到新位置");
            Expect(UpdateFiles.Same(ShellLink.ReadTarget(otherLnk!), otherExe), "别的程序的快捷方式被改了");
            Expect(!File.Exists(IOPath.Combine(env.StartMenuDir, "LemonGalaxy.lnk")), "开始菜单里已有指向新位置的，不该再建");
        }

        // 旧位置打开时转去新位置；新位置自己不转；新位置版本比自己旧时不转
        var own = UpdatePackage.ExeVersion(oldExe) ?? "0";
        Expect(UpdateFiles.Same(InstallPointer.RedirectTarget(env, oldExe, own), newExe), "旧位置打开时没转去新位置");
        Expect(InstallPointer.RedirectTarget(env, newExe, own) == null, "新位置自己转了");
        Expect(InstallPointer.RedirectTarget(env, oldExe, "999.0") == null, "新位置版本更旧时也转了");

        // 用户目录里再更新：原地替换并更新记录
        var pkg5 = MakePackage(IOPath.Combine(root, "pkg5"), "v5", false);
        var c = InstallTarget.Choose(env.UserInstallDir, env, null);
        var plan = Plan(pkg5, c.Dir, newExe, c.Migrate, "t5");
        plan.NewVersion = "10.0.0";
        rc = UpdateHelper.Execute(plan, env, new UpdateLog(null), new FakeLauncher());
        Expect(rc == 0 && Tag(env.UserInstallDir) == "v5" && InstallPointer.Read(env)?.Version == "10.0.0" && UpdateFiles.Same(InstallPointer.Read(env)?.From, oldDir),
            "用户目录里的再次更新不对");

        // 改装到用户目录但新版本打不开：用户目录里新放的文件拿掉，不记新位置，打开旧位置的程序
        var env2 = MakeEnv(root, "m2");
        l = new FakeLauncher { Outcome = StartOutcome.TimedOut };
        rc = UpdateHelper.Execute(Plan(pkg, env2.UserInstallDir, oldExe, true, "t6"), env2, new UpdateLog(null), l);
        Expect(rc == 1 && !File.Exists(IOPath.Combine(env2.UserInstallDir, UpdatePackage.ExeName)), "改装失败后用户目录里还有新程序");
        Expect(!File.Exists(env2.PointerFile), "改装失败还记了新位置");
        Expect(l.Opened.Count == 1 && l.Opened[0].StartsWith(oldExe, StringComparison.Ordinal), "改装失败后没打开旧位置的程序");
        Expect(Tag(oldDir) == "old", "改装失败动了旧位置");
    }

    private static void CheckStrings()
    {
        Expect(UpdateFiles.CompareVersions("0.1.0.68", "0.1.0.9") > 0, "版本比较 68 > 9");
        Expect(UpdateFiles.CompareVersions("0.1.0", "0.1.0.0") == 0, "版本比较 0.1.0 = 0.1.0.0");
        Expect(UpdateFiles.CompareVersions("v1.2", "1.10") < 0, "版本比较 1.2 < 1.10");
        var joined = UpdateStartup.JoinArgs(new[] { "--apply-update", @"C:\Users\张 三\AppData\Local\LemonGalaxy\updates\plan.json", @"C:\dir\", "a\"b", "" });
        Expect(joined == "--apply-update \"C:\\Users\\张 三\\AppData\\Local\\LemonGalaxy\\updates\\plan.json\" C:\\dir\\ \"a\\\"b\" \"\"", "命令行参数拼接不对：" + joined);
        Expect(UpdateStartup.JoinArgs(new[] { @"C:\a b\" }) == "\"C:\\a b\\\\\"", "带空格且以反斜杠结尾的参数拼接不对：" + UpdateStartup.JoinArgs(new[] { @"C:\a b\" }));
        Expect(UpdateFiles.SafeRel(@"Resources\a.json") && !UpdateFiles.SafeRel(@"..\x") && !UpdateFiles.SafeRel(@"C:\x") && !UpdateFiles.SafeRel("a.txt:Zone.Identifier") && !UpdateFiles.SafeRel(""),
            "清单路径检查不对");
        Expect(UpdateFailure.Classify(new UnauthorizedAccessException()) == ("denied", true), "没有权限的归类不对");
        Expect(UpdateFailure.Classify(new UpdateFailure("startup", "x")).Code == "startup", "失败归类不对");
    }

    /// <summary>Windows：启动进程、等它退出、超时结束它；报到事件；更新互斥量</summary>
    private static void CheckProcesses(UpdateEnv env)
    {
        var cmd = IOPath.Combine(Environment.SystemDirectory, "cmd.exe");
        using (var p = NativeProcess.Start(cmd, "/c exit 7", null))
            Expect(p.WaitOne(15_000) && p.ExitCode == 7, "启动进程拿退出码不对");
        var log = new UpdateLog(null);
        var o = ProcessLauncher.Instance.StartAndWait(cmd, "/c exit 5", Environment.SystemDirectory, env.EventPrefix + "never", 15_000, log, out var code);
        Expect(o == StartOutcome.Exited && code == 5, $"新程序启动后退出没认出来：{o} {code}");
        var started = DateTime.UtcNow;
        o = ProcessLauncher.Instance.StartAndWait(cmd, "/c ping -n 30 127.0.0.1 >nul", Environment.SystemDirectory, env.EventPrefix + "never2", 1500, log, out _);
        Expect(o == StartOutcome.TimedOut && (DateTime.UtcNow - started).TotalSeconds < 15, "新程序超时没认出来：" + o);
        Expect(ProcessLauncher.Instance.StartAndWait(IOPath.Combine(env.TempDir, "no-such.exe"), "", Environment.SystemDirectory, env.EventPrefix + "never3", 1000, log, out _) == StartOutcome.CannotStart,
            "启动不存在的程序没认出来");

        using (var e = new EventWaitHandle(false, EventResetMode.ManualReset, env.EventPrefix + "tok"))
        {
            UpdateStartup.Signal(env, "tok");
            Expect(e.WaitOne(0), "报到事件没置位");
        }

        Expect(UpdateStartup.WaitForRunningUpdate(env, 100) == UpdateStartup.RunningUpdate.None, "没有更新在装时判断错了");
        using var hold = new ManualResetEventSlim();
        using var release = new ManualResetEventSlim();
        var t = new Thread(() =>
        {
            using var m = new Mutex(false, env.MutexName);
            m.WaitOne();
            hold.Set();
            release.Wait(10_000);
            m.ReleaseMutex();
        }) { IsBackground = true };
        t.Start();
        hold.Wait(5000);
        Expect(UpdateStartup.WaitForRunningUpdate(env, 200) == UpdateStartup.RunningUpdate.StillRunning, "更新装太久时判断错了");
        ThreadPool.QueueUserWorkItem(_ => { Thread.Sleep(300); release.Set(); });
        Expect(UpdateStartup.WaitForRunningUpdate(env, 5000) == UpdateStartup.RunningUpdate.Finished, "等更新装完判断错了");
        t.Join(5000);
    }
}
