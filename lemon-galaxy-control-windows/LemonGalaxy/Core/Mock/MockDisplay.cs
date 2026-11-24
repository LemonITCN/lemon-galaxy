using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Drawing.Text;
using System.IO;
using System.Runtime.InteropServices;

namespace LemonGalaxy.Core.Mock;

/// <summary>
/// 假远程桌面：用 GDI+ 画一张「被控端桌面」，按帧率经 VideoSink（on_config / on_frame / on_cursor）推给界面，
/// 数据路径与真内核一致；只是帧内容不是 H.264，而是「LGMK + 宽 + 高 + BGRA」原始帧，由 MockFrameDecoder 还原。
/// </summary>
internal sealed class MockDisplayChannel : ICoreDisplay
{
    private readonly VideoSink sink;
    private readonly MockDesktopRenderer renderer;
    private readonly DisplayInfo display;
    private readonly Timer timer;
    private readonly object lk = new();
    private bool stopped, stopCalled;
    private ulong frame;
    private readonly int w, h;
    private readonly bool zh;
    private readonly uint kbps;
    /// <summary>模拟录像：不写文件，只记下路径与开始时间（Clock 毫秒，selftest 下时钟冻结）</summary>
    private string? recordPath;
    private long recordStartMs;

    public MockDisplayChannel(DeviceInfo device, DisplayInfo display, DisplayOptions options, VideoSink sink, bool zh, bool live)
    {
        this.sink = sink;
        this.display = display;
        this.zh = zh;
        kbps = options.BitrateKbps;
        // 模拟端不按原始分辨率出流（CPU 画图太慢），长边限制在 1280，宽高取偶数
        var scale = Math.Min(1.0, 1280.0 / Math.Max(display.Width, display.Height));
        w = (int)(display.Width * scale) & ~1;
        h = (int)(display.Height * scale) & ~1;
        renderer = new MockDesktopRenderer(device, zh, w, h);
        var fps = Math.Max(5, Math.Min((int)options.MaxFps, 15));
        sink.OnConfig(VideoCodec.H264, (uint)w, (uint)h);
        timer = new Timer(_ => Render(), null, 0, live ? 1000 / fps : Timeout.Infinite);
    }

    private void Render()
    {
        byte[] payload;
        lock (lk)
        {
            if (stopped) return;
            payload = renderer.RawFrame(DateTime.Now);
            frame++;
        }
        sink.OnFrame(payload, frame * 66_666, frame == 1);
    }

    private (int x, int y) ToStream(int x, int y) => ((int)((double)x * w / display.Width), (int)((double)y * h / display.Height));

    public void SendInput(InputEvent e)
    {
        lock (lk)
        {
            switch (e)
            {
                case InputEvent.MouseMove m:
                    renderer.Cursor = ToStream(m.X, m.Y);
                    sink.OnCursor(m.X, m.Y, true);
                    break;
                case InputEvent.MouseButton b when b.Down:
                    renderer.Clicks.Add((ToStream(b.X, b.Y), DateTime.Now));
                    break;
                case InputEvent.Text t:
                    renderer.Typed += t.TextValue;
                    break;
                case InputEvent.Key k when k.Down:
                    if (k.HidUsage == 0x2A && renderer.Typed.Length > 0) renderer.Typed = renderer.Typed.Substring(0, renderer.Typed.Length - 1);
                    else if (k.HidUsage == 0x28) renderer.Typed += "\n";
                    else if (k.HidUsage >= 0x04 && k.HidUsage <= 0x1D) renderer.Typed += (char)('a' + (int)k.HidUsage - 0x04);
                    else if (k.HidUsage == 0x2C) renderer.Typed += " ";
                    break;
            }
        }
        // 模拟端不跑定时器时（selftest），输入后补一帧
        Render();
    }

    public void RequestKeyframe() { }
    public void SetBitrate(uint kbps) { }

    public void Stop()
    {
        bool first;
        lock (lk)
        {
            first = !stopCalled;
            stopCalled = stopped = true;
            // 与内核一致：关画面时自动结束录像
            recordPath = null;
        }
        timer.Dispose();
        // 与 FFI 一致：自己停的也回一次 on_closed("stopped")，界面要认得出不是断线
        if (first) sink.OnClosed("stopped");
    }

    /// <summary>selftest：模拟画面被被控端结束（流断了、看门狗判定画面没有响应），与内核一样通知一次 on_closed</summary>
    public void SimulateEnded(string reason)
    {
        lock (lk)
        {
            if (stopped) return;
            stopped = true;
            recordPath = null;
        }
        timer.Dispose();
        sink.OnClosed(reason);
    }

    public Task StartRecording(string path, bool withAudio)
    {
        lock (lk)
        {
            if (stopped) return Task.FromException(new GalaxyException.Other(zh ? "画面已关闭" : "The display is closed"));
            if (recordPath != null) return Task.FromException(new GalaxyException.Other(zh ? "已经在录像" : "Already recording"));
            recordPath = path;
            recordStartMs = Clock.NowMs;
        }
        return Task.CompletedTask;
    }

    public RecordingInfo StopRecording()
    {
        lock (lk)
        {
            if (recordPath == null) throw new GalaxyException.Other(zh ? "没有在录像" : "Not recording");
            var ms = (ulong)Math.Max(0, Clock.NowMs - recordStartMs);
            var info = new RecordingInfo(new[] { recordPath }, ms, ms * kbps / 8);
            recordPath = null;
            return info;
        }
    }

    public bool IsRecording() { lock (lk) return recordPath != null; }
}

/// <summary>模拟内核的「解码器」：还原 MockDisplayChannel 的原始帧</summary>
internal sealed class MockFrameDecoder : IFrameDecoder
{
    public DecodedFrame? Decode(byte[] data)
    {
        if (data.Length < 12 || data[0] != (byte)'L' || data[1] != (byte)'G' || data[2] != (byte)'M' || data[3] != (byte)'K') return null;
        var w = BitConverter.ToInt32(data, 4);
        var h = BitConverter.ToInt32(data, 8);
        if (w <= 0 || h <= 0 || data.Length < 12 + w * h * 4) return null;
        var bgra = new byte[w * h * 4];
        Buffer.BlockCopy(data, 12, bgra, 0, bgra.Length);
        return new DecodedFrame((uint)w, (uint)h, bgra);
    }
}

/// <summary>画一张假桌面：Windows 风格（底部任务栏）或 macOS 风格（顶部菜单栏 + 程序坞），外加资源监视器和记事本两个窗口。</summary>
internal sealed class MockDesktopRenderer
{
    private readonly DeviceInfo device;
    private readonly bool zh;
    public readonly int W, H;
    public (int x, int y) Cursor = (-100, -100);
    public readonly List<((int x, int y) p, DateTime at)> Clicks = new();
    public string Typed = "";
    private Bitmap? background;
    private readonly string fontName;

    public MockDesktopRenderer(DeviceInfo device, bool zh, int w, int h)
    {
        this.device = device;
        this.zh = zh;
        W = w;
        H = h;
        fontName = zh ? "Microsoft YaHei" : "Segoe UI";
    }

    private bool IsWindows => device.Platform == Platform.Windows;
    private float U => H / 900f;

    private Font F(float size) => new(fontName, Math.Max(1f, size * U), GraphicsUnit.Pixel);

    private RectangleF EditorRect => new(W * 0.1f, H * 0.2f, W * 0.38f, H * 0.5f);
    private RectangleF MonitorRect => new(W * 0.52f, H * 0.12f, W * 0.4f, H * 0.42f);

    /// <summary>记事本「保存」按钮在画面里的位置（AI 点击演示用）</summary>
    public PointF SaveButtonPoint => new(EditorRect.Right - 40 * U, EditorRect.Top + 15 * U);

    private static GraphicsPath Rounded(RectangleF r, float rad)
    {
        var p = new GraphicsPath();
        var d = Math.Max(0.1f, rad * 2);
        p.AddArc(r.X, r.Y, d, d, 180, 90);
        p.AddArc(r.Right - d, r.Y, d, d, 270, 90);
        p.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
        p.AddArc(r.X, r.Bottom - d, d, d, 90, 90);
        p.CloseFigure();
        return p;
    }

    private Bitmap Background()
    {
        if (background != null) return background;
        var bmp = new Bitmap(W, H, PixelFormat.Format32bppPArgb);
        using var g = Graphics.FromImage(bmp);
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
        var rect = new Rectangle(0, 0, W, H);
        var c1 = IsWindows ? Color.FromArgb(13, 69, 158) : Color.FromArgb(46, 31, 97);
        var c2 = IsWindows ? Color.FromArgb(84, 158, 237) : Color.FromArgb(219, 115, 107);
        using (var lg = new LinearGradientBrush(rect, c1, c2, IsWindows ? 30f : 90f)) g.FillRectangle(lg, rect);
        // 柔和的光斑
        var spots = new[] { (0.2f, 0.3f), (0.75f, 0.6f), (0.5f, 0.85f) };
        for (var i = 0; i < spots.Length; i++)
        {
            var r = H * (0.35f + i * 0.1f);
            using var gp = new GraphicsPath();
            gp.AddEllipse(W * spots[i].Item1 - r, H * spots[i].Item2 - r, r * 2, r * 2);
            using var pb = new PathGradientBrush(gp) { CenterColor = Color.FromArgb(26, 255, 255, 255), SurroundColors = new[] { Color.FromArgb(0, 255, 255, 255) } };
            g.FillPath(pb, gp);
        }
        using var white = new SolidBrush(Color.FromArgb(230, 255, 255, 255));
        if (IsWindows)
        {
            var icons = zh ? new[] { "此电脑", "回收站", "文档", "LemonGalaxy" } : new[] { "This PC", "Recycle Bin", "Documents", "LemonGalaxy" };
            using var f = F(12);
            using var sf = new StringFormat { Alignment = StringAlignment.Center };
            for (var i = 0; i < icons.Length; i++)
            {
                var r = new RectangleF(24 * U, (24 + i * 96) * U, 56 * U, 48 * U);
                using var p = Rounded(new RectangleF(r.X + 8 * U, r.Y + 4 * U, r.Width - 16 * U, r.Height - 8 * U), 6 * U);
                g.FillPath(white, p);
                g.DrawString(icons[i], f, Brushes.White, new RectangleF(r.X - 12 * U, r.Bottom + 4 * U, 80 * U, 40 * U), sf);
            }
            var bar = new RectangleF(0, H - 48 * U, W, 48 * U);
            using (var bb = new SolidBrush(Color.FromArgb(235, 237, 240, 245))) g.FillRectangle(bb, bar);
            var colors = new[] { Color.FromArgb(0, 122, 255), Color.FromArgb(48, 176, 199), Color.FromArgb(255, 149, 0), Color.FromArgb(52, 199, 89), Color.FromArgb(175, 82, 222), Color.FromArgb(255, 204, 0) };
            for (var i = 0; i < 6; i++)
            {
                using var b = new SolidBrush(colors[i]);
                using var p = Rounded(new RectangleF(W / 2f - 150 * U + i * 50 * U, bar.Top + 8 * U, 32 * U, 32 * U), 6 * U);
                g.FillPath(b, p);
            }
        }
        else
        {
            using (var bb = new SolidBrush(Color.FromArgb(64, 0, 0, 0))) g.FillRectangle(bb, 0, 0, W, 26 * U);
            using var f = F(13);
            g.DrawString(zh ? "访达   文件   编辑   显示   前往   窗口   帮助" : "Finder   File   Edit   View   Go   Window   Help", f, Brushes.White, 40 * U, 4 * U);
            var dock = new RectangleF(W / 2f - 240 * U, H - 74 * U, 480 * U, 64 * U);
            using (var db = new SolidBrush(Color.FromArgb(72, 255, 255, 255)))
            using (var dp = Rounded(dock, 18 * U)) g.FillPath(db, dp);
            var colors = new[] { Color.FromArgb(0, 122, 255), Color.Gray, Color.FromArgb(52, 199, 89), Color.FromArgb(255, 149, 0), Color.FromArgb(255, 59, 48), Color.FromArgb(175, 82, 222), Color.FromArgb(48, 176, 199), Color.FromArgb(88, 86, 214) };
            for (var i = 0; i < 8; i++)
            {
                using var b = new SolidBrush(colors[i]);
                using var p = Rounded(new RectangleF(dock.X + 14 * U + i * 58 * U, dock.Y + 8 * U, 48 * U, 48 * U), 11 * U);
                g.FillPath(b, p);
            }
        }
        Window(g, MonitorRect, zh ? "资源监视器" : "Resource Monitor");
        var m = MonitorRect;
        var vals = new[] { 0.42, 0.67, 0.31, 0.55, 0.78, 0.49, 0.62, 0.38, 0.71, 0.44, 0.58, 0.66 };
        using (var blue = new SolidBrush(Color.FromArgb(190, 0, 122, 255)))
        {
            for (var i = 0; i < vals.Length; i++)
            {
                var bw = (m.Width - 40 * U) / 12;
                var bh = (float)((m.Height - 90 * U) * vals[i]);
                using var p = Rounded(new RectangleF(m.X + 20 * U + i * bw + 2 * U, m.Bottom - 20 * U - bh, bw - 4 * U, bh), 2 * U);
                g.FillPath(blue, p);
            }
        }
        Window(g, EditorRect, zh ? "无标题 - 记事本" : "Untitled - Notepad");
        background = bmp;
        return bmp;
    }

    private void Window(Graphics g, RectangleF r, string title)
    {
        for (var i = 6; i >= 1; i--)
        {
            using var sh = new SolidBrush(Color.FromArgb(10, 0, 0, 0));
            using var sp = Rounded(new RectangleF(r.X - i * U, r.Y + (6 - i) * U, r.Width + i * 2 * U, r.Height + i * 2 * U), 10 * U);
            g.FillPath(sh, sp);
        }
        using (var p = Rounded(r, 8 * U)) g.FillPath(Brushes.White, p);
        using (var tb = new SolidBrush(Color.FromArgb(242, 242, 242)))
        {
            using var tp = Rounded(new RectangleF(r.X, r.Y, r.Width, 30 * U), 8 * U);
            g.FillPath(tb, tp);
            g.FillRectangle(tb, r.X, r.Y + 20 * U, r.Width, 10 * U);
        }
        using var f = F(12);
        using var tc = new SolidBrush(Color.FromArgb(51, 51, 51));
        g.DrawString(title, f, tc, r.X + 14 * U, r.Y + 7 * U);
    }

    public void Draw(Graphics g, DateTime time)
    {
        g.DrawImageUnscaled(Background(), 0, 0);
        g.SmoothingMode = SmoothingMode.AntiAlias;
        g.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
        var r = EditorRect;
        var body = Typed.Length == 0
            ? (zh ? "这是模拟的远程桌面画面。\n在这里点击、打字，\n输入会通过 InputEvent 发到被控端。" : "This is a simulated remote desktop.\nClick and type here;\ninput is sent as InputEvent.")
            : Typed;
        using (var f = F(15))
        using (var tc = new SolidBrush(Color.FromArgb(38, 38, 38)))
            g.DrawString(body + ((long)(time - DateTime.MinValue).TotalMilliseconds / 500 % 2 == 0 ? "|" : " "), f, tc,
                new RectangleF(r.X + 16 * U, r.Y + 44 * U, r.Width - 32 * U, r.Height - 60 * U));
        var clock = time.ToString("HH:mm:ss");
        using (var f = F(13))
        {
            if (IsWindows)
            {
                using var tc = new SolidBrush(Color.FromArgb(26, 26, 26));
                g.DrawString(clock, f, tc, W - 90 * U, H - 34 * U);
            }
            else g.DrawString(clock, f, Brushes.White, W - 90 * U, 5 * U);
        }
        Clicks.RemoveAll(c => (time - c.at).TotalSeconds > 0.6);
        foreach (var (p, at) in Clicks)
        {
            var k = (float)((time - at).TotalSeconds / 0.6);
            var rad = (8 + 26 * k) * U;
            using var pen = new Pen(Color.FromArgb((int)(230 * (1 - k)), 255, 204, 0), 3 * U);
            g.DrawEllipse(pen, p.x - rad, p.y - rad, rad * 2, rad * 2);
        }
    }

    /// <summary>「LGMK + 宽 + 高 + BGRA」原始帧</summary>
    public byte[] RawFrame(DateTime time)
    {
        using var bmp = new Bitmap(W, H, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(bmp)) Draw(g, time);
        var data = bmp.LockBits(new Rectangle(0, 0, W, H), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        try
        {
            var outBuf = new byte[12 + W * H * 4];
            outBuf[0] = (byte)'L'; outBuf[1] = (byte)'G'; outBuf[2] = (byte)'M'; outBuf[3] = (byte)'K';
            BitConverter.GetBytes(W).CopyTo(outBuf, 4);
            BitConverter.GetBytes(H).CopyTo(outBuf, 8);
            for (var y = 0; y < H; y++) Marshal.Copy(data.Scan0 + y * data.Stride, outBuf, 12 + y * W * 4, W * 4);
            return outBuf;
        }
        finally { bmp.UnlockBits(data); }
    }

    /// <summary>截图（AI 的 screenshot 工具、点击标注图用）：PNG</summary>
    public ImageData Png(int maxWidth, PointF? mark = null)
    {
        var w = Math.Min(maxWidth > 0 ? maxWidth : W, W);
        var h = (int)((double)w * H / W);
        using var full = new Bitmap(W, H, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(full))
        {
            Draw(g, Fmt0.Frozen);
            if (mark is { } m)
            {
                g.SmoothingMode = SmoothingMode.AntiAlias;
                using var pen = new Pen(Color.FromArgb(255, 59, 48), 4 * U);
                var rad = 18 * U;
                g.DrawEllipse(pen, m.X - rad, m.Y - rad, rad * 2, rad * 2);
            }
        }
        using var scaled = new Bitmap(w, h, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(scaled))
        {
            g.InterpolationMode = InterpolationMode.HighQualityBicubic;
            g.DrawImage(full, 0, 0, w, h);
        }
        using var ms = new MemoryStream();
        scaled.Save(ms, ImageFormat.Png);
        return new ImageData("image/png", ms.ToArray(), (uint)w, (uint)h);
    }
}

internal static class Fmt0
{
    /// <summary>截图里用的固定时间（selftest 冻结时间时用冻结值）</summary>
    public static DateTime Frozen => DateTimeOffset.FromUnixTimeMilliseconds(Clock.NowMs).LocalDateTime;
}
