using System.IO;
using System.Security.Cryptography;
using System.Text;
using LemonGalaxy.Core.Mock;

namespace LemonGalaxy.Core;

/// <summary>模拟内核的可调项（selftest 用：去掉延时、不起后台定时器）</summary>
internal sealed class MockOptions
{
    /// <summary>模拟网络延时倍率，0 表示立即返回</summary>
    public double LatencyScale = 1;
    /// <summary>是否在后台模拟设备上下线、路径切换等变化</summary>
    public bool LiveSimulation = true;
}

/// <summary>
/// 界面拿内核实现的唯一入口。
///   - 启动参数 --mock、环境变量 LEMONGALAXY_MOCK=1、--selftest：模拟内核（不加载 galaxy_ffi.dll）；
///   - 其余情况用 exe 同目录的 galaxy_ffi.dll；加载失败时退回模拟内核并提示。
/// </summary>
internal static class CoreProvider
{
    private static bool initialized;

    public static bool WantsMock(string[] args)
    {
        if (args.Contains("--mock")) return true;
        var env = Environment.GetEnvironmentVariable("LEMONGALAXY_MOCK");
        return env == "1" || string.Equals(env, "true", StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>返回内核与（退回模拟内核时的）原因</summary>
    public static (ICoreClient client, string? fallbackReason) Make(CoreConfig config, SecureStore store, ClientListener listener, bool forceMock, MockOptions? options = null)
    {
        if (forceMock) return (new MockClient(store, listener, config.Locale, options ?? new MockOptions()), null);
        var dll = IOPath.Combine(AppInfo.BaseDir, "galaxy_ffi.dll");
        if (!File.Exists(dll))
            return (new MockClient(store, listener, config.Locale, options ?? new MockOptions()), "galaxy_ffi.dll not found: " + dll);
        try
        {
            if (!initialized)
            {
                GalaxyFfiMethods.CoreInit(config);
                initialized = true;
            }
            return (new RealClient(new GalaxyClient(store, listener)), null);
        }
        catch (Exception e)
        {
            Log.Error("内核加载失败：" + e);
            var inner = e is TypeInitializationException { InnerException: { } ie } ? ie : e;
            return (new MockClient(store, listener, config.Locale, options ?? new MockOptions()), inner.GetType().Name + ": " + inner.Message);
        }
    }
}

/// <summary>本地安全存储：Windows DPAPI（当前用户），每个键一个文件。</summary>
internal sealed class DpapiStore : SecureStore
{
    private readonly string dir;
    private static readonly byte[] Entropy = Encoding.UTF8.GetBytes("cn.lemonit.galaxy.control");

    public DpapiStore(string dir)
    {
        this.dir = dir;
        Directory.CreateDirectory(dir);
    }

    private string PathFor(string key)
    {
        using var sha = SHA256.Create();
        var h = sha.ComputeHash(Encoding.UTF8.GetBytes(key));
        return IOPath.Combine(dir, BitConverter.ToString(h, 0, 16).Replace("-", "").ToLowerInvariant() + ".bin");
    }

    public byte[]? Get(string key)
    {
        try
        {
            var p = PathFor(key);
            if (!File.Exists(p)) return null;
            return ProtectedData.Unprotect(File.ReadAllBytes(p), Entropy, DataProtectionScope.CurrentUser);
        }
        catch (Exception e)
        {
            Log.Error("读取安全存储失败（" + key + "）：" + e.Message);
            return null;
        }
    }

    public void Set(string key, byte[] value)
    {
        try
        {
            var p = PathFor(key);
            var tmp = p + ".tmp";
            File.WriteAllBytes(tmp, ProtectedData.Protect(value, Entropy, DataProtectionScope.CurrentUser));
            if (File.Exists(p)) File.Delete(p);
            File.Move(tmp, p);
        }
        catch (Exception e) { Log.Error("写入安全存储失败（" + key + "）：" + e.Message); }
    }

    public void Remove(string key)
    {
        try
        {
            var p = PathFor(key);
            if (File.Exists(p)) File.Delete(p);
        }
        catch { }
    }
}

/// <summary>内存里的安全存储（模拟内核与 selftest 用，不落盘）</summary>
internal sealed class MemoryStore : SecureStore
{
    private readonly Dictionary<string, byte[]> map = new();
    public byte[]? Get(string key) { lock (map) return map.TryGetValue(key, out var v) ? v : null; }
    public void Set(string key, byte[] value) { lock (map) map[key] = value; }
    public void Remove(string key) { lock (map) map.Remove(key); }
}
