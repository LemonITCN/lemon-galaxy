using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace LemonGalaxy.UI;

/// <summary>自适应网格：按最小宽度算出列数，每列等宽（卡片网格用）</summary>
internal sealed class AdaptiveGrid : Panel
{
    public double MinItem = 220;
    public double Spacing = 12;

    private int Cols(double width) => double.IsInfinity(width) ? Math.Max(1, InternalChildren.Count) : Math.Max(1, (int)((width + Spacing) / (MinItem + Spacing)));

    protected override Size MeasureOverride(Size available)
    {
        var w = double.IsInfinity(available.Width) ? MinItem * Math.Max(1, InternalChildren.Count) : available.Width;
        var cols = Cols(available.Width);
        var itemW = Math.Max(0, (w - Spacing * (cols - 1)) / cols);
        double total = 0, rowH = 0;
        for (var i = 0; i < InternalChildren.Count; i++)
        {
            var c = InternalChildren[i];
            c.Measure(new Size(itemW, double.PositiveInfinity));
            rowH = Math.Max(rowH, c.DesiredSize.Height);
            if (i % cols == cols - 1 || i == InternalChildren.Count - 1)
            {
                total += rowH + (total > 0 || i >= cols ? Spacing : 0);
                rowH = 0;
            }
        }
        return new Size(w, total);
    }

    protected override Size ArrangeOverride(Size final)
    {
        var cols = Cols(final.Width);
        var itemW = Math.Max(0, (final.Width - Spacing * (cols - 1)) / cols);
        double y = 0;
        for (var start = 0; start < InternalChildren.Count; start += cols)
        {
            double rowH = 0;
            for (var i = start; i < Math.Min(start + cols, InternalChildren.Count); i++) rowH = Math.Max(rowH, InternalChildren[i].DesiredSize.Height);
            for (var i = start; i < Math.Min(start + cols, InternalChildren.Count); i++)
                InternalChildren[i].Arrange(new Rect((i - start) * (itemW + Spacing), y, itemW, rowH));
            y += rowH + Spacing;
        }
        return final;
    }
}

/// <summary>自动换行排列（标签、目标选择）</summary>
internal sealed class FlowPanel : Panel
{
    public double Spacing = 6;

    protected override Size MeasureOverride(Size available)
    {
        double x = 0, y = 0, rowH = 0, maxW = 0;
        foreach (UIElement c in InternalChildren)
        {
            c.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
            var s = c.DesiredSize;
            if (x > 0 && x + s.Width > available.Width) { x = 0; y += rowH + Spacing; rowH = 0; }
            x += s.Width + Spacing;
            rowH = Math.Max(rowH, s.Height);
            maxW = Math.Max(maxW, x);
        }
        return new Size(double.IsInfinity(available.Width) ? maxW : available.Width, y + rowH);
    }

    protected override Size ArrangeOverride(Size final)
    {
        double x = 0, y = 0, rowH = 0;
        foreach (UIElement c in InternalChildren)
        {
            var s = c.DesiredSize;
            if (x > 0 && x + s.Width > final.Width) { x = 0; y += rowH + Spacing; rowH = 0; }
            c.Arrange(new Rect(x, y, s.Width, s.Height));
            x += s.Width + Spacing;
            rowH = Math.Max(rowH, s.Height);
        }
        return final;
    }
}

/// <summary>列表行拖放（主机拖进文件夹）</summary>
internal static class Dnd
{
    public const string HostFormat = "LemonGalaxy.Host";

    public static void Source(Btn e, string format, Func<string> data)
    {
        Point? start = null;
        e.PreviewMouseLeftButtonDown += (_, a) => start = a.GetPosition(e);
        e.PreviewMouseLeftButtonUp += (_, _) => start = null;
        e.PreviewMouseMove += (_, a) =>
        {
            if (start == null || a.LeftButton != MouseButtonState.Pressed) return;
            var p = a.GetPosition(e);
            if (Math.Abs(p.X - start.Value.X) < 5 && Math.Abs(p.Y - start.Value.Y) < 5) return;
            start = null;
            e.CancelPress();
            try { DragDrop.DoDragDrop(e, new DataObject(format, data()), DragDropEffects.Move); }
            catch (Exception ex) { Log.Error("拖放出错：" + ex.Message); }
        };
    }

    public static void Target(FrameworkElement e, string format, Action<string> drop, Action<bool>? hover = null)
    {
        e.AllowDrop = true;
        e.DragEnter += (_, a) => { if (a.Data.GetDataPresent(format)) { a.Effects = DragDropEffects.Move; hover?.Invoke(true); } else a.Effects = DragDropEffects.None; a.Handled = true; };
        e.DragOver += (_, a) => { a.Effects = a.Data.GetDataPresent(format) ? DragDropEffects.Move : DragDropEffects.None; a.Handled = true; };
        e.DragLeave += (_, _) => hover?.Invoke(false);
        e.Drop += (_, a) =>
        {
            hover?.Invoke(false);
            if (a.Data.GetData(format) is string s) { a.Handled = true; drop(s); }
        };
    }
}
