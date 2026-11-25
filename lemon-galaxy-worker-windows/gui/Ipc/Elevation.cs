using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Security.Principal;
using System.Text;

namespace LemonGalaxy.Ipc;

/// <summary>
/// 以管理员身份（UAC）运行本程序的一个子命令，等它结束。界面本身一直以普通权限运行。
/// 子命令把结果（成功为空，失败为原因）写进调用方指定的文件：提权后的进程可能是另一个账号（标准用户输入管理员密码时），
/// 读不到调用方的输出。
/// </summary>
internal static class Elevation
{
    public sealed record Result(bool Ok, bool Cancelled, string? Error);

    public static bool IsAdmin
    {
        get
        {
            try { return new WindowsPrincipal(WindowsIdentity.GetCurrent()).IsInRole(WindowsBuiltInRole.Administrator); }
            catch { return false; }
        }
    }

    public static string Quote(string s) => "\"" + s.Replace("\"", "") + "\"";

    public static Task<Result> Run(params string[] args) => Task.Run(() => RunSync(args));

    public static Result RunSync(string[] args)
    {
        var result = IOPath.Combine(IOPath.GetTempPath(), $"lemon-galaxy-worker-ui-{Guid.NewGuid():N}.txt");
        try
        {
            var all = args.Concat(new[] { "--result", result }).Select(Quote);
            var psi = new ProcessStartInfo(AppInfo.ExePath, string.Join(" ", all)) { UseShellExecute = true, Verb = "runas", WindowStyle = ProcessWindowStyle.Hidden };
            using var p = Process.Start(psi);
            if (p == null) return new(false, false, "Process.Start");
            p.WaitForExit();
            string? text = null;
            try { if (File.Exists(result)) text = File.ReadAllText(result, Encoding.UTF8).Trim(); } catch { }
            if (p.ExitCode == 0) return new(true, false, null);
            return new(false, false, string.IsNullOrEmpty(text) ? $"exit {p.ExitCode}" : text);
        }
        catch (Win32Exception e) when (e.NativeErrorCode == 1223)
        {
            // 在「用户账户控制」里点了「否」
            return new(false, true, null);
        }
        catch (Exception e)
        {
            return new(false, false, e.Message);
        }
        finally
        {
            try { File.Delete(result); } catch { }
        }
    }

    /// <summary>提权子命令把结果写回调用方</summary>
    public static void WriteResult(string? path, string? error)
    {
        if (string.IsNullOrEmpty(path)) return;
        try { File.WriteAllText(path, error ?? "", new UTF8Encoding(false)); } catch { }
    }

    /// <summary>
    /// 让当前连接经 UAC 授权：向服务要一把一次性钥匙，以管理员身份运行 `--elevate 钥匙 --pipe 管道名`，
    /// 那个进程以管理员身份连上来把钥匙交给服务，这个连接就能在几分钟内直接改配置。
    /// </summary>
    public static async Task<Result> Authorize(ControlClient client)
    {
        var r = await client.Call(Req.ElevationKey());
        if (r.Type != "elevation_key") return new(false, false, r.Ok ? r.Type : r.Message);
        var key = r.J.S("key");
        return await Run("--elevate", key, "--pipe", client.PipeName ?? ControlClient.SystemPipe);
    }

    /// <summary>`--elevate`：以管理员身份运行时把钥匙交给服务。返回退出码。</summary>
    public static int ElevateMain(string key, string pipe, string? resultFile)
    {
        try
        {
            var system = string.Equals(ControlClient.StripPrefix(pipe), ControlClient.SystemPipe, StringComparison.OrdinalIgnoreCase);
            var rs = ControlClient.Once(ControlClient.StripPrefix(pipe), system, new[] { Req.Hello(L.Pack.CoreLocale), Req.Elevate(key) });
            var last = rs.Last();
            if (last.Ok) return 0;
            WriteResult(resultFile, last.Message);
            return 2;
        }
        catch (Exception e)
        {
            WriteResult(resultFile, e.Message);
            return 3;
        }
    }
}
