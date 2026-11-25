// 老系统兼容版：界面另按 .NET Framework 4.0 编一份（XP SP3 / Vista / Windows 7 / 8 / Server 2003–2012 R2 都能装 4.0，
// 4.5 起不支持 XP，4.6 起不支持 Vista，4.8 要 Windows 7 SP1），见 docs/spec.md 3.0.2。
// 4.0 的类库没有 async/await 要的类型、值元组和 WindowChrome，下面只在 NET40 时补上最小实现；
// 跑在装了 4.5 起的系统上时也用本程序集里的这些（与系统自带的同名类型不冲突）。
// Bg 是两份都用的小工具：Task.Run / Task.FromResult 是 4.5 起才有的。

using System.Runtime.CompilerServices;

namespace LemonGalaxy
{
    internal static class Bg
    {
        /// <summary>在线程池上跑（等同 Task.Run）</summary>
        public static Task<T> Run<T>(Func<T> f) =>
            Task.Factory.StartNew(f, CancellationToken.None, TaskCreationOptions.None, TaskScheduler.Default);

        public static Task Run(Action f) =>
            Task.Factory.StartNew(f, CancellationToken.None, TaskCreationOptions.None, TaskScheduler.Default);

        /// <summary>已经完成的任务（等同 Task.FromResult）</summary>
        public static Task<T> Done<T>(T value)
        {
            var t = new TaskCompletionSource<T>();
            t.SetResult(value);
            return t.Task;
        }

        /// <summary>
        /// 由后台线程完成的任务：完成时等它的代码不在完成它的那个线程上接着跑。
        /// 4.6 起用 RunContinuationsAsynchronously；4.0 版本的 await 本来就回到界面线程或线程池（见下面的 TaskAwaiter）。
        /// </summary>
        public static TaskCompletionSource<T> Tcs<T>() =>
#if NET40
            new TaskCompletionSource<T>();
#else
            new TaskCompletionSource<T>(TaskCreationOptions.RunContinuationsAsynchronously);
#endif
    }
}

#if NET40
namespace System.Runtime.CompilerServices
{
    public interface IAsyncStateMachine
    {
        void MoveNext();
        void SetStateMachine(IAsyncStateMachine stateMachine);
    }

    public interface INotifyCompletion
    {
        void OnCompleted(Action continuation);
    }

    public interface ICriticalNotifyCompletion : INotifyCompletion
    {
        void UnsafeOnCompleted(Action continuation);
    }

    [AttributeUsage(AttributeTargets.Method, Inherited = false, AllowMultiple = false)]
    public class StateMachineAttribute : Attribute
    {
        public StateMachineAttribute(Type stateMachineType) { StateMachineType = stateMachineType; }
        public Type StateMachineType { get; }
    }

    [AttributeUsage(AttributeTargets.Method, Inherited = false, AllowMultiple = false)]
    public sealed class AsyncStateMachineAttribute : StateMachineAttribute
    {
        public AsyncStateMachineAttribute(Type stateMachineType) : base(stateMachineType) { }
    }

    [AttributeUsage(AttributeTargets.Field | AttributeTargets.Parameter | AttributeTargets.Property | AttributeTargets.ReturnValue | AttributeTargets.Class | AttributeTargets.Struct | AttributeTargets.Event)]
    public sealed class TupleElementNamesAttribute : Attribute
    {
        public TupleElementNamesAttribute(string?[] transformNames) { TransformNames = transformNames; }
        public IList<string?> TransformNames { get; }
    }

    /// <summary>
    /// 状态机第一次要等时装箱一次，之后每次接着跑都在这一份上；生成器的结构体副本共用这个对象，所以都看得到同一份。
    /// </summary>
    internal class AsyncBox
    {
        private Action? next;

        public Action Next<TStateMachine>(ref TStateMachine stateMachine) where TStateMachine : IAsyncStateMachine
        {
            if (next == null)
            {
                IAsyncStateMachine box = stateMachine;
                next = box.MoveNext;
            }
            return next;
        }
    }

    public struct AsyncVoidMethodBuilder
    {
        private AsyncBox box;
        private SynchronizationContext? context;

        public static AsyncVoidMethodBuilder Create() => new AsyncVoidMethodBuilder { box = new AsyncBox(), context = SynchronizationContext.Current };
        public void Start<TStateMachine>(ref TStateMachine stateMachine) where TStateMachine : IAsyncStateMachine => stateMachine.MoveNext();
        public void SetStateMachine(IAsyncStateMachine stateMachine) { }
        public void AwaitOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
            where TAwaiter : INotifyCompletion where TStateMachine : IAsyncStateMachine => awaiter.OnCompleted(box.Next(ref stateMachine));
        public void AwaitUnsafeOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
            where TAwaiter : ICriticalNotifyCompletion where TStateMachine : IAsyncStateMachine => awaiter.UnsafeOnCompleted(box.Next(ref stateMachine));
        public void SetResult() { }

        /// <summary>与 4.5 一样：async void 里没接住的异常抛回原来的线程（界面线程上就是界面的未处理异常）</summary>
        public void SetException(Exception exception)
        {
            var e = new System.Reflection.TargetInvocationException(exception.Message, exception);
            if (context != null) context.Post(_ => throw e, null);
            else ThreadPool.QueueUserWorkItem(_ => throw e);
        }
    }

    public struct AsyncTaskMethodBuilder
    {
        private AsyncTaskMethodBuilder<object?> inner;

        public static AsyncTaskMethodBuilder Create() => new AsyncTaskMethodBuilder { inner = AsyncTaskMethodBuilder<object?>.Create() };
        public Task Task => inner.Task;
        public void Start<TStateMachine>(ref TStateMachine stateMachine) where TStateMachine : IAsyncStateMachine => stateMachine.MoveNext();
        public void SetStateMachine(IAsyncStateMachine stateMachine) { }
        public void AwaitOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
            where TAwaiter : INotifyCompletion where TStateMachine : IAsyncStateMachine => inner.AwaitOnCompleted(ref awaiter, ref stateMachine);
        public void AwaitUnsafeOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
            where TAwaiter : ICriticalNotifyCompletion where TStateMachine : IAsyncStateMachine => inner.AwaitUnsafeOnCompleted(ref awaiter, ref stateMachine);
        public void SetResult() => inner.SetResult(null);
        public void SetException(Exception exception) => inner.SetException(exception);
    }

    public struct AsyncTaskMethodBuilder<TResult>
    {
        private sealed class Core : AsyncBox
        {
            public readonly TaskCompletionSource<TResult> Tcs = new TaskCompletionSource<TResult>();
        }

        private Core core;

        public static AsyncTaskMethodBuilder<TResult> Create() => new AsyncTaskMethodBuilder<TResult> { core = new Core() };
        public Task<TResult> Task => core.Tcs.Task;
        public void Start<TStateMachine>(ref TStateMachine stateMachine) where TStateMachine : IAsyncStateMachine => stateMachine.MoveNext();
        public void SetStateMachine(IAsyncStateMachine stateMachine) { }
        public void AwaitOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
            where TAwaiter : INotifyCompletion where TStateMachine : IAsyncStateMachine => awaiter.OnCompleted(core.Next(ref stateMachine));
        public void AwaitUnsafeOnCompleted<TAwaiter, TStateMachine>(ref TAwaiter awaiter, ref TStateMachine stateMachine)
            where TAwaiter : ICriticalNotifyCompletion where TStateMachine : IAsyncStateMachine => awaiter.UnsafeOnCompleted(core.Next(ref stateMachine));
        public void SetResult(TResult result) => core.Tcs.TrySetResult(result);

        public void SetException(Exception exception)
        {
            if (exception is OperationCanceledException) core.Tcs.TrySetCanceled();
            else core.Tcs.TrySetException(exception);
        }
    }

    public readonly struct TaskAwaiter : ICriticalNotifyCompletion
    {
        private readonly Task task;
        internal TaskAwaiter(Task task) { this.task = task; }
        public bool IsCompleted => task.IsCompleted;
        public void OnCompleted(Action continuation) => AwaitSupport.Continue(task, continuation);
        public void UnsafeOnCompleted(Action continuation) => AwaitSupport.Continue(task, continuation);
        public void GetResult() => AwaitSupport.Check(task);
    }

    public readonly struct TaskAwaiter<TResult> : ICriticalNotifyCompletion
    {
        private readonly Task<TResult> task;
        internal TaskAwaiter(Task<TResult> task) { this.task = task; }
        public bool IsCompleted => task.IsCompleted;
        public void OnCompleted(Action continuation) => AwaitSupport.Continue(task, continuation);
        public void UnsafeOnCompleted(Action continuation) => AwaitSupport.Continue(task, continuation);

        public TResult GetResult()
        {
            AwaitSupport.Check(task);
            return task.Result;
        }
    }

    internal static class AwaitSupport
    {
        /// <summary>与 4.5 的 await 一样：有界面线程的同步上下文就回到界面线程接着跑，否则在线程池（或当前调度器）上</summary>
        public static void Continue(Task task, Action continuation)
        {
            var context = SynchronizationContext.Current;
            if (context != null && context.GetType() != typeof(SynchronizationContext))
            {
                task.ContinueWith(_ => context.Post(s => ((Action)s!)(), continuation),
                    CancellationToken.None, TaskContinuationOptions.ExecuteSynchronously, TaskScheduler.Default);
                return;
            }
            task.ContinueWith(_ => continuation(), CancellationToken.None, TaskContinuationOptions.ExecuteSynchronously, TaskScheduler.Current);
        }

        /// <summary>任务失败时抛出里面的那个异常（与 4.5 一样，不是外面包的 AggregateException）</summary>
        public static void Check(Task task)
        {
            if (task.IsCanceled) throw new TaskCanceledException(task);
            if (task.IsFaulted)
            {
                var all = task.Exception!;
                if (all.InnerExceptions.Count == 1) throw Rethrow(all.InnerExceptions[0]);
                throw all;
            }
        }

        /// <summary>原样再抛前留住原来的调用栈（4.0 没有 ExceptionDispatchInfo，用类库内部的 PrepForRemoting，找不到就算了）</summary>
        private static Exception Rethrow(Exception e)
        {
            try { PrepForRemoting?.Invoke(e); } catch { }
            return e;
        }

        private static readonly Func<Exception, Exception>? PrepForRemoting = CreatePrep();

        private static Func<Exception, Exception>? CreatePrep()
        {
            var m = typeof(Exception).GetMethod("PrepForRemoting", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);
            return m == null ? null : Delegate.CreateDelegate(typeof(Func<Exception, Exception>), m, false) as Func<Exception, Exception>;
        }
    }
}

namespace System.Threading.Tasks
{
    /// <summary>4.0 的 Task 没有 GetAwaiter，await 靠这两个扩展方法</summary>
    public static class Net40TaskAwait
    {
        public static System.Runtime.CompilerServices.TaskAwaiter GetAwaiter(this Task task) => new(task);
        public static System.Runtime.CompilerServices.TaskAwaiter<T> GetAwaiter<T>(this Task<T> task) => new(task);
    }
}

namespace System
{
    public struct ValueTuple<T1, T2> : IEquatable<ValueTuple<T1, T2>>
    {
        public T1 Item1;
        public T2 Item2;

        public ValueTuple(T1 item1, T2 item2)
        {
            Item1 = item1;
            Item2 = item2;
        }

        public bool Equals(ValueTuple<T1, T2> other) =>
            EqualityComparer<T1>.Default.Equals(Item1, other.Item1) && EqualityComparer<T2>.Default.Equals(Item2, other.Item2);

        public override bool Equals(object? obj) => obj is ValueTuple<T1, T2> o && Equals(o);
        public override int GetHashCode() => (EqualityComparer<T1>.Default.GetHashCode(Item1!) * 397) ^ EqualityComparer<T2>.Default.GetHashCode(Item2!);
        public override string ToString() => $"({Item1}, {Item2})";
    }

    public struct ValueTuple<T1, T2, T3> : IEquatable<ValueTuple<T1, T2, T3>>
    {
        public T1 Item1;
        public T2 Item2;
        public T3 Item3;

        public ValueTuple(T1 item1, T2 item2, T3 item3)
        {
            Item1 = item1;
            Item2 = item2;
            Item3 = item3;
        }

        public bool Equals(ValueTuple<T1, T2, T3> other) =>
            EqualityComparer<T1>.Default.Equals(Item1, other.Item1) && EqualityComparer<T2>.Default.Equals(Item2, other.Item2)
            && EqualityComparer<T3>.Default.Equals(Item3, other.Item3);

        public override bool Equals(object? obj) => obj is ValueTuple<T1, T2, T3> o && Equals(o);

        public override int GetHashCode() =>
            (((EqualityComparer<T1>.Default.GetHashCode(Item1!) * 397) ^ EqualityComparer<T2>.Default.GetHashCode(Item2!)) * 397)
            ^ EqualityComparer<T3>.Default.GetHashCode(Item3!);

        public override string ToString() => $"({Item1}, {Item2}, {Item3})";
    }

    public struct ValueTuple<T1, T2, T3, T4> : IEquatable<ValueTuple<T1, T2, T3, T4>>
    {
        public T1 Item1;
        public T2 Item2;
        public T3 Item3;
        public T4 Item4;

        public ValueTuple(T1 item1, T2 item2, T3 item3, T4 item4)
        {
            Item1 = item1;
            Item2 = item2;
            Item3 = item3;
            Item4 = item4;
        }

        public bool Equals(ValueTuple<T1, T2, T3, T4> other) =>
            EqualityComparer<T1>.Default.Equals(Item1, other.Item1) && EqualityComparer<T2>.Default.Equals(Item2, other.Item2)
            && EqualityComparer<T3>.Default.Equals(Item3, other.Item3) && EqualityComparer<T4>.Default.Equals(Item4, other.Item4);

        public override bool Equals(object? obj) => obj is ValueTuple<T1, T2, T3, T4> o && Equals(o);

        public override int GetHashCode() =>
            (((((EqualityComparer<T1>.Default.GetHashCode(Item1!) * 397) ^ EqualityComparer<T2>.Default.GetHashCode(Item2!)) * 397)
              ^ EqualityComparer<T3>.Default.GetHashCode(Item3!)) * 397) ^ EqualityComparer<T4>.Default.GetHashCode(Item4!);

        public override string ToString() => $"({Item1}, {Item2}, {Item3}, {Item4})";
    }
}

namespace System.Windows.Shell
{
    /// <summary>共用控件里的按钮会登记「自绘标题栏里也能点」；被控端界面用系统标题栏，4.0 版本什么都不做</summary>
    internal static class WindowChrome
    {
        public static void SetIsHitTestVisibleInChrome(System.Windows.IInputElement element, bool value) { }
    }
}
#endif
