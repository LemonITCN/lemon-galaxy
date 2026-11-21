using System.Windows.Input;

namespace LemonGalaxy.UI;

/// <summary>WPF 按键 → USB HID 键盘用法码（Usage Page 0x07），主控端把本机按键发给被控端（protocol.md 8.4）</summary>
internal static class HidKeyMap
{
    private static readonly Dictionary<Key, uint> Map = Build();

    private static Dictionary<Key, uint> Build()
    {
        var m = new Dictionary<Key, uint>();
        for (var k = Key.A; k <= Key.Z; k++) m[k] = 0x04u + (uint)(k - Key.A);
        m[Key.D1] = 0x1E; m[Key.D2] = 0x1F; m[Key.D3] = 0x20; m[Key.D4] = 0x21; m[Key.D5] = 0x22;
        m[Key.D6] = 0x23; m[Key.D7] = 0x24; m[Key.D8] = 0x25; m[Key.D9] = 0x26; m[Key.D0] = 0x27;
        m[Key.Enter] = 0x28; m[Key.Escape] = 0x29; m[Key.Back] = 0x2A; m[Key.Tab] = 0x2B; m[Key.Space] = 0x2C;
        m[Key.OemMinus] = 0x2D; m[Key.OemPlus] = 0x2E; m[Key.OemOpenBrackets] = 0x2F; m[Key.OemCloseBrackets] = 0x30;
        m[Key.Oem5] = 0x31; m[Key.Oem1] = 0x33; m[Key.OemQuotes] = 0x34; m[Key.Oem3] = 0x35;
        m[Key.OemComma] = 0x36; m[Key.OemPeriod] = 0x37; m[Key.OemQuestion] = 0x38; m[Key.CapsLock] = 0x39;
        for (var i = 0; i < 12; i++) m[Key.F1 + i] = 0x3Au + (uint)i;
        for (var i = 0; i < 12; i++) m[Key.F13 + i] = 0x68u + (uint)i;
        m[Key.PrintScreen] = 0x46; m[Key.Scroll] = 0x47; m[Key.Pause] = 0x48; m[Key.Insert] = 0x49; m[Key.Home] = 0x4A; m[Key.PageUp] = 0x4B;
        m[Key.Delete] = 0x4C; m[Key.End] = 0x4D; m[Key.PageDown] = 0x4E; m[Key.Right] = 0x4F; m[Key.Left] = 0x50; m[Key.Down] = 0x51; m[Key.Up] = 0x52;
        m[Key.NumLock] = 0x53; m[Key.Divide] = 0x54; m[Key.Multiply] = 0x55; m[Key.Subtract] = 0x56; m[Key.Add] = 0x57;
        m[Key.NumPad1] = 0x59; m[Key.NumPad2] = 0x5A; m[Key.NumPad3] = 0x5B; m[Key.NumPad4] = 0x5C; m[Key.NumPad5] = 0x5D;
        m[Key.NumPad6] = 0x5E; m[Key.NumPad7] = 0x5F; m[Key.NumPad8] = 0x60; m[Key.NumPad9] = 0x61; m[Key.NumPad0] = 0x62; m[Key.Decimal] = 0x63;
        m[Key.Oem102] = 0x64; m[Key.Apps] = 0x65;
        m[Key.VolumeMute] = 0x7F; m[Key.VolumeUp] = 0x80; m[Key.VolumeDown] = 0x81;
        m[Key.LeftCtrl] = 0xE0; m[Key.LeftShift] = 0xE1; m[Key.LeftAlt] = 0xE2; m[Key.LWin] = 0xE3;
        m[Key.RightCtrl] = 0xE4; m[Key.RightShift] = 0xE5; m[Key.RightAlt] = 0xE6; m[Key.RWin] = 0xE7;
        return m;
    }

    /// <summary>小键盘回车与主键盘回车在 WPF 里同是 Key.Enter，用扩展键标记区分</summary>
    public static uint? Hid(KeyEventArgs e)
    {
        var key = e.Key switch
        {
            Key.System => e.SystemKey,
            Key.ImeProcessed => e.ImeProcessedKey,
            _ => e.Key,
        };
        if (key == Key.Enter && IsExtended(e)) return 0x58;
        return Map.TryGetValue(key, out var v) ? v : null;
    }

    private static bool IsExtended(KeyEventArgs e)
    {
        try
        {
            var prop = typeof(KeyEventArgs).GetProperty("IsExtendedKey", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);
            return prop?.GetValue(e) is true;
        }
        catch { return false; }
    }
}
