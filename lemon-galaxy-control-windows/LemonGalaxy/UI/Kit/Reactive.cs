using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Threading;

namespace LemonGalaxy.UI;

/// <summary>
/// 可观察的界面状态。界面区域用 Watch(owner, action) 订阅：只弱引用 owner，
/// owner 被丢弃（界面重建）后订阅自动失效，不需要手动退订。Notify 必须在界面线程调用。
/// </summary>
internal class Observable
{
    private static readonly ConditionalWeakTable<object, Dictionary<Observable, List<Action>>> Handlers = new();
    private readonly List<WeakReference> watchers = new();

    public void Watch(object owner, Action onChange)
    {
        var map = Handlers.GetOrCreateValue(owner);
        if (!map.TryGetValue(this, out var list))
        {
            list = new List<Action>();
            map[this] = list;
            watchers.Add(new WeakReference(owner));
        }
        if (!list.Contains(onChange)) list.Add(onChange);
    }

    public void Notify()
    {
        if (watchers.Count == 0) return;
        var alive = new List<object>(watchers.Count);
        watchers.RemoveAll(w =>
        {
            var o = w.Target;
            if (o == null) return true;
            alive.Add(o);
            return false;
        });
        foreach (var o in alive)
        {
            if (!Handlers.TryGetValue(o, out var map) || !map.TryGetValue(this, out var list)) continue;
            foreach (var a in list.ToArray())
            {
                try { a(); }
                catch (Exception e) { Log.Error("界面刷新出错：" + e); }
            }
        }
    }
}

/// <summary>界面线程工具：投递、合并刷新、异步任务出错提示、selftest 抽空处理消息。</summary>
internal static class Ui
{
    public static Dispatcher Dispatcher = Dispatcher.CurrentDispatcher;
    private static readonly HashSet<object> Pending = new();

    public static void Init()
    {
        Dispatcher = Dispatcher.CurrentDispatcher;
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(Dispatcher));
    }

    public static bool OnUiThread => Dispatcher.CheckAccess();

    /// <summary>从任意线程投递到界面线程</summary>
    public static void Post(Action a)
    {
        Dispatcher.BeginInvoke(DispatcherPriority.Normal, new Action(() =>
        {
            try { a(); }
            catch (Exception e) { Log.Error("界面回调出错：" + e); }
        }));
    }

    /// <summary>同一 key 在下一次处理前只执行一次（连续的状态变化合并成一次重绘）</summary>
    public static void Coalesce(object key, Action a, DispatcherPriority priority = DispatcherPriority.Background)
    {
        lock (Pending)
        {
            if (!Pending.Add(key)) return;
        }
        Dispatcher.BeginInvoke(priority, new Action(() =>
        {
            lock (Pending) Pending.Remove(key);
            try { a(); }
            catch (Exception e) { Log.Error("界面刷新出错：" + e); }
        }));
    }

    /// <summary>延时执行（界面线程）</summary>
    public static void After(int ms, Action a)
    {
        var t = new DispatcherTimer(DispatcherPriority.Normal, Dispatcher) { Interval = TimeSpan.FromMilliseconds(ms) };
        t.Tick += (_, _) =>
        {
            t.Stop();
            try { a(); }
            catch (Exception e) { Log.Error("定时回调出错：" + e); }
        };
        t.Start();
    }

    public static DispatcherTimer Every(int ms, Action a)
    {
        var t = new DispatcherTimer(DispatcherPriority.Background, Dispatcher) { Interval = TimeSpan.FromMilliseconds(ms) };
        t.Tick += (_, _) =>
        {
            try { a(); }
            catch (Exception e) { Log.Error("定时回调出错：" + e); }
        };
        t.Start();
        return t;
    }

    /// <summary>界面线程上跑一个异步动作；未处理的异常以错误提示条显示。</summary>
    public static async void Run(Func<Task> f)
    {
        try { await f(); }
        catch (Exception e) { AppModel.Current?.ShowError(e); }
    }

    /// <summary>处理完当前排队的消息（selftest 用）</summary>
    public static void DoEvents()
    {
        var frame = new DispatcherFrame();
        Dispatcher.BeginInvoke(DispatcherPriority.ContextIdle, new DispatcherOperationCallback(f =>
        {
            ((DispatcherFrame)f).Continue = false;
            return null;
        }), frame);
        Dispatcher.PushFrame(frame);
    }

    public static void Pump(int ms)
    {
        var end = DateTime.UtcNow.AddMilliseconds(ms);
        do
        {
            DoEvents();
            Thread.Sleep(5);
        } while (DateTime.UtcNow < end);
        DoEvents();
    }

    public static bool WaitFor(Func<bool> cond, int timeoutMs)
    {
        var end = DateTime.UtcNow.AddMilliseconds(timeoutMs);
        while (DateTime.UtcNow < end)
        {
            DoEvents();
            if (cond()) return true;
            Thread.Sleep(10);
        }
        DoEvents();
        return cond();
    }
}

/// <summary>按需重建内容的区域：Build 返回当前状态对应的子元素，Invalidate 合并到下一次消息处理时重建。</summary>
internal sealed class Region : Border
{
    private readonly Func<UIElement?> build;

    public Region(Func<UIElement?> build, bool immediate = true)
    {
        this.build = build;
        if (immediate) Refresh();
    }

    public void Refresh()
    {
        UIElement? c;
        try { c = build(); }
        catch (Exception e)
        {
            Log.Error("区域构建出错：" + e);
            c = new TextBlock { Text = e.Message, Foreground = System.Windows.Media.Brushes.Red, TextWrapping = TextWrapping.Wrap };
        }
        Detach(c);
        Child = c;
        if (keyFn != null) lastKey = keyFn();
    }

    /// <summary>正常运行时为 true：不在任何窗口里的区域（界面重建后留下的旧区域）不再重建。selftest 离屏渲染时为 false。</summary>
    public static bool RequireAttached;

    public void Invalidate() => Ui.Coalesce(this, () =>
    {
        if (RequireAttached && PresentationSource.FromVisual(this) == null) return;
        if (keyFn != null)
        {
            var k = keyFn();
            if (Equals(k, lastKey)) return;
        }
        Refresh();
    });

    private Func<object?>? keyFn;
    private object? lastKey;

    /// <summary>只在 key 变化时重建（里面有输入框等需要保持焦点的元素时用）</summary>
    public Region Keyed(Func<object?> key)
    {
        keyFn = key;
        lastKey = key();
        return this;
    }

    /// <summary>不管 key 是否变化都重建</summary>
    public void Force()
    {
        if (keyFn != null) lastKey = keyFn();
        Refresh();
    }

    /// <summary>常驻控件（终端、画面）在重建时会被挂到新的父元素，先从旧父元素上摘下来</summary>
    public static void Detach(UIElement? e)
    {
        if (e is not FrameworkElement fe || fe.Parent == null) return;
        switch (fe.Parent)
        {
            case Panel p: p.Children.Remove(fe); break;
            case Decorator d: d.Child = null; break;
            case ContentControl cc: cc.Content = null; break;
            case ContentPresenter cp: cp.Content = null; break;
        }
    }

    /// <summary>订阅一个或多个状态，任何一个变化都重建</summary>
    public Region On(params Observable?[] sources)
    {
        foreach (var s in sources) s?.Watch(this, Invalidate);
        return this;
    }
}
