using System.Globalization;

namespace LemonGalaxy.UI;

internal static class Fmt
{
    private static readonly string[] Units = { "B", "KB", "MB", "GB", "TB" };

    /// <summary>字节数，1024 进制（与 Mac 端一致）</summary>
    public static string Bytes(ulong n)
    {
        if (n < 1024) return n + " " + (L.IsChinese ? "字节" : "bytes");
        double v = n;
        var i = 0;
        while (v >= 1024 && i < Units.Length - 1) { v /= 1024; i++; }
        return (v >= 100 ? v.ToString("0", CultureInfo.InvariantCulture) : v.ToString("0.#", CultureInfo.InvariantCulture)) + " " + Units[i];
    }

    public static DateTime Local(long ms) => DateTimeOffset.FromUnixTimeMilliseconds(ms).LocalDateTime;

    public static string Date(long ms) => Local(ms).ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture);

    /// <summary>列表里的短日期：今年只显示 月-日 时:分</summary>
    public static string ShortDate(long ms)
    {
        var d = Local(ms);
        var now = Local(Clock.NowMs);
        return d.Year == now.Year ? d.ToString("MM-dd HH:mm", CultureInfo.InvariantCulture) : d.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture);
    }

    public static string Time(long ms) => Local(ms).ToString("HH:mm:ss", CultureInfo.InvariantCulture);

    public static string Relative(long ms)
    {
        var diff = Math.Max(0, (Clock.NowMs - ms) / 1000);
        if (diff < 60) return L.T("time.justNow");
        if (diff < 3600) return L.T("time.minutesAgo", diff / 60);
        if (diff < 86_400) return L.T("time.hoursAgo", diff / 3600);
        return L.T("time.daysAgo", diff / 86_400);
    }

    /// <summary>rwxr-xr-x</summary>
    public static string Mode(uint m)
    {
        var chars = "rwx";
        var s = new char[9];
        var k = 0;
        for (var shift = 6; shift >= 0; shift -= 3)
            for (var i = 0; i < 3; i++)
                s[k++] = ((m >> (shift + 2 - i)) & 1) == 1 ? chars[i] : '-';
        return new string(s);
    }

    public static string Kilo(ulong n) => n >= 1000 ? (n / 1000.0).ToString("0.0", CultureInfo.InvariantCulture) + "k" : n.ToString(CultureInfo.InvariantCulture);

    /// <summary>内核错误转成界面文字（GalaxyException 的 detail 已本地化）</summary>
    public static string Error(Exception e)
    {
        if (e is AggregateException ag && ag.InnerException != null) e = ag.InnerException;
        return e switch
        {
            GalaxyException.Network n => n.detail,
            GalaxyException.Auth a => a.detail,
            GalaxyException.Permission p => p.detail,
            GalaxyException.NotFound nf => nf.detail,
            GalaxyException.Protocol pr => pr.detail,
            GalaxyException.Other o => o.detail,
            GalaxyException.Cancelled => L.T("common.cancelled"),
            _ => e.Message,
        };
    }
}
