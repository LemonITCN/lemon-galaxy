using System.Text;

namespace LemonGalaxy.UI;

/// <summary>
/// 二维码编码（字节模式、纠错等级 M、版本 1–40 自动选择、按惩罚分选掩码），用于两步验证的 otpauth 地址。
/// 按 ISO/IEC 18004 实现；Windows 没有现成的系统接口可用。
/// </summary>
internal static class QrCode
{
    private static readonly int[] EccPerBlockM = { -1, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28 };
    private static readonly int[] BlocksM = { -1, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21, 23, 25, 26, 28, 29, 31, 33, 35, 37, 38, 40, 43, 45, 47, 49 };

    private static int RawModules(int ver)
    {
        var r = (16 * ver + 128) * ver + 64;
        if (ver >= 2)
        {
            var n = ver / 7 + 2;
            r -= (25 * n - 10) * n - 55;
            if (ver >= 7) r -= 36;
        }
        return r;
    }

    private static int DataCodewords(int ver) => RawModules(ver) / 8 - EccPerBlockM[ver] * BlocksM[ver];

    /// <summary>编码，返回模块矩阵（true 为深色）</summary>
    public static bool[,] Encode(string text)
    {
        var data = Encoding.UTF8.GetBytes(text);
        int ver = 1;
        for (; ver <= 40; ver++)
        {
            var bits = 4 + (ver <= 9 ? 8 : 16) + data.Length * 8;
            if (bits <= DataCodewords(ver) * 8) break;
        }
        if (ver > 40) throw new ArgumentException("text too long");
        // 数据位流
        var bb = new List<bool>();
        void Append(int val, int len) { for (var i = len - 1; i >= 0; i--) bb.Add(((val >> i) & 1) != 0); }
        Append(4, 4);
        Append(data.Length, ver <= 9 ? 8 : 16);
        foreach (var b in data) Append(b, 8);
        var cap = DataCodewords(ver) * 8;
        Append(0, Math.Min(4, cap - bb.Count));
        Append(0, (8 - bb.Count % 8) % 8);
        for (var pad = 0xEC; bb.Count < cap; pad ^= 0xEC ^ 0x11) Append(pad, 8);
        var codewords = new byte[bb.Count / 8];
        for (var i = 0; i < bb.Count; i++) if (bb[i]) codewords[i >> 3] |= (byte)(1 << (7 - (i & 7)));
        var all = AddEcc(codewords, ver);
        var size = ver * 4 + 17;
        var modules = new bool[size, size];
        var isFunc = new bool[size, size];
        DrawFunctionPatterns(modules, isFunc, ver, size);
        DrawCodewords(modules, isFunc, all, size);
        // 选惩罚分最低的掩码
        var best = 0;
        var bestPenalty = int.MaxValue;
        for (var m = 0; m < 8; m++)
        {
            ApplyMask(modules, isFunc, m, size);
            DrawFormat(modules, isFunc, m, size);
            var p = Penalty(modules, size);
            if (p < bestPenalty) { bestPenalty = p; best = m; }
            ApplyMask(modules, isFunc, m, size);
        }
        ApplyMask(modules, isFunc, best, size);
        DrawFormat(modules, isFunc, best, size);
        return modules;
    }

    private static byte[] AddEcc(byte[] data, int ver)
    {
        int numBlocks = BlocksM[ver], eccLen = EccPerBlockM[ver];
        var raw = RawModules(ver) / 8;
        var numShort = numBlocks - raw % numBlocks;
        var shortLen = raw / numBlocks;
        var divisor = RsDivisor(eccLen);
        var blocks = new List<byte[]>();
        for (int i = 0, k = 0; i < numBlocks; i++)
        {
            var datLen = shortLen - eccLen + (i < numShort ? 0 : 1);
            var dat = new byte[datLen];
            Array.Copy(data, k, dat, 0, datLen);
            k += datLen;
            var ecc = RsRemainder(dat, divisor);
            var block = new byte[shortLen + 1];
            Array.Copy(dat, 0, block, 0, datLen);
            // 短块在数据末尾留一个空位，交织时跳过
            Array.Copy(ecc, 0, block, block.Length - eccLen, eccLen);
            blocks.Add(block);
        }
        var result = new List<byte>(raw);
        for (var i = 0; i < shortLen + 1; i++)
            for (var j = 0; j < blocks.Count; j++)
                if (i != shortLen - eccLen || j >= numShort) result.Add(blocks[j][i]);
        return result.ToArray();
    }

    private static int Mul(int x, int y)
    {
        var z = 0;
        for (var i = 7; i >= 0; i--)
        {
            z = (z << 1) ^ ((z >> 7) * 0x11D);
            z ^= ((y >> i) & 1) * x;
        }
        return z;
    }

    private static byte[] RsDivisor(int degree)
    {
        var result = new byte[degree];
        result[degree - 1] = 1;
        var root = 1;
        for (var i = 0; i < degree; i++)
        {
            for (var j = 0; j < degree; j++)
            {
                result[j] = (byte)Mul(result[j], root);
                if (j + 1 < degree) result[j] ^= result[j + 1];
            }
            root = Mul(root, 0x02);
        }
        return result;
    }

    private static byte[] RsRemainder(byte[] data, byte[] divisor)
    {
        var result = new byte[divisor.Length];
        foreach (var b in data)
        {
            var factor = b ^ result[0];
            Array.Copy(result, 1, result, 0, result.Length - 1);
            result[result.Length - 1] = 0;
            for (var i = 0; i < result.Length; i++) result[i] ^= (byte)Mul(divisor[i], factor);
        }
        return result;
    }

    private static void Set(bool[,] m, bool[,] f, int x, int y, bool dark)
    {
        m[y, x] = dark;
        f[y, x] = true;
    }

    private static void DrawFunctionPatterns(bool[,] m, bool[,] f, int ver, int size)
    {
        for (var i = 0; i < size; i++)
        {
            Set(m, f, 6, i, i % 2 == 0);
            Set(m, f, i, 6, i % 2 == 0);
        }
        void Finder(int cx, int cy)
        {
            for (var dy = -4; dy <= 4; dy++)
                for (var dx = -4; dx <= 4; dx++)
                {
                    var d = Math.Max(Math.Abs(dx), Math.Abs(dy));
                    int x = cx + dx, y = cy + dy;
                    if (x >= 0 && x < size && y >= 0 && y < size) Set(m, f, x, y, d != 2 && d != 4);
                }
        }
        Finder(3, 3);
        Finder(size - 4, 3);
        Finder(3, size - 4);
        if (ver >= 2)
        {
            var n = ver / 7 + 2;
            var step = ver == 32 ? 26 : (ver * 4 + n * 2 + 1) / (n * 2 - 2) * 2;
            var pos = new int[n];
            pos[0] = 6;
            for (int i = n - 1, p = size - 7; i >= 1; i--, p -= step) pos[i] = p;
            for (var i = 0; i < n; i++)
                for (var j = 0; j < n; j++)
                {
                    if (i == 0 && j == 0 || i == 0 && j == n - 1 || i == n - 1 && j == 0) continue;
                    for (var dy = -2; dy <= 2; dy++)
                        for (var dx = -2; dx <= 2; dx++)
                            Set(m, f, pos[i] + dx, pos[j] + dy, Math.Max(Math.Abs(dx), Math.Abs(dy)) != 1);
                }
        }
        // 预留格式信息位
        DrawFormat(m, f, 0, size);
        if (ver >= 7)
        {
            var rem = ver;
            for (var i = 0; i < 12; i++) rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
            var bits = ver << 12 | rem;
            for (var i = 0; i < 18; i++)
            {
                var bit = ((bits >> i) & 1) != 0;
                int a = size - 11 + i % 3, b = i / 3;
                Set(m, f, a, b, bit);
                Set(m, f, b, a, bit);
            }
        }
    }

    private static void DrawFormat(bool[,] m, bool[,] f, int mask, int size)
    {
        // 纠错等级 M 的格式位是 0
        var data = 0 << 3 | mask;
        var rem = data;
        for (var i = 0; i < 10; i++) rem = (rem << 1) ^ ((rem >> 9) * 0x537);
        var bits = (data << 10 | rem) ^ 0x5412;
        bool B(int i) => ((bits >> i) & 1) != 0;
        for (var i = 0; i <= 5; i++) Set(m, f, 8, i, B(i));
        Set(m, f, 8, 7, B(6));
        Set(m, f, 8, 8, B(7));
        Set(m, f, 7, 8, B(8));
        for (var i = 9; i < 15; i++) Set(m, f, 14 - i, 8, B(i));
        for (var i = 0; i < 8; i++) Set(m, f, size - 1 - i, 8, B(i));
        for (var i = 8; i < 15; i++) Set(m, f, 8, size - 15 + i, B(i));
        Set(m, f, 8, size - 8, true);
    }

    private static void DrawCodewords(bool[,] m, bool[,] f, byte[] data, int size)
    {
        var i = 0;
        for (var right = size - 1; right >= 1; right -= 2)
        {
            if (right == 6) right = 5;
            for (var vert = 0; vert < size; vert++)
                for (var j = 0; j < 2; j++)
                {
                    var x = right - j;
                    var upward = ((right + 1) & 2) == 0;
                    var y = upward ? size - 1 - vert : vert;
                    if (!f[y, x] && i < data.Length * 8)
                    {
                        m[y, x] = ((data[i >> 3] >> (7 - (i & 7))) & 1) != 0;
                        i++;
                    }
                }
        }
    }

    private static void ApplyMask(bool[,] m, bool[,] f, int mask, int size)
    {
        for (var y = 0; y < size; y++)
            for (var x = 0; x < size; x++)
            {
                if (f[y, x]) continue;
                var invert = mask switch
                {
                    0 => (x + y) % 2 == 0,
                    1 => y % 2 == 0,
                    2 => x % 3 == 0,
                    3 => (x + y) % 3 == 0,
                    4 => (x / 3 + y / 2) % 2 == 0,
                    5 => x * y % 2 + x * y % 3 == 0,
                    6 => (x * y % 2 + x * y % 3) % 2 == 0,
                    _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
                };
                if (invert) m[y, x] = !m[y, x];
            }
    }

    private static int Penalty(bool[,] m, int size)
    {
        var result = 0;
        // 同色连续 5 个以上
        for (var pass = 0; pass < 2; pass++)
            for (var a = 0; a < size; a++)
            {
                var run = 1;
                for (var b = 1; b < size; b++)
                {
                    var cur = pass == 0 ? m[a, b] : m[b, a];
                    var prev = pass == 0 ? m[a, b - 1] : m[b - 1, a];
                    if (cur == prev) run++;
                    else
                    {
                        if (run >= 5) result += run - 2;
                        run = 1;
                    }
                }
                if (run >= 5) result += run - 2;
            }
        // 2×2 同色块
        for (var y = 0; y < size - 1; y++)
            for (var x = 0; x < size - 1; x++)
                if (m[y, x] == m[y, x + 1] && m[y, x] == m[y + 1, x] && m[y, x] == m[y + 1, x + 1]) result += 3;
        // 类似定位图案的 1:1:3:1:1
        for (var pass = 0; pass < 2; pass++)
            for (var a = 0; a < size; a++)
                for (var b = 0; b + 10 < size; b++)
                {
                    bool G(int k) => pass == 0 ? m[a, b + k] : m[b + k, a];
                    var p1 = G(0) && !G(1) && G(2) && G(3) && G(4) && !G(5) && G(6) && !G(7) && !G(8) && !G(9) && !G(10);
                    var p2 = !G(0) && !G(1) && !G(2) && !G(3) && G(4) && !G(5) && G(6) && G(7) && G(8) && !G(9) && G(10);
                    if (p1 || p2) result += 40;
                }
        // 深色比例
        var dark = 0;
        foreach (var v in m) if (v) dark++;
        var total = size * size;
        var k2 = (Math.Abs(dark * 20 - total * 10) + total - 1) / total - 1;
        result += Math.Max(0, k2) * 10;
        return result;
    }
}
