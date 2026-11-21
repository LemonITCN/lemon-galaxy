namespace LemonGalaxy;

/// <summary>简单日志：写到 %LOCALAPPDATA%\LemonGalaxy\control\ui.log（selftest 时写到输出目录）</summary>
internal static class Log
{
    public static string? FilePath;
    private static readonly object Lock = new();

    public static void Info(string s) => Write("INFO", s);
    public static int ErrorCount;

    public static void Error(string s)
    {
        Interlocked.Increment(ref ErrorCount);
        Write("ERROR", s);
    }

    private static void Write(string level, string s)
    {
        var line = $"{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff} {level} {s}";
        System.Diagnostics.Debug.WriteLine(line);
        var path = FilePath;
        if (path == null) return;
        lock (Lock)
        {
            try { System.IO.File.AppendAllText(path, line + Environment.NewLine); }
            catch { }
        }
    }
}
