using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using ComTypes = System.Runtime.InteropServices.ComTypes;

namespace LemonGalaxy;

/// <summary>
/// 改装到用户目录后：把开始菜单、桌面、固定到任务栏的快捷方式里指向旧位置 exe 的改到新位置；
/// 公共的（所有用户的开始菜单、公用桌面）没有权限改，留着——旧位置的程序打开时会转去新位置。
/// 用户的开始菜单里没有指向新位置的就建一个（程序搬进了隐藏的 AppData，不然不好找）。
/// </summary>
internal static class Shortcuts
{
    public static int RetargetAll(UpdateEnv env, string oldExe, string newExe, UpdateLog log)
    {
        var changed = 0;
        var inStartMenu = false;
        foreach (var root in env.ShortcutRoots)
        {
            foreach (var lnk in Links(root.Key, root.Value))
            {
                string? target;
                try { target = ShellLink.ReadTarget(lnk); }
                catch { continue; }
                var underStart = env.StartMenuDir.Length > 0 && UpdateFiles.IsUnder(lnk, env.StartMenuDir);
                if (UpdateFiles.Same(target, newExe))
                {
                    if (underStart) inStartMenu = true;
                    continue;
                }
                if (!UpdateFiles.Same(target, oldExe)) continue;
                try
                {
                    ShellLink.Retarget(lnk, oldExe, newExe);
                    changed++;
                    if (underStart) inStartMenu = true;
                    log.Write("快捷方式已改到新位置：" + lnk);
                }
                catch (Exception e)
                {
                    log.Write($"快捷方式改不了（{e.Message}），旧位置的程序打开时会转到新位置：{lnk}");
                }
            }
        }
        if (!inStartMenu && env.StartMenuDir.Length > 0)
        {
            var lnk = IOPath.Combine(env.StartMenuDir, "LemonGalaxy.lnk");
            try
            {
                if (!File.Exists(lnk))
                {
                    Directory.CreateDirectory(env.StartMenuDir);
                    ShellLink.Create(lnk, newExe, "LemonGalaxy");
                    log.Write("已在开始菜单建快捷方式：" + lnk);
                }
            }
            catch (Exception e) { log.Write("建开始菜单快捷方式失败：" + e.Message); }
        }
        return changed;
    }

    private static IEnumerable<string> Links(string dir, bool recursive, int depth = 0)
    {
        string[] files, dirs;
        try
        {
            if (!Directory.Exists(dir)) yield break;
            files = Directory.GetFiles(dir, "*.lnk");
            dirs = recursive && depth < 4 ? Directory.GetDirectories(dir) : Array.Empty<string>();
        }
        catch { yield break; }
        foreach (var f in files) yield return f;
        foreach (var d in dirs)
            foreach (var f in Links(d, true, depth + 1))
                yield return f;
    }
}

/// <summary>.lnk 读写（Shell 的 IShellLinkW，不依赖脚本宿主）。调用线程要是 STA（Main 带 [STAThread]）</summary>
internal static class ShellLink
{
    private const int StgmRead = 0;
    private const int StgmReadWrite = 2;

    public static string? ReadTarget(string lnk)
    {
        var link = (IShellLinkW)new CShellLink();
        try
        {
            ((ComTypes.IPersistFile)link).Load(lnk, StgmRead);
            var sb = new StringBuilder(1024);
            link.GetPath(sb, sb.Capacity, IntPtr.Zero, 0);
            return sb.Length > 0 ? sb.ToString() : null;
        }
        finally { Marshal.ReleaseComObject(link); }
    }

    /// <summary>目标改到新 exe；起始位置、图标原来指向旧位置的一起改，参数不动</summary>
    public static void Retarget(string lnk, string oldExe, string newExe)
    {
        var link = (IShellLinkW)new CShellLink();
        try
        {
            var file = (ComTypes.IPersistFile)link;
            file.Load(lnk, StgmReadWrite);
            var oldDir = IOPath.GetDirectoryName(oldExe) ?? "";
            var newDir = IOPath.GetDirectoryName(newExe) ?? "";
            link.SetPath(newExe);
            var wd = new StringBuilder(1024);
            link.GetWorkingDirectory(wd, wd.Capacity);
            if (wd.Length == 0 || UpdateFiles.Same(wd.ToString(), oldDir)) link.SetWorkingDirectory(newDir);
            var sb = new StringBuilder(1024);
            link.GetIconLocation(sb, sb.Capacity, out var index);
            var icon = sb.ToString();
            if (icon.Length > 0 && UpdateFiles.Same(icon, oldExe)) link.SetIconLocation(newExe, index);
            else if (icon.Length > 0 && UpdateFiles.IsUnder(icon, oldDir))
                link.SetIconLocation(IOPath.Combine(newDir, UpdateFiles.Norm(icon).Substring(UpdateFiles.Norm(oldDir).Length + 1)), index);
            file.Save(lnk, true);
        }
        finally { Marshal.ReleaseComObject(link); }
    }

    public static void Create(string lnk, string exe, string description)
    {
        var link = (IShellLinkW)new CShellLink();
        try
        {
            link.SetPath(exe);
            link.SetWorkingDirectory(IOPath.GetDirectoryName(exe) ?? "");
            link.SetDescription(description);
            link.SetIconLocation(exe, 0);
            ((ComTypes.IPersistFile)link).Save(lnk, true);
        }
        finally { Marshal.ReleaseComObject(link); }
    }

    [ComImport, Guid("00021401-0000-0000-C000-000000000046")]
    private class CShellLink { }

    [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("000214F9-0000-0000-C000-000000000046")]
    private interface IShellLinkW
    {
        void GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszFile, int cch, IntPtr pfd, uint fFlags);
        void GetIDList(out IntPtr ppidl);
        void SetIDList(IntPtr pidl);
        void GetDescription([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszName, int cch);
        void SetDescription([MarshalAs(UnmanagedType.LPWStr)] string pszName);
        void GetWorkingDirectory([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszDir, int cch);
        void SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string pszDir);
        void GetArguments([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszArgs, int cch);
        void SetArguments([MarshalAs(UnmanagedType.LPWStr)] string pszArgs);
        void GetHotkey(out short pwHotkey);
        void SetHotkey(short wHotkey);
        void GetShowCmd(out int piShowCmd);
        void SetShowCmd(int iShowCmd);
        void GetIconLocation([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszIconPath, int cch, out int piIcon);
        void SetIconLocation([MarshalAs(UnmanagedType.LPWStr)] string pszIconPath, int iIcon);
        void SetRelativePath([MarshalAs(UnmanagedType.LPWStr)] string pszPathRel, uint dwReserved);
        void Resolve(IntPtr hwnd, uint fFlags);
        void SetPath([MarshalAs(UnmanagedType.LPWStr)] string pszFile);
    }
}
