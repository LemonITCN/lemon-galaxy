using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>
/// 虚拟化列表：系统 ListBox（选择、键盘、滚动、虚拟化），每一行由 RowBuilder 用代码构建（不用 XAML 模板）。
/// </summary>
internal sealed class VList : ListBox
{
    public VList(Func<object, UIElement> rowBuilder, SelectionMode mode = SelectionMode.Extended)
    {
        SelectionMode = mode;
        ItemTemplate = new DataTemplate { VisualTree = new FrameworkElementFactory(typeof(RowHost)) };
        SetValue(RowHost.BuilderProperty, rowBuilder);
        VirtualizingPanel.SetIsVirtualizing(this, true);
        VirtualizingPanel.SetVirtualizationMode(this, VirtualizationMode.Recycling);
        ScrollViewer.SetHorizontalScrollBarVisibility(this, ScrollBarVisibility.Disabled);
        Background = Brushes.Transparent;
        BorderThickness = new Thickness(0);
    }

    /// <summary>事件源所在行对应的数据</summary>
    public static object? ItemAt(DependencyObject? source)
    {
        while (source != null && source is not ListBoxItem)
            source = source is Visual or System.Windows.Media.Media3D.Visual3D ? VisualTreeHelper.GetParent(source) : LogicalTreeHelper.GetParent(source);
        return (source as ListBoxItem)?.DataContext;
    }
}

/// <summary>列表行宿主：数据变化时用继承下来的 Builder 重建内容</summary>
internal sealed class RowHost : Border
{
    public static readonly DependencyProperty BuilderProperty = DependencyProperty.RegisterAttached(
        "Builder", typeof(Func<object, UIElement>), typeof(RowHost),
        new FrameworkPropertyMetadata(null, FrameworkPropertyMetadataOptions.Inherits, (d, _) => (d as RowHost)?.Rebuild()));

    protected override void OnPropertyChanged(DependencyPropertyChangedEventArgs e)
    {
        base.OnPropertyChanged(e);
        if (e.Property == DataContextProperty) Rebuild();
    }

    private void Rebuild()
    {
        var b = GetValue(BuilderProperty) as Func<object, UIElement>;
        var data = DataContext;
        Child = b != null && data != null ? b(data) : null;
    }
}

internal static class InputExt
{
    public static bool Ctrl => Keyboard.Modifiers.HasFlag(ModifierKeys.Control);
}
