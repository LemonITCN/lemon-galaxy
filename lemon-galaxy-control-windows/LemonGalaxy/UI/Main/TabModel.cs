using System.Windows;

namespace LemonGalaxy.UI;

/// <summary>工作区标签（终端、文件、远程桌面、AI 对话）的基类。视图按需创建、换主题时重建；终端、画面等常驻控件放在模型里，重建时挂到新视图上。</summary>
internal abstract class TabModel : Observable
{
    public readonly string Id = Guid.NewGuid().ToString("N");
    public readonly TabKind Kind;
    protected readonly AppModel App;
    private string title;
    private bool? live;
    public string? DeviceId;
    public string? SshHostId;
    private FrameworkElement? view;

    protected TabModel(AppModel app, TabKind kind, string title)
    {
        App = app;
        Kind = kind;
        this.title = title;
    }

    public string Title
    {
        get => title;
        set { if (title == value) return; title = value; Notify(); }
    }

    /// <summary>标签上的状态点：null 不显示，true 正常，false 断开</summary>
    public bool? Live
    {
        get => live;
        set { if (live == value) return; live = value; Notify(); }
    }

    public string Icon => Kind switch
    {
        TabKind.Terminal => "terminal",
        TabKind.Sftp => "folder",
        _ => "display",
    };

    public FrameworkElement View => view ??= CreateView();
    public bool HasView => view != null;
    public void ResetView() => view = null;
    protected abstract FrameworkElement CreateView();

    public virtual void Close() { }

    /// <summary>标签被切到前台</summary>
    public virtual void Activated() { }
}
