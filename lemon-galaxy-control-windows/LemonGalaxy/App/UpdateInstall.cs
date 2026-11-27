using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Web.Script.Serialization;
using Microsoft.Win32.SafeHandles;

namespace LemonGalaxy;

// Windows 主控端在线更新的安装部分（与界面无关；设置 → 关于里点「立即更新」后由 UI/Settings/Updater.cs 调用）。
//
// 1. 主程序（旧版本）：核对下载好的压缩包 → 解压到暂存目录、核对新程序文件齐全 → 选安装位置（程序目录可写就原地换；
//    不可写或在临时目录里就改装到 %LOCALAPPDATA%\Programs\LemonGalaxy，不提权、不弹 UAC）→ 把当前版本的 exe 复制到
//    更新目录当辅助程序 → 写 plan.json → 启动「辅助程序 --apply-update plan.json」→ 记下界面状态后退出。
// 2. 辅助程序：拿更新互斥量 → 等旧进程真正退出（20 秒不退就结束它）→ 新文件先复制进程序目录里的工作目录 .lg-update-*，
//    逐个核对 SHA-256 → 逐个改名替换（旧文件改名挪进工作目录；正在运行的 exe / dll 也能改名，被别的进程占着也不怕；
//    每一步记日志，断电后能恢复）→ 启动新程序（--update-token）等它报到 → 报到了就收尾（装到用户目录时改快捷方式、
//    记下新位置）；启动后退出、两分钟没打开、或任何一步失败，都把旧文件换回来并重新打开旧程序（--update-failed）。
// 3. 程序启动时：有更新正在装就等它装完后直接退出（辅助程序会打开新程序）；程序目录里有没做完的替换就先换回旧文件再重开；
//    记下的新位置不比自己旧就转去打开那一份，免得旧位置与新位置两份同时在跑。
//
// 新旧版本之间的约定（辅助程序是旧版本，被它启动的是新版本；改动时两边都要兼容）：
//   --update-token <令牌>   辅助程序启动的。界面画出来后置位事件 Local\LemonGalaxy.UpdateStarted.<令牌>；
//                            内核加载失败时写 updates\startup-<令牌>.txt 后以退出码 3 退出（辅助程序据此换回旧版本）。
//   --updated-from <旧版本> 更新成功后打开的；--update-failed 换回旧版本后打开的（原因在 updates\result.json）。
//   --update-probe          （检查用）不开窗口，只加载内核，成功就报到并退出、失败以退出码 3 退出。

/// <summary>更新用到的目录与系统对象名。正式运行用 Default；selftest 换成临时目录，不碰用户目录。</summary>
internal sealed class UpdateEnv
{
    /// <summary>下载、暂存、辅助程序、日志、结果：%LOCALAPPDATA%\LemonGalaxy\updates</summary>
    public string UpdatesDir = "";
    /// <summary>程序目录不可写时装到这里：%LOCALAPPDATA%\Programs\LemonGalaxy</summary>
    public string UserInstallDir = "";
    /// <summary>装到用户目录后记下新位置：%LOCALAPPDATA%\LemonGalaxy\install.json</summary>
    public string PointerFile = "";
    /// <summary>临时目录：直接从压缩包里运行时程序在这下面，下次就没了，也改装到用户目录</summary>
    public string TempDir = "";
    /// <summary>用户的开始菜单「程序」：改装到用户目录后，没有指向新位置的快捷方式时在这里建一个</summary>
    public string StartMenuDir = "";
    /// <summary>找快捷方式的目录（Value：是否找子目录）</summary>
    public List<KeyValuePair<string, bool>> ShortcutRoots = new();
    public string MutexName = @"Local\LemonGalaxy.ControlUpdate";
    public string EventPrefix = @"Local\LemonGalaxy.UpdateStarted.";

    public string LogFile => IOPath.Combine(UpdatesDir, "update.log");
    public string ResultFile => IOPath.Combine(UpdatesDir, "result.json");
    public string ResumeFile => IOPath.Combine(UpdatesDir, "resume.json");
    public string PlanFile => IOPath.Combine(UpdatesDir, "plan.json");
    public string StagingDir => IOPath.Combine(UpdatesDir, "staging");
    public string MarkerFile => IOPath.Combine(UpdatesDir, "download.json");
    public string StartupFailureFile(string token) => IOPath.Combine(UpdatesDir, "startup-" + token + ".txt");

    public static UpdateEnv Default
    {
        get
        {
            var local = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            var roaming = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
            // LEMONGALAXY_UPDATES_DIR：换一个更新目录（build.sh 的更新端到端检查用，不碰本机真的更新目录）
            var updates = Environment.GetEnvironmentVariable("LEMONGALAXY_UPDATES_DIR");
            var env = new UpdateEnv
            {
                UpdatesDir = !string.IsNullOrEmpty(updates) ? updates! : IOPath.Combine(local, "LemonGalaxy", "updates"),
                UserInstallDir = IOPath.Combine(local, "Programs", "LemonGalaxy"),
                PointerFile = IOPath.Combine(local, "LemonGalaxy", "install.json"),
                TempDir = IOPath.GetTempPath(),
                StartMenuDir = Environment.GetFolderPath(Environment.SpecialFolder.Programs),
            };
            void Add(string dir, bool recursive)
            {
                if (!string.IsNullOrEmpty(dir)) env.ShortcutRoots.Add(new KeyValuePair<string, bool>(dir, recursive));
            }
            Add(env.StartMenuDir, true);
            Add(Environment.GetFolderPath(Environment.SpecialFolder.CommonPrograms), true);
            Add(Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory), false);
            Add(Environment.GetFolderPath(Environment.SpecialFolder.CommonDesktopDirectory), false);
            // 固定到任务栏 / 开始菜单的（User Pinned\TaskBar、User Pinned\StartMenu）
            if (roaming.Length > 0) Add(IOPath.Combine(roaming, "Microsoft", "Internet Explorer", "Quick Launch"), true);
            return env;
        }
    }

    /// <summary>正在运行的 exe 的完整路径</summary>
    public static string CurrentExe
    {
        get
        {
            try
            {
                var p = Assembly.GetEntryAssembly()?.Location;
                if (!string.IsNullOrEmpty(p)) return p!;
            }
            catch { }
            return IOPath.Combine(AppDomain.CurrentDomain.BaseDirectory, UpdatePackage.ExeName);
        }
    }
}

/// <summary>更新日志：updates\update.log（主程序与辅助程序都往里写，行首带进程号）</summary>
internal sealed class UpdateLog
{
    private readonly string? path;
    public readonly List<string> Lines = new();
    private static readonly int Pid = Process.GetCurrentProcess().Id;

    public UpdateLog(string? path)
    {
        this.path = path;
        if (path == null) return;
        try
        {
            Directory.CreateDirectory(IOPath.GetDirectoryName(path)!);
            var fi = new FileInfo(path);
            if (fi.Exists && fi.Length > 1_000_000)
            {
                var text = File.ReadAllText(path);
                File.WriteAllText(path, text.Substring(text.Length - 300_000), new UTF8Encoding(false));
            }
        }
        catch { }
    }

    public void Write(string s)
    {
        var line = $"{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff} [{Pid}] {s}";
        lock (Lines) Lines.Add(line);
        if (path == null) return;
        var bytes = Encoding.UTF8.GetBytes(line + "\r\n");
        for (var i = 0; i < 3; i++)
        {
            try
            {
                using var f = new FileStream(path, FileMode.Append, FileAccess.Write, FileShare.ReadWrite | FileShare.Delete);
                f.Write(bytes, 0, bytes.Length);
                return;
            }
            catch { Thread.Sleep(20); }
        }
    }
}

/// <summary>更新失败；Code 决定界面上的说法（update.fail.&lt;code&gt;）：package / corrupt / locked / denied / disk / startup / other</summary>
internal sealed class UpdateFailure : Exception
{
    public readonly string Code;
    public UpdateFailure(string code, string message, Exception? inner = null) : base(message, inner) { Code = code; }

    /// <summary>异常归类；denied 表示没有权限（下次改装到用户目录）</summary>
    public static (string Code, bool Denied) Classify(Exception e)
    {
        for (Exception? x = e; x != null; x = x.InnerException)
        {
            if (x is UpdateFailure f) return (f.Code, f.Code == "denied");
            if (x is UnauthorizedAccessException) return ("denied", true);
            if (x is IOException io)
            {
                var code = io.HResult & 0xFFFF;
                if (code is 32 or 33) return ("locked", false);     // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
                if (code is 39 or 112) return ("disk", false);      // ERROR_HANDLE_DISK_FULL / ERROR_DISK_FULL
                if (code == 5) return ("denied", true);
            }
        }
        return ("other", false);
    }
}

internal static class UpdateJson
{
    public static T? Load<T>(string path) where T : class
    {
        try { return File.Exists(path) ? new JavaScriptSerializer().Deserialize<T>(File.ReadAllText(path, Encoding.UTF8)) : null; }
        catch { return null; }
    }

    /// <summary>先写临时文件再整体替换（NTFS 上是原子的），中途断电不会留下半个文件</summary>
    public static void Save(string path, object value)
    {
        Directory.CreateDirectory(IOPath.GetDirectoryName(path)!);
        var tmp = path + ".tmp";
        File.WriteAllText(tmp, new JavaScriptSerializer().Serialize(value), new UTF8Encoding(false));
        if (!File.Exists(path))
        {
            File.Move(tmp, path);
            return;
        }
        try { File.Replace(tmp, path, null); }
        catch (Exception e) when (e is IOException or PlatformNotSupportedException)
        {
            // 不支持整体替换的文件系统（FAT 的 U 盘等）：先删再改名
            File.Delete(path);
            File.Move(tmp, path);
        }
    }
}

internal static class UpdateFiles
{
    public static string Sha256(string path)
    {
        using var s = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read | FileShare.Delete);
        using var h = SHA256.Create();
        var hash = h.ComputeHash(s);
        var sb = new StringBuilder(hash.Length * 2);
        foreach (var b in hash) sb.Append(b.ToString("x2"));
        return sb.ToString();
    }

    public static string Norm(string p)
    {
        try { p = IOPath.GetFullPath(p); } catch { }
        return p.TrimEnd('\\', '/');
    }

    public static bool Same(string? a, string? b)
        => !string.IsNullOrEmpty(a) && !string.IsNullOrEmpty(b) && string.Equals(Norm(a!), Norm(b!), StringComparison.OrdinalIgnoreCase);

    /// <summary>path 在 root 下面（不含 root 本身）</summary>
    public static bool IsUnder(string path, string root)
        => !string.IsNullOrEmpty(root) && Norm(path).StartsWith(Norm(root) + IOPath.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase);

    /// <summary>清单里读到的相对路径：不能是绝对路径、不能往上走、不能带备用数据流</summary>
    public static bool SafeRel(string rel)
        => rel.Length > 0 && !IOPath.IsPathRooted(rel) && rel.IndexOf(':') < 0
           && !rel.Split('\\', '/').Any(s => s.Length == 0 || s == ".." || s == ".");

    /// <summary>文件被占用、杀毒软件正在扫描这类一时的失败：隔一会儿再试</summary>
    public static void Retry(Action a, int attempts)
    {
        for (var i = 1; ; i++)
        {
            try
            {
                a();
                return;
            }
            catch (Exception e) when (i < attempts && e is IOException or UnauthorizedAccessException && e is not FileNotFoundException && e is not DirectoryNotFoundException)
            {
                Thread.Sleep(Math.Min(150 * i, 1000));
            }
        }
    }

    public static void Move(string from, string to)
    {
        Directory.CreateDirectory(IOPath.GetDirectoryName(to)!);
        File.Move(from, to);
    }

    public static bool TryDeleteDir(string dir)
    {
        try
        {
            if (!Directory.Exists(dir)) return true;
            foreach (var f in Directory.GetFiles(dir, "*", SearchOption.AllDirectories))
                try { File.SetAttributes(f, FileAttributes.Normal); } catch { }
            Directory.Delete(dir, true);
            return true;
        }
        catch { return false; }
    }

    public static void TryDelete(string file)
    {
        try { if (File.Exists(file)) { File.SetAttributes(file, FileAttributes.Normal); File.Delete(file); } } catch { }
    }

    /// <summary>「0.1.0.68」这类按段比较数字；有非数字段时按字符串比</summary>
    public static int CompareVersions(string a, string b)
    {
        var x = a.Trim().TrimStart('v', 'V').Split('.');
        var y = b.Trim().TrimStart('v', 'V').Split('.');
        for (var i = 0; i < Math.Max(x.Length, y.Length); i++)
        {
            var p = i < x.Length ? x[i] : "0";
            var q = i < y.Length ? y[i] : "0";
            var c = long.TryParse(p, out var m) && long.TryParse(q, out var n) ? m.CompareTo(n) : string.CompareOrdinal(p, q);
            if (c != 0) return Math.Sign(c);
        }
        return 0;
    }
}

/// <summary>更新包（压缩包里只有一个文件夹，或文件直接在根上）</summary>
internal static class UpdatePackage
{
    public const string ExeName = "LemonGalaxy.exe";
    public const string CoreName = "galaxy_ffi.dll";
    /// <summary>缺了就打不开的文件（核对新程序文件齐全）</summary>
    public static readonly string[] Required = { ExeName, CoreName };

    public static string? FindRoot(string dir)
    {
        if (File.Exists(IOPath.Combine(dir, ExeName))) return dir;
        try { return Directory.GetDirectories(dir).FirstOrDefault(d => File.Exists(IOPath.Combine(d, ExeName))); }
        catch { return null; }
    }

    /// <summary>新程序文件不齐全、不像样时返回原因</summary>
    public static string? Problem(string root)
    {
        if (!Directory.Exists(root)) return "找不到新程序目录 " + root;
        foreach (var f in Required)
        {
            var p = IOPath.Combine(root, f);
            var fi = new FileInfo(p);
            if (!fi.Exists) return "更新包里缺少 " + f;
            if (fi.Length < 1024) return "更新包里的 " + f + " 不完整（" + fi.Length + " 字节）";
            if (!IsPe(p)) return f + " 不是有效的程序文件";
        }
        try { AssemblyName.GetAssemblyName(IOPath.Combine(root, ExeName)); }
        catch (Exception e) { return ExeName + " 不是有效的 .NET 程序：" + e.Message; }
        if (!Directory.Exists(IOPath.Combine(root, "Resources"))) return "更新包里缺少 Resources 目录";
        return null;
    }

    private static bool IsPe(string path)
    {
        try
        {
            using var s = File.OpenRead(path);
            return s.ReadByte() == 'M' && s.ReadByte() == 'Z';
        }
        catch { return false; }
    }

    /// <summary>exe 的发布版本（程序集信息版本「+」前面的部分）</summary>
    public static string? ExeVersion(string exe)
    {
        try
        {
            if (!File.Exists(exe)) return null;
            var v = FileVersionInfo.GetVersionInfo(exe);
            var s = !string.IsNullOrEmpty(v.ProductVersion) ? v.ProductVersion : v.FileVersion;
            if (string.IsNullOrEmpty(s)) return null;
            var i = s!.IndexOf('+');
            return (i >= 0 ? s.Substring(0, i) : s).Trim();
        }
        catch { return null; }
    }

    /// <summary>目录里全部文件的相对路径（不含清单文件），按序</summary>
    public static List<string> Files(string root)
    {
        var full = UpdateFiles.Norm(root);
        return Directory.GetFiles(full, "*", SearchOption.AllDirectories)
            .Select(f => f.Substring(full.Length + 1))
            .Where(r => !string.Equals(r, FileSwap.ManifestName, StringComparison.OrdinalIgnoreCase))
            .OrderBy(r => r, StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    /// <summary>解压到暂存目录（先清空），返回包根目录</summary>
    public static string Extract(string zip, string staging)
    {
        if (Directory.Exists(staging) && !UpdateFiles.TryDeleteDir(staging))
            throw new UpdateFailure("locked", "清不掉上次的暂存目录 " + staging);
        // .NET Framework 4.6.1 起会拒绝解压到目标目录外面的条目
        ZipFile.ExtractToDirectory(zip, staging);
        return FindRoot(staging) ?? throw new UpdateFailure("package", "压缩包里没有 " + ExeName);
    }
}

/// <summary>辅助程序按这个装</summary>
internal sealed class UpdatePlan
{
    /// <summary>旧进程（等它退出）；PidStart 是它的启动时间（UTC ticks），PID 被复用时不误杀别的进程</summary>
    public int Pid;
    public long PidStart;
    public string OldExe = "";
    public string OldVersion = "";
    public string NewVersion = "";
    /// <summary>新程序文件（解压后的包根目录）</summary>
    public string Source = "";
    /// <summary>装到哪里</summary>
    public string Target = "";
    /// <summary>原目录不可写，改装到用户目录（旧位置不动）</summary>
    public bool Migrate;
    public string Token = "";
    /// <summary>更新包的 SHA-256（服务器给的；装失败时记下，后台不再提示同一个包）</summary>
    public string Sha256 = "";
    /// <summary>装好后删掉的压缩包</summary>
    public string ZipPath = "";
    /// <summary>打开新程序（以及失败后打开旧程序）时多带的参数（build.sh 的端到端检查带 --update-probe）</summary>
    public string[] ExtraArgs = Array.Empty<string>();
}

/// <summary>辅助程序装完写的结果（updates\result.json）</summary>
internal sealed class UpdateResult
{
    public bool Ok;
    public string Version = "";
    public string From = "";
    public string Code = "";
    public string Detail = "";
    public string Sha256 = "";
    public string Target = "";
    public bool Denied;
    public long Time;
}

/// <summary>装到用户目录后记下的新位置（%LOCALAPPDATA%\LemonGalaxy\install.json）</summary>
internal sealed class InstallPointer
{
    public string Exe = "";
    public string Version = "";
    /// <summary>从哪里搬过来的（旧位置，只做记录）</summary>
    public string From = "";
    public long Time;

    public static InstallPointer? Read(UpdateEnv env) => UpdateJson.Load<InstallPointer>(env.PointerFile);

    public static void Write(UpdateEnv env, string exe, string version, string from)
    {
        var old = Read(env);
        UpdateJson.Save(env.PointerFile, new InstallPointer
        {
            Exe = exe, Version = version, From = from.Length > 0 ? from : old?.From ?? "", Time = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
        });
    }

    /// <summary>这一份不是记下的新位置、而新位置那一份存在且版本不比这份旧时，返回新位置的 exe（转去打开它）</summary>
    public static string? RedirectTarget(UpdateEnv env, string ownExe, string ownVersion)
    {
        var p = Read(env);
        if (p == null || p.Exe.Length == 0 || UpdateFiles.Same(p.Exe, ownExe) || !File.Exists(p.Exe)) return null;
        var v = UpdatePackage.ExeVersion(p.Exe) ?? p.Version;
        return v.Length > 0 && UpdateFiles.CompareVersions(v, ownVersion) >= 0 ? p.Exe : null;
    }
}

/// <summary>装到哪里</summary>
internal static class InstallTarget
{
    /// <summary>程序目录可写就原地换；在临时目录里、不可写、或上次在这里替换时没有权限，就改装到用户目录</summary>
    public static (string Dir, bool Migrate, string Why) Choose(string baseDir, UpdateEnv env, UpdateResult? last)
    {
        var dir = UpdateFiles.Norm(baseDir);
        var user = UpdateFiles.Norm(env.UserInstallDir);
        if (UpdateFiles.Same(dir, user)) return (user, false, "已在用户目录");
        if (UpdateFiles.IsUnder(dir, env.TempDir)) return (user, true, "程序在临时目录里");
        if (last is { Ok: false, Denied: true } && UpdateFiles.Same(last.Target, dir)) return (user, true, "上次在原目录替换时没有权限");
        if (!CanWrite(dir)) return (user, true, "原目录不可写");
        return (dir, false, "原目录可写");
    }

    /// <summary>能在目录里建文件、改名、删除（替换就是这三样）</summary>
    public static bool CanWrite(string dir)
    {
        var a = IOPath.Combine(dir, ".lg-write-test-" + Guid.NewGuid().ToString("N").Substring(0, 8));
        var b = a + "-renamed";
        try
        {
            File.WriteAllText(a, "");
            File.Move(a, b);
            File.Delete(b);
            return true;
        }
        catch
        {
            UpdateFiles.TryDelete(a);
            UpdateFiles.TryDelete(b);
            return false;
        }
    }
}

/// <summary>替换日志：工作目录里的 journal.json。Status：preparing → swapping → swapped → committed / rolledback</summary>
internal sealed class SwapJournal
{
    public string Status = "preparing";
    public string Version = "";
    public List<SwapEntry> Entries = new();
    /// <summary>替换时新建的子目录（换回时删掉空的）</summary>
    public List<string> CreatedDirs = new();
}

internal sealed class SwapEntry
{
    public string Rel = "";
    /// <summary>replace（原来有，换掉）/ add（原来没有）/ remove（上一版装的、这一版没有了）</summary>
    public string Kind = "";
    public long Size;
}

/// <summary>
/// 在程序目录里换文件。新文件先复制进程序目录下的工作目录（同一个盘，之后都是改名），全部核对过再逐个改名替换；
/// 旧文件改名挪进工作目录（正在运行的 exe / dll 也能改名）。换回（Rollback）按文件实际状态做，可以重复执行，
/// 程序启动时发现没做完的（换到一半断电、辅助程序被结束）也用它换回。
/// </summary>
internal sealed class FileSwap
{
    public const string Prefix = ".lg-update-";
    /// <summary>程序目录里记下这一版装了哪些文件，下次更新时删掉新版本已经不要的</summary>
    public const string ManifestName = ".lemongalaxy-files";
    /// <summary>每个文件操作的重试次数（selftest 调小）</summary>
    public static int Attempts = 12;
    /// <summary>selftest：换到第几个就停下（模拟换到一半断电）</summary>
    public int? StopAfter;

    public readonly string Target;
    public readonly string Work;
    public SwapJournal Journal = new();
    private readonly UpdateLog log;

    private string NewDir => IOPath.Combine(Work, "new");
    private string OldDir => IOPath.Combine(Work, "old");
    private string TrashDir => IOPath.Combine(Work, "trash");
    private string JournalPath => IOPath.Combine(Work, "journal.json");

    private FileSwap(string target, string work, UpdateLog log)
    {
        Target = UpdateFiles.Norm(target);
        Work = work;
        this.log = log;
    }

    /// <summary>开始一次替换：先恢复上次没做完的、清掉做完的，再建新的工作目录</summary>
    public static FileSwap Begin(string target, UpdateLog log)
    {
        Directory.CreateDirectory(target);
        RecoverAll(target, log);
        var work = IOPath.Combine(UpdateFiles.Norm(target), Prefix + DateTime.UtcNow.ToString("yyyyMMddHHmmssfff"));
        Directory.CreateDirectory(work);
        try { File.SetAttributes(work, File.GetAttributes(work) | FileAttributes.Hidden); } catch { }
        var s = new FileSwap(target, work, log);
        s.Save();
        return s;
    }

    private void Save() => UpdateJson.Save(JournalPath, Journal);

    /// <summary>把新程序复制进工作目录并逐个核对大小与 SHA-256，写好文件清单，排好要做的替换</summary>
    public void Prepare(string source, string version)
    {
        Journal.Version = version;
        var files = UpdatePackage.Files(source);
        foreach (var rel in files)
        {
            var from = IOPath.Combine(source, rel);
            var to = IOPath.Combine(NewDir, rel);
            UpdateFiles.Retry(() =>
            {
                Directory.CreateDirectory(IOPath.GetDirectoryName(to)!);
                File.Copy(from, to, true);
            }, Attempts);
            var a = new FileInfo(from);
            var b = new FileInfo(to);
            if (a.Length != b.Length || UpdateFiles.Sha256(from) != UpdateFiles.Sha256(to))
                throw new UpdateFailure("other", "复制后核对不一致：" + rel);
        }
        File.WriteAllLines(IOPath.Combine(NewDir, ManifestName), files, new UTF8Encoding(false));
        var all = files.Concat(new[] { ManifestName }).ToList();

        // 上一版装过、这一版没有了的文件：只删清单里记过的（用户自己放进来的不动）
        var stale = new List<string>();
        var oldManifest = IOPath.Combine(Target, ManifestName);
        if (File.Exists(oldManifest))
        {
            var keep = new HashSet<string>(all, StringComparer.OrdinalIgnoreCase);
            foreach (var line in File.ReadAllLines(oldManifest))
            {
                var rel = line.Trim();
                if (UpdateFiles.SafeRel(rel) && !keep.Contains(rel) && !rel.StartsWith(Prefix, StringComparison.OrdinalIgnoreCase) && File.Exists(IOPath.Combine(Target, rel)))
                    stale.Add(rel);
            }
        }

        Journal.Entries.Clear();
        Journal.CreatedDirs.Clear();
        var created = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var rel in all)
        {
            var dst = IOPath.Combine(Target, rel);
            Journal.Entries.Add(new SwapEntry { Rel = rel, Kind = File.Exists(dst) ? "replace" : "add", Size = new FileInfo(IOPath.Combine(NewDir, rel)).Length });
            // 要新建的子目录（由浅到深）
            var parts = rel.Split('\\', '/');
            var cur = Target;
            for (var i = 0; i < parts.Length - 1; i++)
            {
                cur = IOPath.Combine(cur, parts[i]);
                if (!Directory.Exists(cur) && created.Add(cur)) Journal.CreatedDirs.Add(cur);
            }
        }
        foreach (var rel in stale) Journal.Entries.Add(new SwapEntry { Rel = rel, Kind = "remove" });
        Journal.Status = "preparing";
        Save();
        log.Write($"新文件已复制进 {Work} 并核对：{files.Count} 个；替换 {Journal.Entries.Count(e => e.Kind == "replace")}，新增 {Journal.Entries.Count(e => e.Kind == "add")}，删除 {stale.Count}");
    }

    /// <summary>逐个改名替换，换完核对每个文件都在、大小对</summary>
    public void Swap()
    {
        Journal.Status = "swapping";
        Save();
        var n = 0;
        foreach (var e in Journal.Entries)
        {
            if (StopAfter is { } stop && n++ >= stop) throw new SimulatedCrash();
            var dst = IOPath.Combine(Target, e.Rel);
            if (e.Kind is "replace" or "remove")
            {
                var old = IOPath.Combine(OldDir, e.Rel);
                UpdateFiles.Retry(() => UpdateFiles.Move(dst, old), Attempts);
            }
            if (e.Kind is "replace" or "add")
            {
                var src = IOPath.Combine(NewDir, e.Rel);
                UpdateFiles.Retry(() => UpdateFiles.Move(src, dst), Attempts);
            }
        }
        foreach (var e in Journal.Entries.Where(e => e.Kind != "remove"))
        {
            var fi = new FileInfo(IOPath.Combine(Target, e.Rel));
            if (!fi.Exists || fi.Length != e.Size) throw new UpdateFailure("other", "替换后核对不一致：" + e.Rel);
        }
        Journal.Status = "swapped";
        Save();
        log.Write("文件已替换");
    }

    /// <summary>新程序已经正常打开：确认，删掉工作目录（里面的旧文件还被占着时留到下次删）</summary>
    public void Commit()
    {
        Journal.Status = "committed";
        Save();
        // 杀毒软件可能还在扫刚挪开的旧文件：稍等再试几次
        for (var i = 0; i < 4; i++)
        {
            if (UpdateFiles.TryDeleteDir(Work)) return;
            Thread.Sleep(500);
        }
        log.Write("工作目录暂时删不掉（旧文件还被占用），下次再删：" + Work);
    }

    /// <summary>换回旧文件。按文件实际状态做，可以重复执行</summary>
    public void Rollback()
    {
        log.Write("换回旧文件…");
        if (Journal.Status is "swapping" or "swapped")
        {
            var trash = IOPath.Combine(TrashDir, DateTime.UtcNow.Ticks.ToString("x"));
            var attempts = Math.Max(Attempts, 30);
            for (var i = Journal.Entries.Count - 1; i >= 0; i--)
            {
                var e = Journal.Entries[i];
                var dst = IOPath.Combine(Target, e.Rel);
                var old = IOPath.Combine(OldDir, e.Rel);
                var neu = IOPath.Combine(NewDir, e.Rel);
                switch (e.Kind)
                {
                    case "replace":
                        // 旧文件挪走了才需要换回；新文件（可能正在运行）改名挪进回收目录
                        if (!File.Exists(old)) break;
                        if (File.Exists(dst)) UpdateFiles.Retry(() => UpdateFiles.Move(dst, IOPath.Combine(trash, e.Rel)), attempts);
                        UpdateFiles.Retry(() => UpdateFiles.Move(old, dst), attempts);
                        break;
                    case "add":
                        // 新文件已经从工作目录挪过去了才是我们放的
                        if (File.Exists(dst) && !File.Exists(neu)) UpdateFiles.Retry(() => UpdateFiles.Move(dst, IOPath.Combine(trash, e.Rel)), attempts);
                        break;
                    case "remove":
                        if (File.Exists(old) && !File.Exists(dst)) UpdateFiles.Retry(() => UpdateFiles.Move(old, dst), attempts);
                        break;
                }
            }
            foreach (var d in Enumerable.Reverse(Journal.CreatedDirs))
                try { if (Directory.Exists(d) && !Directory.EnumerateFileSystemEntries(d).Any()) Directory.Delete(d); } catch { }
        }
        Journal.Status = "rolledback";
        Save();
        UpdateFiles.TryDeleteDir(Work);
        log.Write("已换回旧文件");
    }

    /// <summary>程序目录里上次没做完的替换：换回旧文件。做完的（确认过或换回过）删掉。返回是否换回过</summary>
    public static bool RecoverAll(string target, UpdateLog log)
    {
        var rolled = false;
        string[] dirs;
        try { dirs = Directory.Exists(target) ? Directory.GetDirectories(target, Prefix + "*") : Array.Empty<string>(); }
        catch { return false; }
        foreach (var work in dirs)
        {
            var journal = UpdateJson.Load<SwapJournal>(IOPath.Combine(work, "journal.json"));
            if (journal == null)
            {
                // 日志是整体替换写的，没有日志说明还没开始挪旧文件；万一旧文件目录里有东西，宁可留着不删
                var oldDir = IOPath.Combine(work, "old");
                if (Directory.Exists(oldDir) && Directory.EnumerateFiles(oldDir, "*", SearchOption.AllDirectories).Any())
                    log.Write("工作目录没有替换日志但有旧文件，保留不动：" + work);
                else UpdateFiles.TryDeleteDir(work);
                continue;
            }
            if (journal.Status is "swapping" or "swapped")
            {
                log.Write($"发现没做完的替换（{journal.Status}，版本 {journal.Version}）：{work}");
                var s = new FileSwap(target, work, log) { Journal = journal };
                s.Rollback();
                rolled = true;
            }
            else UpdateFiles.TryDeleteDir(work);
        }
        return rolled;
    }

    /// <summary>selftest 用：模拟换到一半断电</summary>
    public sealed class SimulatedCrash : Exception
    {
        public SimulatedCrash() : base("模拟中断") { }
    }
}

internal enum StartOutcome { Started, Exited, TimedOut, CannotStart }

/// <summary>辅助程序启动、等待进程的方式（selftest 换成假的）</summary>
internal interface IUpdateLauncher
{
    /// <summary>等旧进程退出；超时就结束它</summary>
    void WaitForExit(int pid, long startTicks, UpdateLog log);
    /// <summary>启动新程序，等它置位事件（报到）、退出或超时（超时就结束它）</summary>
    StartOutcome StartAndWait(string exe, string args, string cwd, string eventName, int timeoutMs, UpdateLog log, out int exitCode);
    /// <summary>启动程序，不等</summary>
    bool Start(string exe, string args, string cwd, UpdateLog log);
}

internal sealed class ProcessLauncher : IUpdateLauncher
{
    public static readonly ProcessLauncher Instance = new();
    public int ExitWaitMs = 20_000;

    public void WaitForExit(int pid, long startTicks, UpdateLog log)
    {
        if (pid <= 0) return;
        Process p;
        try { p = Process.GetProcessById(pid); }
        catch (ArgumentException) { return; }
        using (p)
        {
            try
            {
                if (Math.Abs(p.StartTime.ToUniversalTime().Ticks - startTicks) > TimeSpan.TicksPerSecond) return;
            }
            catch (InvalidOperationException) { return; }
            catch { }
            if (p.WaitForExit(ExitWaitMs)) return;
            log.Write($"旧进程 {pid} {ExitWaitMs / 1000} 秒还没退出，结束它");
            try { p.Kill(); } catch (Exception e) { log.Write("结束旧进程失败：" + e.Message); }
            if (!p.WaitForExit(15_000)) log.Write("旧进程仍在运行，照常替换（改名替换不受占用影响）");
        }
    }

    public StartOutcome StartAndWait(string exe, string args, string cwd, string eventName, int timeoutMs, UpdateLog log, out int exitCode)
    {
        exitCode = 0;
        using var started = new EventWaitHandle(false, EventResetMode.ManualReset, eventName);
        Spawned p;
        try { p = NativeProcess.Start(exe, args, cwd); }
        catch (Exception e)
        {
            log.Write("启动新程序失败：" + e.Message);
            return StartOutcome.CannotStart;
        }
        using (p)
        {
            log.Write($"已启动新程序（进程 {p.Pid}），等它打开");
            var i = WaitHandle.WaitAny(new WaitHandle[] { started, p }, timeoutMs);
            if (i == 0 || started.WaitOne(0)) return StartOutcome.Started;
            if (i == 1)
            {
                exitCode = p.ExitCode ?? -1;
                return StartOutcome.Exited;
            }
            log.Write($"新程序 {timeoutMs / 1000} 秒内没有打开，结束它");
            p.Kill();
            p.WaitOne(10_000);
            return StartOutcome.TimedOut;
        }
    }

    public bool Start(string exe, string args, string cwd, UpdateLog log)
    {
        try
        {
            using var p = NativeProcess.Start(exe, args, cwd);
            log.Write($"已打开 {exe}（进程 {p.Pid}）");
            return true;
        }
        catch (Exception e)
        {
            log.Write($"打开 {exe} 失败：{e.Message}");
            return false;
        }
    }
}

/// <summary>辅助程序（LemonGalaxy.exe --apply-update plan.json）</summary>
internal static class UpdateHelper
{
    /// <summary>启动新程序后等它报到的时间</summary>
    public static int StartTimeoutMs = 120_000;

    public static int Run(string planPath)
    {
        // 辅助程序不能有任何未处理的异常（会弹系统的「已停止工作」窗口）
        UpdateLog? log = null;
        Mutex? mutex = null;
        var held = false;
        try
        {
            var env = UpdateEnv.Default;
            log = new UpdateLog(env.LogFile);
            var plan = UpdateJson.Load<UpdatePlan>(planPath);
            if (plan == null || plan.Target.Length == 0 || plan.Source.Length == 0)
            {
                log.Write("读不了更新计划：" + planPath);
                return 2;
            }
            mutex = new Mutex(false, env.MutexName);
            try { held = mutex.WaitOne(TimeSpan.FromMinutes(10)); }
            catch (AbandonedMutexException) { held = true; }
            if (!held) log.Write("等不到更新互斥量，照常继续");
            return Execute(plan, env, log, ProcessLauncher.Instance);
        }
        catch (Exception e)
        {
            log?.Write("辅助程序出错：" + e);
            return 1;
        }
        finally
        {
            if (held) try { mutex!.ReleaseMutex(); } catch { }
            mutex?.Dispose();
        }
    }

    /// <summary>等旧进程退出 → 换文件 → 启动新程序等它报到 → 收尾；任何一步失败都换回旧文件并重新打开旧程序</summary>
    public static int Execute(UpdatePlan plan, UpdateEnv env, UpdateLog log, IUpdateLauncher launcher)
    {
        log.Write($"开始更新 {plan.OldVersion} → {plan.NewVersion}，装到 {plan.Target}{(plan.Migrate ? "（原目录不可写，改装到用户目录）" : "")}");
        launcher.WaitForExit(plan.Pid, plan.PidStart, log);
        var result = new UpdateResult { Version = plan.NewVersion, From = plan.OldVersion, Sha256 = plan.Sha256, Target = UpdateFiles.Norm(plan.Target) };
        FileSwap? swap = null;
        try
        {
            if (UpdatePackage.Problem(plan.Source) is { } problem) throw new UpdateFailure("package", problem);
            swap = FileSwap.Begin(plan.Target, log);
            swap.Prepare(plan.Source, plan.NewVersion);
            swap.Swap();
        }
        catch (Exception e)
        {
            log.Write("替换失败：" + e);
            var (code, denied) = UpdateFailure.Classify(e);
            RollbackQuietly(swap, log);
            return Fail(plan, env, log, launcher, result, code, e.Message, denied);
        }

        var newExe = IOPath.Combine(plan.Target, UpdatePackage.ExeName);
        var args = UpdateStartup.JoinArgs(new[] { "--updated-from", plan.OldVersion, "--update-token", plan.Token }.Concat(plan.ExtraArgs ?? Array.Empty<string>()));
        StartOutcome outcome;
        int exitCode;
        try { outcome = launcher.StartAndWait(newExe, args, plan.Target, env.EventPrefix + plan.Token, StartTimeoutMs, log, out exitCode); }
        catch (Exception e)
        {
            log.Write("启动新程序出错：" + e);
            outcome = StartOutcome.CannotStart;
            exitCode = -1;
        }
        if (outcome != StartOutcome.Started)
        {
            var detail = outcome switch
            {
                StartOutcome.Exited => $"新程序启动后退出（退出码 {exitCode}）",
                StartOutcome.TimedOut => $"新程序 {StartTimeoutMs / 1000} 秒内没有打开",
                _ => "新程序启动不了",
            };
            var reason = env.StartupFailureFile(plan.Token);
            if (File.Exists(reason))
            {
                try { detail += "：" + File.ReadAllText(reason).Trim(); } catch { }
                UpdateFiles.TryDelete(reason);
            }
            log.Write(detail);
            RollbackQuietly(swap, log);
            return Fail(plan, env, log, launcher, result, "startup", detail, false);
        }

        log.Write("新程序已打开");
        try { swap!.Commit(); } catch (Exception e) { log.Write("确认替换出错：" + e.Message); }
        if (plan.Migrate)
        {
            try { Shortcuts.RetargetAll(env, plan.OldExe, newExe, log); }
            catch (Exception e) { log.Write("改快捷方式出错：" + e.Message); }
        }
        if (plan.Migrate || UpdateFiles.Same(plan.Target, env.UserInstallDir))
        {
            try { InstallPointer.Write(env, newExe, plan.NewVersion, plan.Migrate ? IOPath.GetDirectoryName(plan.OldExe) ?? "" : ""); }
            catch (Exception e) { log.Write("记下新位置失败：" + e.Message); }
        }
        result.Ok = true;
        SaveResult(env, result, log);
        UpdateFiles.TryDeleteDir(env.StagingDir);
        if (plan.ZipPath.Length > 0) UpdateFiles.TryDelete(plan.ZipPath);
        UpdateFiles.TryDelete(env.MarkerFile);
        log.Write($"更新完成：{plan.NewVersion}");
        return 0;
    }

    private static void RollbackQuietly(FileSwap? swap, UpdateLog log)
    {
        if (swap == null) return;
        // 刚退出的新程序、杀毒软件可能还占着文件：多试几轮；实在换不回来，下次正常启动时再换（替换日志还在）
        for (var round = 1; round <= 3; round++)
        {
            try
            {
                swap.Rollback();
                return;
            }
            catch (Exception e)
            {
                log.Write($"换回旧文件出错（第 {round} 轮）：{e.Message}");
                if (round < 3) Thread.Sleep(3000);
            }
        }
    }

    private static int Fail(UpdatePlan plan, UpdateEnv env, UpdateLog log, IUpdateLauncher launcher, UpdateResult result, string code, string detail, bool denied)
    {
        result.Ok = false;
        result.Code = code;
        result.Detail = detail;
        result.Denied = denied;
        SaveResult(env, result, log);
        // 重新打开旧程序（原地替换时这里已经换回旧文件；改装到用户目录时旧位置本来就没动）
        var exe = plan.OldExe;
        if (!File.Exists(exe)) exe = IOPath.Combine(plan.Target, UpdatePackage.ExeName);
        launcher.Start(exe, UpdateStartup.JoinArgs(new[] { "--update-failed", "--update-token", plan.Token }.Concat(plan.ExtraArgs ?? Array.Empty<string>())), IOPath.GetDirectoryName(exe) ?? plan.Target, log);
        return 1;
    }

    private static void SaveResult(UpdateEnv env, UpdateResult r, UpdateLog log)
    {
        r.Time = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        try { UpdateJson.Save(env.ResultFile, r); }
        catch (Exception e) { log.Write("写结果失败：" + e.Message); }
    }
}

/// <summary>主程序这边：准备好要装的东西，交给辅助程序</summary>
internal static class UpdatePreparer
{
    public sealed class Prepared
    {
        public UpdatePlan Plan = new();
        public string PlanPath = "";
        public string HelperExe = "";
        public string Why = "";
    }

    /// <param name="zip">下载好的压缩包</param>
    /// <param name="zipSha">下载完时算的 SHA-256（为空时不核对）</param>
    /// <param name="infoSha">服务器给的 SHA-256（记进计划，失败时用来认出同一个包）</param>
    public static Prepared Prepare(UpdateEnv env, string zip, string zipSha, string infoSha, string currentExe, string oldVersion, UpdateLog log)
    {
        Directory.CreateDirectory(env.UpdatesDir);
        if (zipSha.Length > 0 && !string.Equals(UpdateFiles.Sha256(zip), zipSha, StringComparison.OrdinalIgnoreCase))
            throw new UpdateFailure("corrupt", "下载好的更新包已经变了（校验不对）");
        var root = UpdatePackage.Extract(zip, env.StagingDir);
        if (UpdatePackage.Problem(root) is { } problem) throw new UpdateFailure("package", problem);
        var newVersion = UpdatePackage.ExeVersion(IOPath.Combine(root, UpdatePackage.ExeName)) ?? "";
        var last = UpdateJson.Load<UpdateResult>(env.ResultFile);
        var (target, migrate, why) = InstallTarget.Choose(IOPath.GetDirectoryName(currentExe)!, env, last);
        log.Write($"准备更新 {oldVersion} → {newVersion}：装到 {target}（{why}）");
        var helper = CopyHelper(env, currentExe);
        using var me = Process.GetCurrentProcess();
        var plan = new UpdatePlan
        {
            Pid = me.Id,
            PidStart = me.StartTime.ToUniversalTime().Ticks,
            OldExe = UpdateFiles.Norm(currentExe),
            OldVersion = oldVersion,
            NewVersion = newVersion,
            Source = root,
            Target = target,
            Migrate = migrate,
            Token = Guid.NewGuid().ToString("N"),
            Sha256 = infoSha,
            ZipPath = zip,
        };
        UpdateJson.Save(env.PlanFile, plan);
        return new Prepared { Plan = plan, PlanPath = env.PlanFile, HelperExe = helper, Why = why };
    }

    /// <summary>辅助程序：当前这份 exe 复制到更新目录里运行（程序目录里的文件要被换掉）。每次用新的子目录，上一次的可能还没退出</summary>
    public static string CopyHelper(UpdateEnv env, string currentExe)
    {
        CleanHelpers(env);
        var dir = IOPath.Combine(env.UpdatesDir, "helper-" + DateTime.UtcNow.Ticks.ToString("x"));
        Directory.CreateDirectory(dir);
        var exe = IOPath.Combine(dir, UpdatePackage.ExeName);
        File.Copy(currentExe, exe, true);
        var config = currentExe + ".config";
        if (File.Exists(config)) File.Copy(config, exe + ".config", true);
        return exe;
    }

    /// <summary>删掉用完的辅助程序目录（还在运行的删不掉，留着）与旧版本留下的批处理、暂存目录</summary>
    public static void CleanHelpers(UpdateEnv env)
    {
        try
        {
            if (!Directory.Exists(env.UpdatesDir)) return;
            foreach (var d in Directory.GetDirectories(env.UpdatesDir, "helper-*")) UpdateFiles.TryDeleteDir(d);
            foreach (var d in Directory.GetDirectories(env.UpdatesDir, "staging-*")) UpdateFiles.TryDeleteDir(d);
            UpdateFiles.TryDelete(IOPath.Combine(env.UpdatesDir, "install.cmd"));
        }
        catch { }
    }
}

/// <summary>程序启动时与更新有关的几件事（Program.Main 调）</summary>
internal static class UpdateStartup
{
    public static string? Arg(string[] args, string name)
    {
        var i = Array.IndexOf(args, name);
        return i >= 0 && i + 1 < args.Length ? args[i + 1] : null;
    }

    /// <summary>把参数原样拼回命令行（按 CommandLineToArgvW 的规则加引号、转义引号前的反斜杠）</summary>
    public static string JoinArgs(IEnumerable<string> args)
    {
        var sb = new StringBuilder();
        foreach (var a in args)
        {
            if (sb.Length > 0) sb.Append(' ');
            if (a.Length > 0 && a.IndexOfAny(new[] { ' ', '\t', '"' }) < 0)
            {
                sb.Append(a);
                continue;
            }
            sb.Append('"');
            var slashes = 0;
            foreach (var c in a)
            {
                if (c == '\\') { slashes++; continue; }
                sb.Append('\\', c == '"' ? slashes * 2 + 1 : slashes);
                slashes = 0;
                sb.Append(c);
            }
            sb.Append('\\', slashes * 2);
            sb.Append('"');
        }
        return sb.ToString();
    }

    public enum RunningUpdate { None, Finished, StillRunning }

    /// <summary>
    /// 有更新正在装（辅助程序拿着互斥量）时等它装完：Finished 表示等到了，本进程直接退出（辅助程序会打开新程序或换回的旧程序）；
    /// StillRunning 表示等了 timeoutMs 还没装完（不正常），照常打开但不做恢复、不转去新位置。
    /// </summary>
    public static RunningUpdate WaitForRunningUpdate(UpdateEnv env, int timeoutMs)
    {
        try
        {
            using var m = new Mutex(false, env.MutexName);
            bool got;
            try { got = m.WaitOne(0); }
            catch (AbandonedMutexException) { got = true; }
            if (got)
            {
                m.ReleaseMutex();
                return RunningUpdate.None;
            }
            try { got = m.WaitOne(timeoutMs); }
            catch (AbandonedMutexException) { got = true; }
            if (!got) return RunningUpdate.StillRunning;
            m.ReleaseMutex();
            return RunningUpdate.Finished;
        }
        catch { return RunningUpdate.None; }
    }

    /// <summary>新程序的界面画出来了：告诉辅助程序</summary>
    public static void Signal(UpdateEnv env, string token)
    {
        try
        {
            if (EventWaitHandle.TryOpenExisting(env.EventPrefix + token, out var e))
                using (e) e.Set();
        }
        catch { }
    }

    /// <summary>新程序打不开（内核加载失败）：写下原因，辅助程序换回旧版本时带上</summary>
    public static void ReportStartupFailure(UpdateEnv env, string token, string reason)
    {
        try
        {
            Directory.CreateDirectory(env.UpdatesDir);
            File.WriteAllText(env.StartupFailureFile(token), reason, new UTF8Encoding(false));
        }
        catch { }
    }

    /// <summary>后台清理：程序目录里做完的替换工作目录、用完的辅助程序</summary>
    public static void Cleanup(UpdateEnv env, string baseDir)
    {
        try
        {
            foreach (var d in Directory.GetDirectories(baseDir, FileSwap.Prefix + "*"))
            {
                var j = UpdateJson.Load<SwapJournal>(IOPath.Combine(d, "journal.json"));
                if (j?.Status is "committed" or "rolledback") UpdateFiles.TryDeleteDir(d);
            }
        }
        catch { }
        UpdatePreparer.CleanHelpers(env);
    }
}

/// <summary>辅助程序启动的进程（等待它 = 等它退出）</summary>
internal sealed class Spawned : WaitHandle
{
    public readonly int Pid;

    public Spawned(IntPtr process, int pid)
    {
        SafeWaitHandle = new SafeWaitHandle(process, true);
        Pid = pid;
    }

    /// <summary>还在运行时为 null</summary>
    public int? ExitCode => NativeProcess.GetExitCodeProcess(SafeWaitHandle, out var c) && c != 259 ? (int)c : null;

    public void Kill()
    {
        try { NativeProcess.TerminateProcess(SafeWaitHandle, 1); } catch { }
    }
}

/// <summary>
/// 用 CreateProcess 启动程序：不经 Shell（不弹「打开文件 - 安全警告」、不走 SmartScreen），不继承句柄，
/// 尽量脱离父进程所在的作业对象（有的启动器把子进程放进「关掉就全部结束」的作业里，辅助程序要比主程序活得久）。
/// </summary>
internal static class NativeProcess
{
    private const uint CreateBreakawayFromJob = 0x01000000;
    private const uint CreateUnicodeEnvironment = 0x00000400;
    private const uint CreateDefaultErrorMode = 0x04000000;
    /// <summary>控制台程序不弹黑窗口（对图形界面程序不起作用）</summary>
    private const uint CreateNoWindow = 0x08000000;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct StartupInfo
    {
        public int cb;
        public string? lpReserved, lpDesktop, lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2;
        public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ProcessInformation
    {
        public IntPtr hProcess, hThread;
        public int dwProcessId, dwThreadId;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CreateProcessW(string? app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string? cwd, ref StartupInfo si, out ProcessInformation pi);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr h);

    [DllImport("kernel32.dll", SetLastError = true)]
    internal static extern bool GetExitCodeProcess(SafeWaitHandle h, out uint code);

    [DllImport("kernel32.dll", SetLastError = true)]
    internal static extern bool TerminateProcess(SafeWaitHandle h, uint code);

    [DllImport("user32.dll")]
    private static extern bool AllowSetForegroundWindow(int pid);

    public static Spawned Start(string exe, string args, string? cwd)
    {
        var cmd = "\"" + exe + "\"" + (args.Length > 0 ? " " + args : "");
        foreach (var breakaway in new[] { true, false })
        {
            var si = new StartupInfo { cb = Marshal.SizeOf(typeof(StartupInfo)) };
            var flags = CreateUnicodeEnvironment | CreateDefaultErrorMode | CreateNoWindow | (breakaway ? CreateBreakawayFromJob : 0);
            if (CreateProcessW(exe, new StringBuilder(cmd), IntPtr.Zero, IntPtr.Zero, false, flags, IntPtr.Zero, cwd, ref si, out var pi))
            {
                CloseHandle(pi.hThread);
                return new Spawned(pi.hProcess, pi.dwProcessId);
            }
            var err = Marshal.GetLastWin32Error();
            // 作业不允许脱离时 ERROR_ACCESS_DENIED：不脱离再试一次
            if (!(breakaway && err == 5)) throw new Win32Exception(err);
        }
        throw new Win32Exception(5);
    }

    /// <summary>主程序退出前调用：让接下来打开的新程序能把窗口带到前台</summary>
    public static void AllowForeground()
    {
        try { AllowSetForegroundWindow(-1); } catch { }
    }
}
