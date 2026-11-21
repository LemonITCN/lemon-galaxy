using System.Runtime.InteropServices;
using System.Windows;

namespace LemonGalaxy.UI;

/// <summary>系统剪贴板（别的程序占用时稍后重试几次）</summary>
internal static class SysClipboard
{
    public static void SetText(string text)
    {
        for (var i = 0; i < 5; i++)
        {
            try
            {
                Clipboard.SetText(text);
                return;
            }
            catch (COMException) { Thread.Sleep(30); }
            catch (Exception e) { Log.Error("写剪贴板失败：" + e.Message); return; }
        }
    }

    public static string? GetText()
    {
        for (var i = 0; i < 5; i++)
        {
            try { return Clipboard.ContainsText() ? Clipboard.GetText() : null; }
            catch (COMException) { Thread.Sleep(30); }
            catch { return null; }
        }
        return null;
    }
}
