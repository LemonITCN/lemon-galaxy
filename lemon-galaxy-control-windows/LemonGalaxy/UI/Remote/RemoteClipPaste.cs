using System.Runtime.ExceptionServices;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;
using System.Windows.Threading;
using LemonGalaxy.Core;
using FILETIME = System.Runtime.InteropServices.ComTypes.FILETIME;
using STATSTG = System.Runtime.InteropServices.ComTypes.STATSTG;

namespace LemonGalaxy.UI;

// 远端复制 → 本机粘贴（protocol.md 8.4.1）：远端剪贴板里的文件展开成清单（不下载），做成 OLE 虚拟文件数据对象放进本机剪贴板：
//   FileGroupDescriptorW（各项相对路径、大小、修改时间，带 FD_PROGRESSUI）+ FileContents（按项给 IStream，读时才经内核从对方取）
//   + Preferred DropEffect = 复制 + 私有格式「LemonGalaxy Remote Files」（认出是我们放的，不当成本机复制再传回去）。
// 资源管理器里 Ctrl+V 或右键粘贴时用它自己的「正在复制」进度窗口，这时才真正传。
//
// 线程：数据对象由单独的 STA 线程（剪贴板线程）放进剪贴板，资源管理器的跨进程调用（GetData、IStream.Read）都派发到这个线程，
// 不经过也不卡界面线程。读对方数据会阻塞，放到线程池里做，剪贴板线程用 COM 模态等待：等的时候照常处理别的 COM 调用和
// 跨线程发来的窗口消息（别的程序复制时系统要给剪贴板主人发 WM_DESTROYCLIPBOARD，不能让它们跟着卡住）。

/// <summary>剪贴板线程：放 / 清虚拟文件数据对象，判断剪贴板现在是不是我们放的</summary>
internal static class VirtualClipboard
{
    private static readonly object Lk = new();
    private static Dispatcher? dispatcher;
    private static volatile int threadId;
    private static volatile RemoteFileData? current;

    private static Dispatcher EnsureThread()
    {
        lock (Lk)
        {
            if (dispatcher != null) return dispatcher;
            using var ready = new ManualResetEventSlim();
            var t = new Thread(() =>
            {
                NativeClip.OleInitialize(IntPtr.Zero);
                threadId = NativeClip.GetCurrentThreadId();
                var d = Dispatcher.CurrentDispatcher;
                d.UnhandledException += (_, e) =>
                {
                    Log.Error("剪贴板线程出错：" + e.Exception);
                    e.Handled = true;
                };
                dispatcher = d;
                ready.Set();
                Dispatcher.Run();
            }) { IsBackground = true, Name = "LemonGalaxy 剪贴板文件" };
            t.SetApartmentState(ApartmentState.STA);
            t.Start();
            ready.Wait();
            return dispatcher!;
        }
    }

    /// <summary>剪贴板现在归剪贴板线程，也就是放着我们的对方文件（只看剪贴板主人窗口属于哪个线程，不走 OLE，不会被正在进行的粘贴卡住）</summary>
    public static bool OwnsClipboard
    {
        get
        {
            var tid = threadId;
            if (tid == 0) return false;
            var owner = NativeClip.GetClipboardOwner();
            return owner != IntPtr.Zero && NativeClip.GetWindowThreadProcessId(owner, out _) == tid;
        }
    }

    /// <summary>这份数据对象还在本机剪贴板上</summary>
    public static bool Holds(RemoteFileData data) => ReferenceEquals(current, data) && OwnsClipboard;

    /// <summary>放进本机剪贴板（剪贴板被别的程序占着时重试几次）；成功后 data.Placed 为真</summary>
    public static Task<bool> Set(RemoteFileData data)
    {
        var tcs = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        EnsureThread().BeginInvoke(new Action(() =>
        {
            var hr = 0;
            for (var i = 0; i < 10; i++)
            {
                hr = NativeClip.OleSetClipboard(data);
                if (hr == 0) break;
                System.Threading.Thread.Sleep(30);
            }
            if (hr == 0)
            {
                current = data;
                data.Placed = true;
            }
            else Log.Error($"放对方文件到剪贴板失败：0x{hr:X8}");
            tcs.TrySetResult(hr == 0);
        }));
        return tcs.Task;
    }

    /// <summary>它还在本机剪贴板上就清掉（会话断了，免得之后粘贴到一份已经断开的清单）；别人的内容不动</summary>
    public static void Clear(RemoteFileData data)
    {
        if (dispatcher == null) return;
        dispatcher.BeginInvoke(new Action(() => ClearOnThread(data)));
    }

    private static void ClearOnThread(RemoteFileData data)
    {
        if (!ReferenceEquals(current, data)) return;
        current = null;
        if (NativeClip.OleIsCurrentClipboard(data) == 0) NativeClip.OleSetClipboard(null);
    }

    /// <summary>退出前清掉剪贴板里我们的对方文件：进程没了它也读不了，留着只会让粘贴失败（最多等 1 秒）</summary>
    public static void Shutdown()
    {
        var d = dispatcher;
        var c = current;
        if (d == null || c == null) return;
        try { d.Invoke(DispatcherPriority.Send, TimeSpan.FromSeconds(1), new Action(() => ClearOnThread(c))); }
        catch (Exception e) { Log.Info("退出时清剪贴板失败：" + e.Message); }
    }
}

/// <summary>
/// 对方剪贴板里文件的虚拟文件数据对象。会话断开或关标签时 Cancel（之后读都报网络错误）；
/// 剪贴板被换掉时 Detach（正在粘贴的照常读完，闲下来后自动作废）。打开过的读取器闲 30 秒自动关掉，再读时从原位置重开。
/// </summary>
[ComVisible(true)]
internal sealed class RemoteFileData : IOleDataObject, IDataObjectAsyncCapability
{
    public static readonly short CfDescriptor = NativeClip.Format("FileGroupDescriptorW");
    public static readonly short CfContents = NativeClip.Format("FileContents");
    public static readonly short CfDropEffect = NativeClip.Format("Preferred DropEffect");
    /// <summary>私有格式：内容是清单 id（8 字节），认出是我们放的</summary>
    public static readonly short CfMarker = NativeClip.Format("LemonGalaxy Remote Files");

    public const int DescriptorSize = 592;
    public const uint FdAttributes = 0x4, FdWriteTime = 0x20, FdFileSize = 0x40, FdProgressUi = 0x4000;
    public const uint AttrDirectory = 0x10, AttrNormal = 0x80;
    private const int MaxName = 259;
    private const int ReaderIdleMs = 30_000, DetachedIdleMs = 120_000;

    private readonly ICoreRemoteClipOffer offer;
    private readonly byte[] descriptor;
    /// <summary>描述表第 i 项对应清单里的哪一项（放不下的长路径跳过了，序号会错开）</summary>
    private readonly uint[] offerIndex;
    private readonly ClipOfferEntry[] items;
    private readonly List<WeakReference<RemoteFileStream>> streams = new();
    private readonly FORMATETC[] formats;
    private volatile bool cancelled, detached;
    private int lastUse = Environment.TickCount;
    private volatile bool asyncMode = true, inAsyncOp;

    public readonly ulong OfferId;
    /// <summary>描述表里的路径（反斜杠分隔，已换掉 Windows 文件名里不能有的字符）</summary>
    public readonly string[] Names;
    /// <summary>路径太长（超过 259 个字符）放不进描述表、跳过的项数</summary>
    public readonly int Skipped;
    /// <summary>已放进本机剪贴板（剪贴板线程设）</summary>
    public volatile bool Placed;

    public RemoteFileData(ICoreRemoteClipOffer offer)
    {
        this.offer = offer;
        var info = offer.Info();
        OfferId = info.Id;
        var idx = new List<uint>();
        var list = new List<ClipOfferEntry>();
        var names = new List<string>();
        for (var i = 0; i < info.Entries.Length; i++)
        {
            var e = info.Entries[i];
            var name = WindowsPath(e.Path);
            if (name == null)
            {
                Skipped++;
                continue;
            }
            idx.Add((uint)i);
            list.Add(e);
            names.Add(name);
        }
        if (Skipped > 0) Log.Info($"对方文件里有 {Skipped} 项路径太长，粘贴时跳过");
        offerIndex = idx.ToArray();
        items = list.ToArray();
        Names = names.ToArray();
        descriptor = BuildDescriptor(items, Names);
        formats = new[]
        {
            Formatetc(CfDescriptor, TYMED.TYMED_HGLOBAL),
            Formatetc(CfContents, TYMED.TYMED_ISTREAM),
            Formatetc(CfDropEffect, TYMED.TYMED_HGLOBAL),
            Formatetc(CfMarker, TYMED.TYMED_HGLOBAL),
        };
        Watchdog.Add(this);
    }

    public int Count => items.Length;
    public bool Cancelled => cancelled;
    /// <summary>selftest 核对用</summary>
    internal ICoreRemoteClipOffer Offer => offer;

    private static FORMATETC Formatetc(short cf, TYMED tymed) => new() { cfFormat = cf, dwAspect = DVASPECT.DVASPECT_CONTENT, lindex = -1, ptd = IntPtr.Zero, tymed = tymed };

    // ---------- 路径与描述表 ----------

    private static readonly HashSet<string> Reserved = new(StringComparer.OrdinalIgnoreCase)
    {
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    };

    /// <summary>清单路径（/ 分隔）→ 描述表路径（\ 分隔）；每段换掉 Windows 不允许的字符、结尾的点与空格、设备名。太长时为 null</summary>
    public static string? WindowsPath(string path)
    {
        var parts = path.Split('/');
        var sb = new StringBuilder(path.Length + 4);
        foreach (var part in parts)
        {
            if (part.Length == 0) return null;
            if (sb.Length > 0) sb.Append('\\');
            sb.Append(SafeSegment(part));
        }
        return sb.Length is > 0 and <= MaxName ? sb.ToString() : null;
    }

    private static string SafeSegment(string s)
    {
        var chars = s.ToCharArray();
        for (var i = 0; i < chars.Length; i++)
            if (chars[i] < 32 || "<>:\"|?*\\".IndexOf(chars[i]) >= 0) chars[i] = '_';
        var r = new string(chars);
        if (r.EndsWith(".", StringComparison.Ordinal) || r.EndsWith(" ", StringComparison.Ordinal)) r = r.TrimEnd('.', ' ') + "_";
        var dot = r.IndexOf('.');
        if (Reserved.Contains(dot < 0 ? r : r.Substring(0, dot))) r = "_" + r;
        return r;
    }

    /// <summary>Unix 秒 → FILETIME（100 纳秒，从 1601 年起）</summary>
    public static long FileTime(long unixSeconds) => unixSeconds * 10_000_000L + 116_444_736_000_000_000L;

    /// <summary>FILEGROUPDESCRIPTORW：cItems + FILEDESCRIPTORW[]（每项 592 字节，文件名在第 72 字节起，最多 260 个 UTF-16 字符含结尾 0）</summary>
    private static byte[] BuildDescriptor(ClipOfferEntry[] items, string[] names)
    {
        var buf = new byte[4 + DescriptorSize * items.Length];
        Put32(buf, 0, (uint)items.Length);
        for (var i = 0; i < items.Length; i++)
        {
            var e = items[i];
            var o = 4 + i * DescriptorSize;
            var flags = FdAttributes | FdProgressUi;
            Put32(buf, o + 36, e.IsDir ? AttrDirectory : AttrNormal);
            if (e.Mtime > 0)
            {
                flags |= FdWriteTime;
                Put64(buf, o + 56, FileTime(e.Mtime));
            }
            if (!e.IsDir)
            {
                flags |= FdFileSize;
                Put32(buf, o + 64, (uint)(e.Size >> 32));
                Put32(buf, o + 68, (uint)e.Size);
            }
            Put32(buf, o, flags);
            var name = Encoding.Unicode.GetBytes(names[i]);
            Buffer.BlockCopy(name, 0, buf, o + 72, name.Length);
        }
        return buf;
    }

    private static void Put32(byte[] b, int at, uint v) => Buffer.BlockCopy(BitConverter.GetBytes(v), 0, b, at, 4);
    private static void Put64(byte[] b, int at, long v) => Buffer.BlockCopy(BitConverter.GetBytes(v), 0, b, at, 8);

    // ---------- IDataObject ----------

    public int GetData(ref FORMATETC format, out STGMEDIUM medium)
    {
        medium = default;
        try
        {
            var hr = Check(ref format);
            if (hr != Hr.Ok) return hr;
            if (format.cfFormat == CfDescriptor) return Hr.Global(descriptor, out medium);
            if (format.cfFormat == CfDropEffect) return Hr.Global(BitConverter.GetBytes(1u), out medium);
            if (format.cfFormat == CfMarker) return Hr.Global(BitConverter.GetBytes(OfferId), out medium);
            // FileContents：按项给 IStream，读时才打开
            if (cancelled) return Hr.Network;
            Touch();
            var i = format.lindex;
            var s = new RemoteFileStream(this, offerIndex[i], items[i]);
            lock (streams) streams.Add(new WeakReference<RemoteFileStream>(s));
            medium.tymed = TYMED.TYMED_ISTREAM;
            medium.unionmember = Marshal.GetComInterfaceForObject(s, typeof(IOleStream));
            return Hr.Ok;
        }
        catch (Exception e)
        {
            Log.Error("剪贴板取数据出错：" + e);
            return Hr.Unexpected;
        }
    }

    /// <summary>是不是我们有的格式（FileContents 的 lindex 要指向文件；-1 只用于问有没有）</summary>
    private int Check(ref FORMATETC f, bool query = false)
    {
        if (f.dwAspect != DVASPECT.DVASPECT_CONTENT) return Hr.Aspect;
        if (f.cfFormat == CfContents)
        {
            if ((f.tymed & TYMED.TYMED_ISTREAM) == 0) return Hr.Tymed;
            if (query && f.lindex == -1) return Hr.Ok;
            return f.lindex >= 0 && f.lindex < items.Length && !items[f.lindex].IsDir ? Hr.Ok : Hr.Lindex;
        }
        if (f.cfFormat == CfDescriptor || f.cfFormat == CfDropEffect || f.cfFormat == CfMarker)
            return (f.tymed & TYMED.TYMED_HGLOBAL) != 0 ? Hr.Ok : Hr.Tymed;
        return Hr.Format;
    }

    public int GetDataHere(ref FORMATETC format, ref STGMEDIUM medium) => Hr.NotImpl;
    public int QueryGetData(ref FORMATETC format) => Check(ref format, query: true);

    public int GetCanonicalFormatEtc(ref FORMATETC formatIn, out FORMATETC formatOut)
    {
        formatOut = formatIn;
        formatOut.ptd = IntPtr.Zero;
        return Hr.SameFormatEtc;
    }

    /// <summary>粘贴方回写的「Performed DropEffect」「Paste Succeeded」等不需要，不接收（介质仍归调用方）</summary>
    public int SetData(ref FORMATETC formatIn, ref STGMEDIUM medium, bool release) => Hr.NotImpl;

    public int EnumFormatEtc(DATADIR direction, out IEnumFORMATETC? enumerator)
    {
        enumerator = direction == DATADIR.DATADIR_GET ? new FormatEnumerator(formats, 0) : null;
        return enumerator != null ? Hr.Ok : Hr.NotImpl;
    }

    public int DAdvise(ref FORMATETC format, ADVF advf, IAdviseSink sink, out int connection)
    {
        connection = 0;
        return Hr.AdviseNotSupported;
    }

    public int DUnadvise(int connection) => Hr.AdviseNotSupported;

    public int EnumDAdvise(out IEnumSTATDATA? enumerator)
    {
        enumerator = null;
        return Hr.AdviseNotSupported;
    }

    // ---------- IDataObjectAsyncCapability：资源管理器可以在后台线程里取数据 ----------

    public int SetAsyncMode(bool doOpAsync) { asyncMode = doOpAsync; return Hr.Ok; }
    public int GetAsyncMode(out bool isOpAsync) { isOpAsync = asyncMode; return Hr.Ok; }
    public int StartOperation(IntPtr bindCtx) { inAsyncOp = true; Touch(); return Hr.Ok; }
    public int InOperation(out bool inAsync) { inAsync = inAsyncOp; return Hr.Ok; }
    public int EndOperation(int result, IntPtr bindCtx, uint effects) { inAsyncOp = false; Touch(); return Hr.Ok; }

    // ---------- 读取、作废 ----------

    internal void Touch() => Interlocked.Exchange(ref lastUse, Environment.TickCount);
    private int IdleMs => unchecked(Environment.TickCount - Volatile.Read(ref lastUse));

    /// <summary>打开第 index 项（会阻塞到对方应答）</summary>
    internal ICoreClipReader Open(uint index, ulong offset)
    {
        if (cancelled) throw new OperationCanceledException();
        Touch();
        return offer.Open(index, offset);
    }

    private RemoteFileStream[] LiveStreams()
    {
        lock (streams)
        {
            streams.RemoveAll(w => !w.TryGetTarget(out _));
            return streams.Select(w => w.TryGetTarget(out var s) ? s : null).Where(s => s != null).ToArray()!;
        }
    }

    /// <summary>会话断了或关了标签：之后的读都报错，正在读的停下，释放清单</summary>
    public void Cancel()
    {
        if (cancelled) return;
        cancelled = true;
        Watchdog.Remove(this);
        var list = LiveStreams();
        ThreadPool.QueueUserWorkItem(_ =>
        {
            foreach (var s in list) s.Abort();
            try { offer.Dispose(); }
            catch (Exception e) { Log.Info("释放剪贴板清单失败：" + e.Message); }
        });
    }

    /// <summary>剪贴板已被换掉：没人在粘贴就直接作废，有人在粘贴就等它闲下来</summary>
    public void Detach()
    {
        detached = true;
        if (!inAsyncOp && IdleMs > 10_000 && !LiveStreams().Any(s => s.Busy)) Cancel();
    }

    /// <summary>每 5 秒：关掉闲了 30 秒的读取器；已离开剪贴板、闲了 2 分钟的作废</summary>
    private void Tick()
    {
        var list = LiveStreams();
        foreach (var s in list) s.CloseIfIdle(ReaderIdleMs);
        if (detached && !inAsyncOp && IdleMs > DetachedIdleMs && !list.Any(s => s.Busy)) Cancel();
    }

    private static class Watchdog
    {
        private static readonly HashSet<RemoteFileData> Live = new();
        private static Timer? timer;

        public static void Add(RemoteFileData d)
        {
            lock (Live)
            {
                Live.Add(d);
                timer ??= new Timer(_ => Run(), null, 5000, 5000);
            }
        }

        public static void Remove(RemoteFileData d)
        {
            lock (Live) Live.Remove(d);
        }

        private static void Run()
        {
            RemoteFileData[] all;
            lock (Live) all = Live.ToArray();
            foreach (var d in all)
            {
                try { d.Tick(); }
                catch (Exception e) { Log.Error("剪贴板文件巡检出错：" + e); }
            }
        }
    }
}

/// <summary>一项文件的内容流：第一次读时才向对方打开，按需从任意位置重开（Seek、闲置关掉后再读）</summary>
[ComVisible(true)]
internal sealed class RemoteFileStream : IOleStream, IOleSequentialStream
{
    private readonly RemoteFileData owner;
    private readonly uint index;
    private readonly ClipOfferEntry entry;
    private readonly object lk = new();
    private ICoreClipReader? reader;
    private ulong pos;
    /// <summary>打开后得知的实际大小（读到末尾时以实际读到的为准）；没打开过时用清单里的大小</summary>
    private ulong? size;
    private bool reading, aborted;
    private int lastUse = Environment.TickCount;

    public RemoteFileStream(RemoteFileData owner, uint index, ClipOfferEntry entry)
    {
        this.owner = owner;
        this.index = index;
        this.entry = entry;
    }

    public bool Busy { get { lock (lk) return reading; } }

    public int Read(IntPtr pv, int cb, IntPtr pcbRead)
    {
        var done = 0;
        var hr = Hr.Ok;
        lock (lk)
        {
            // 同一个流的读不会并发；等对方数据时剪贴板线程照常派发 COM 调用，万一重入直接拒绝
            if (reading) hr = Hr.AccessDenied;
            else if (aborted) hr = Hr.Network;
            else reading = true;
        }
        if (hr != Hr.Ok)
        {
            if (pcbRead != IntPtr.Zero) Marshal.WriteInt32(pcbRead, 0);
            return hr;
        }
        try
        {
            // 读满再返回：有的调用方把读得比要的少当成到了末尾
            while (done < cb)
            {
                ICoreClipReader? r;
                ulong at;
                lock (lk)
                {
                    if (aborted) throw new OperationCanceledException();
                    if (size is { } s && pos >= s) break;
                    r = reader;
                    at = pos;
                }
                if (r == null)
                {
                    var opened = Blocking.Run(() => owner.Open(index, at));
                    var actual = opened.Size();
                    var stale = false;
                    lock (lk)
                    {
                        if (aborted || reader != null || pos != at) stale = true;
                        else
                        {
                            reader = opened;
                            size = actual;
                        }
                    }
                    if (stale)
                    {
                        Close(opened);
                        if (aborted) throw new OperationCanceledException();
                    }
                    continue;
                }
                var want = (uint)Math.Min(cb - done, 4 << 20);
                var chunk = Blocking.Run(() => r.Read(want));
                if (chunk.Length == 0)
                {
                    // 比打开时说的短（文件在对方那边变了）：以实际读到的为准
                    lock (lk) size = pos;
                    break;
                }
                Marshal.Copy(chunk, 0, pv + done, chunk.Length);
                done += chunk.Length;
                lock (lk)
                {
                    pos += (ulong)chunk.Length;
                    lastUse = Environment.TickCount;
                }
                owner.Touch();
            }
            // 读到末尾就关掉读取器（资源管理器读够大小后不一定再读一次）
            ICoreClipReader? end = null;
            lock (lk)
            {
                if (size is { } s && pos >= s)
                {
                    end = reader;
                    reader = null;
                }
            }
            if (end != null) Close(end);
        }
        catch (Exception e)
        {
            hr = Hr.FromError(e);
            Log.Info($"读取对方文件 {entry.Path} 失败：{e.Message}");
            ICoreClipReader? bad;
            lock (lk)
            {
                bad = reader;
                reader = null;
            }
            if (bad != null) Close(bad);
        }
        finally
        {
            lock (lk)
            {
                reading = false;
                lastUse = Environment.TickCount;
            }
            if (pcbRead != IntPtr.Zero) Marshal.WriteInt32(pcbRead, done);
        }
        return hr;
    }

    public int Write(IntPtr pv, int cb, IntPtr pcbWritten)
    {
        if (pcbWritten != IntPtr.Zero) Marshal.WriteInt32(pcbWritten, 0);
        return Hr.AccessDenied;
    }

    public int Seek(long move, int origin, IntPtr newPosition)
    {
        ICoreClipReader? old = null;
        long target;
        lock (lk)
        {
            if (reading) return Hr.AccessDenied;
            var end = (long)(size ?? entry.Size);
            target = origin switch
            {
                0 => move,
                1 => (long)pos + move,
                2 => end + move,
                _ => -1,
            };
            if (target < 0) return Hr.InvalidFunction;
            if ((ulong)target != pos)
            {
                // 换位置：下次读时从新位置重开
                old = reader;
                reader = null;
                pos = (ulong)target;
            }
            lastUse = Environment.TickCount;
        }
        if (old != null) Close(old);
        if (newPosition != IntPtr.Zero) Marshal.WriteInt64(newPosition, target);
        return Hr.Ok;
    }

    public int SetSize(long newSize) => Hr.InvalidFunction;

    public int CopyTo(IOleStream target, long cb, IntPtr pcbRead, IntPtr pcbWritten)
    {
        const int bufSize = 1 << 20;
        var buf = Marshal.AllocHGlobal(bufSize);
        var counts = Marshal.AllocHGlobal(8);
        long read = 0, written = 0;
        var hr = Hr.Ok;
        try
        {
            while (read < cb)
            {
                hr = Read(buf, (int)Math.Min(bufSize, cb - read), counts);
                if (hr < 0) break;
                var got = Marshal.ReadInt32(counts);
                if (got == 0) break;
                read += got;
                hr = target.Write(buf, got, counts + 4);
                if (hr < 0) break;
                written += Marshal.ReadInt32(counts + 4);
            }
        }
        finally
        {
            Marshal.FreeHGlobal(buf);
            Marshal.FreeHGlobal(counts);
        }
        if (pcbRead != IntPtr.Zero) Marshal.WriteInt64(pcbRead, read);
        if (pcbWritten != IntPtr.Zero) Marshal.WriteInt64(pcbWritten, written);
        return hr < 0 ? hr : Hr.Ok;
    }

    public int Commit(int flags) => Hr.Ok;
    public int Revert() => Hr.Ok;
    public int LockRegion(long offset, long cb, int lockType) => Hr.InvalidFunction;
    public int UnlockRegion(long offset, long cb, int lockType) => Hr.InvalidFunction;

    public int Stat(out STATSTG stat, int flags)
    {
        ulong len;
        lock (lk) len = size ?? entry.Size;
        var ft = RemoteFileData.FileTime(Math.Max(entry.Mtime, 0));
        var mtime = new FILETIME { dwLowDateTime = (int)ft, dwHighDateTime = (int)(ft >> 32) };
        stat = new STATSTG { type = 2 /* STGTY_STREAM */, cbSize = (long)len, mtime = entry.Mtime > 0 ? mtime : default, grfMode = 0 };
        return Hr.Ok;
    }

    public int Clone(out IOleStream? clone)
    {
        clone = null;
        return Hr.NotImpl;
    }

    int IOleSequentialStream.Read(IntPtr pv, int cb, IntPtr pcbRead) => Read(pv, cb, pcbRead);
    int IOleSequentialStream.Write(IntPtr pv, int cb, IntPtr pcbWritten) => Write(pv, cb, pcbWritten);

    /// <summary>闲了 ms 毫秒的读取器关掉（对方那边的读也就停了）；再读时从原位置重开</summary>
    public void CloseIfIdle(int ms)
    {
        ICoreClipReader? r = null;
        lock (lk)
        {
            if (!reading && reader != null && unchecked(Environment.TickCount - lastUse) > ms)
            {
                r = reader;
                reader = null;
            }
        }
        if (r != null) Close(r);
    }

    /// <summary>作废：之后读都报错；正在读的那次由内核在会话断开时报错返回</summary>
    public void Abort()
    {
        ICoreClipReader? r;
        lock (lk)
        {
            aborted = true;
            r = reader;
            reader = null;
        }
        if (r != null) Close(r);
    }

    private static void Close(ICoreClipReader r)
    {
        try { r.Close(); }
        catch (Exception e) { Log.Info("关闭剪贴板文件读取失败：" + e.Message); }
    }
}

[ComVisible(true)]
internal sealed class FormatEnumerator : IEnumFORMATETC
{
    private readonly FORMATETC[] formats;
    private int at;

    public FormatEnumerator(FORMATETC[] formats, int at)
    {
        this.formats = formats;
        this.at = at;
    }

    public int Next(int celt, FORMATETC[] rgelt, int[] pceltFetched)
    {
        var n = 0;
        while (n < celt && at < formats.Length && rgelt != null && n < rgelt.Length) rgelt[n++] = formats[at++];
        if (pceltFetched != null && pceltFetched.Length > 0) pceltFetched[0] = n;
        return n == celt ? Hr.Ok : Hr.False;
    }

    public int Skip(int celt)
    {
        at = Math.Min(formats.Length, at + Math.Max(celt, 0));
        return at < formats.Length ? Hr.Ok : Hr.False;
    }

    public int Reset()
    {
        at = 0;
        return Hr.Ok;
    }

    public void Clone(out IEnumFORMATETC newEnum) => newEnum = new FormatEnumerator(formats, at);
}

/// <summary>在线程池里跑会阻塞的调用；调用线程是 STA 时用 COM 模态等待（照常派发 COM 调用与跨线程发来的消息）</summary>
internal static class Blocking
{
    public static T Run<T>(Func<T> f)
    {
        if (System.Threading.Thread.CurrentThread.GetApartmentState() != ApartmentState.STA) return f();
        T result = default!;
        Exception? error = null;
        using var done = new ManualResetEvent(false);
        ThreadPool.QueueUserWorkItem(_ =>
        {
            try { result = f(); }
            catch (Exception e) { error = e; }
            finally { done.Set(); }
        });
        var handles = new[] { done.SafeWaitHandle.DangerousGetHandle() };
        if (NativeClip.CoWaitForMultipleHandles(0, uint.MaxValue, 1, handles, out _) != 0) done.WaitOne();
        if (error != null) ExceptionDispatchInfo.Capture(error).Throw();
        return result;
    }
}

/// <summary>COM 返回值</summary>
internal static class Hr
{
    public const int Ok = 0, False = 1;
    public const int SameFormatEtc = 0x00040130;
    public const int NotImpl = unchecked((int)0x80004001);
    public const int Abort = unchecked((int)0x80004004);
    public const int Unexpected = unchecked((int)0x8000FFFF);
    public const int OutOfMemory = unchecked((int)0x8007000E);
    public const int AccessDenied = unchecked((int)0x80030005);       // STG_E_ACCESSDENIED
    public const int InvalidFunction = unchecked((int)0x80030001);    // STG_E_INVALIDFUNCTION
    public const int Format = unchecked((int)0x80040064);             // DV_E_FORMATETC
    public const int Lindex = unchecked((int)0x80040068);             // DV_E_LINDEX
    public const int Tymed = unchecked((int)0x80040069);              // DV_E_TYMED
    public const int Aspect = unchecked((int)0x8004006B);             // DV_E_DVASPECT
    public const int AdviseNotSupported = unchecked((int)0x80040003); // OLE_E_ADVISENOTSUPPORTED
    /// <summary>HRESULT_FROM_WIN32(ERROR_UNEXP_NET_ERR)：资源管理器显示「发生意外的网络错误」</summary>
    public const int Network = unchecked((int)0x8007003B);
    public const int FileNotFound = unchecked((int)0x80070002);
    public const int Denied = unchecked((int)0x80070005);

    /// <summary>会话断了、标签关了（我们自己作废）报网络错误；对方取消报中止</summary>
    public static int FromError(Exception e) => e switch
    {
        OperationCanceledException or ObjectDisposedException => Network,
        GalaxyException.Cancelled => Abort,
        GalaxyException.NotFound => FileNotFound,
        GalaxyException.Permission => Denied,
        _ => Network,
    };

    /// <summary>拷到新分配的 HGLOBAL（调用方用 ReleaseStgMedium 释放）</summary>
    public static int Global(byte[] data, out STGMEDIUM medium)
    {
        medium = default;
        var h = NativeClip.GlobalAlloc(NativeClip.GmemMoveable, (UIntPtr)(uint)Math.Max(data.Length, 1));
        if (h == IntPtr.Zero) return OutOfMemory;
        var p = NativeClip.GlobalLock(h);
        if (p == IntPtr.Zero)
        {
            NativeClip.GlobalFree(h);
            return OutOfMemory;
        }
        Marshal.Copy(data, 0, p, data.Length);
        NativeClip.GlobalUnlock(h);
        medium.tymed = TYMED.TYMED_HGLOBAL;
        medium.unionmember = h;
        return Ok;
    }
}

// ---------- COM 接口（全部 PreserveSig，自己返回 HRESULT，不靠抛异常） ----------

[ComImport, Guid("0000010e-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IOleDataObject
{
    [PreserveSig] int GetData([In] ref FORMATETC format, out STGMEDIUM medium);
    [PreserveSig] int GetDataHere([In] ref FORMATETC format, ref STGMEDIUM medium);
    [PreserveSig] int QueryGetData([In] ref FORMATETC format);
    [PreserveSig] int GetCanonicalFormatEtc([In] ref FORMATETC formatIn, out FORMATETC formatOut);
    [PreserveSig] int SetData([In] ref FORMATETC formatIn, [In] ref STGMEDIUM medium, [MarshalAs(UnmanagedType.Bool)] bool release);
    [PreserveSig] int EnumFormatEtc(DATADIR direction, out IEnumFORMATETC? enumerator);
    [PreserveSig] int DAdvise([In] ref FORMATETC format, ADVF advf, IAdviseSink sink, out int connection);
    [PreserveSig] int DUnadvise(int connection);
    [PreserveSig] int EnumDAdvise(out IEnumSTATDATA? enumerator);
}

/// <summary>即 IAsyncOperation：资源管理器据此可以在后台线程里粘贴</summary>
[ComImport, Guid("3D8B0590-F691-11d2-8EA9-006097DF5BD4"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IDataObjectAsyncCapability
{
    [PreserveSig] int SetAsyncMode([MarshalAs(UnmanagedType.Bool)] bool doOpAsync);
    [PreserveSig] int GetAsyncMode([MarshalAs(UnmanagedType.Bool)] out bool isOpAsync);
    [PreserveSig] int StartOperation(IntPtr bindCtx);
    [PreserveSig] int InOperation([MarshalAs(UnmanagedType.Bool)] out bool inAsync);
    [PreserveSig] int EndOperation(int result, IntPtr bindCtx, uint effects);
}

[ComImport, Guid("0000000c-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IOleStream
{
    [PreserveSig] int Read(IntPtr pv, int cb, IntPtr pcbRead);
    [PreserveSig] int Write(IntPtr pv, int cb, IntPtr pcbWritten);
    [PreserveSig] int Seek(long move, int origin, IntPtr newPosition);
    [PreserveSig] int SetSize(long newSize);
    [PreserveSig] int CopyTo(IOleStream target, long cb, IntPtr pcbRead, IntPtr pcbWritten);
    [PreserveSig] int Commit(int flags);
    [PreserveSig] int Revert();
    [PreserveSig] int LockRegion(long offset, long cb, int lockType);
    [PreserveSig] int UnlockRegion(long offset, long cb, int lockType);
    [PreserveSig] int Stat(out STATSTG stat, int flags);
    [PreserveSig] int Clone(out IOleStream? clone);
}

[ComImport, Guid("0c733a30-2a1c-11ce-ade5-00aa0044773d"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IOleSequentialStream
{
    [PreserveSig] int Read(IntPtr pv, int cb, IntPtr pcbRead);
    [PreserveSig] int Write(IntPtr pv, int cb, IntPtr pcbWritten);
}

internal static class NativeClip
{
    public const uint GmemMoveable = 0x2;

    public static short Format(string name) => unchecked((short)(ushort)RegisterClipboardFormat(name));

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern uint RegisterClipboardFormat(string name);
    [DllImport("user32.dll")]
    public static extern IntPtr GetClipboardOwner();
    [DllImport("user32.dll")]
    public static extern int GetWindowThreadProcessId(IntPtr hwnd, out int processId);
    [DllImport("kernel32.dll")]
    public static extern int GetCurrentThreadId();
    [DllImport("ole32.dll")]
    public static extern int OleInitialize(IntPtr reserved);
    [DllImport("ole32.dll")]
    public static extern int OleSetClipboard(IOleDataObject? data);
    [DllImport("ole32.dll")]
    public static extern int OleIsCurrentClipboard(IOleDataObject data);
    [DllImport("ole32.dll")]
    public static extern void ReleaseStgMedium(ref STGMEDIUM medium);
    [DllImport("ole32.dll")]
    public static extern int CoWaitForMultipleHandles(uint flags, uint timeout, uint count, IntPtr[] handles, out uint index);
    [DllImport("kernel32.dll")]
    public static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
    [DllImport("kernel32.dll")]
    public static extern IntPtr GlobalLock(IntPtr h);
    [DllImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GlobalUnlock(IntPtr h);
    [DllImport("kernel32.dll")]
    public static extern UIntPtr GlobalSize(IntPtr h);
    [DllImport("kernel32.dll")]
    public static extern IntPtr GlobalFree(IntPtr h);
}
