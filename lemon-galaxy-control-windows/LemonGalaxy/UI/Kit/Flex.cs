using System.Windows;
using System.Windows.Controls;

namespace LemonGalaxy.UI;

/// <summary>
/// 一行（Row）或一列（Col）排布的面板，带间距；标了 Grow 的子元素分走剩余空间（相当于 SwiftUI 的 HStack / VStack + Spacer）。
/// Row 里子元素默认竖直居中（子元素 VerticalAlignment 不是 Stretch 时按它自己的）；Col 里子元素默认横向撑满（按子元素自己的 HorizontalAlignment）。
/// </summary>
internal sealed class Flex : Panel
{
    public bool Horizontal;
    public double Spacing;

    public static readonly DependencyProperty GrowProperty = DependencyProperty.RegisterAttached(
        "Grow", typeof(double), typeof(Flex), new FrameworkPropertyMetadata(0.0, FrameworkPropertyMetadataOptions.AffectsParentMeasure));

    /// <summary>Row 里要求竖向撑满（例如竖分隔线）</summary>
    public static readonly DependencyProperty StretchProperty = DependencyProperty.RegisterAttached(
        "Stretch", typeof(bool), typeof(Flex), new FrameworkPropertyMetadata(false, FrameworkPropertyMetadataOptions.AffectsParentArrange));

    public static double GetGrow(UIElement e) => (double)e.GetValue(GrowProperty);
    public static void SetGrow(UIElement e, double v) => e.SetValue(GrowProperty, v);
    public static bool GetStretch(UIElement e) => (bool)e.GetValue(StretchProperty);
    public static void SetStretch(UIElement e, bool v) => e.SetValue(StretchProperty, v);

    private static double MainOf(Size s, bool h) => h ? s.Width : s.Height;
    private static double CrossOf(Size s, bool h) => h ? s.Height : s.Width;
    private static Size Make(double main, double cross, bool h) => h ? new Size(main, cross) : new Size(cross, main);

    protected override Size MeasureOverride(Size available)
    {
        var h = Horizontal;
        var main = MainOf(available, h);
        var cross = CrossOf(available, h);
        var kids = InternalChildren.Cast<UIElement>().Where(c => c.Visibility != Visibility.Collapsed).ToList();
        double used = kids.Count > 1 ? Spacing * (kids.Count - 1) : 0;
        double maxCross = 0, totalGrow = 0;
        foreach (var c in kids)
        {
            var g = GetGrow(c);
            if (g > 0) { totalGrow += g; continue; }
            c.Measure(Make(double.PositiveInfinity, cross, h));
            used += MainOf(c.DesiredSize, h);
            maxCross = Math.Max(maxCross, CrossOf(c.DesiredSize, h));
        }
        double growDesired = 0;
        if (totalGrow > 0)
        {
            var remaining = double.IsInfinity(main) ? double.PositiveInfinity : Math.Max(0, main - used);
            foreach (var c in kids)
            {
                var g = GetGrow(c);
                if (g <= 0) continue;
                var share = double.IsInfinity(remaining) ? double.PositiveInfinity : remaining * g / totalGrow;
                c.Measure(Make(share, cross, h));
                growDesired += MainOf(c.DesiredSize, h);
                maxCross = Math.Max(maxCross, CrossOf(c.DesiredSize, h));
            }
        }
        var desiredMain = totalGrow > 0 && !double.IsInfinity(main) ? Math.Max(main, used) : used + growDesired;
        return Make(desiredMain, maxCross, h);
    }

    protected override Size ArrangeOverride(Size final)
    {
        var h = Horizontal;
        var main = MainOf(final, h);
        var cross = CrossOf(final, h);
        var kids = InternalChildren.Cast<UIElement>().Where(c => c.Visibility != Visibility.Collapsed).ToList();
        double used = kids.Count > 1 ? Spacing * (kids.Count - 1) : 0;
        double totalGrow = 0;
        foreach (var c in kids)
        {
            var g = GetGrow(c);
            if (g > 0) totalGrow += g;
            else used += MainOf(c.DesiredSize, h);
        }
        var remaining = Math.Max(0, main - used);
        double pos = 0;
        foreach (var c in kids)
        {
            var g = GetGrow(c);
            var size = g > 0 ? remaining * g / totalGrow : MainOf(c.DesiredSize, h);
            Rect slot;
            if (h)
            {
                var fe = c as FrameworkElement;
                var stretch = GetStretch(c) || (fe != null && fe.VerticalAlignment != VerticalAlignment.Stretch);
                if (stretch) slot = new Rect(pos, 0, size, cross);
                else
                {
                    var dh = Math.Min(c.DesiredSize.Height, cross);
                    slot = new Rect(pos, Math.Max(0, (cross - dh) / 2), size, dh);
                }
            }
            else slot = new Rect(0, pos, cross, size);
            c.Arrange(slot);
            pos += size + Spacing;
        }
        return final;
    }
}
