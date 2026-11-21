using System.Windows;
using System.Windows.Media;

namespace LemonGalaxy.UI;

internal static class QrImage
{
    /// <summary>画成 WPF 元素（白底，四周留 4 个模块的空白）</summary>
    public static FrameworkElement Image(string text, double size)
    {
        bool[,] m;
        try { m = QrCode.Encode(text); }
        catch { return new System.Windows.Controls.Border { Width = size, Height = size }; }
        var n = m.GetLength(0);
        var group = new GeometryGroup { FillRule = FillRule.Nonzero };
        for (var y = 0; y < n; y++)
            for (var x = 0; x < n; x++)
                if (m[y, x]) group.Children.Add(new RectangleGeometry(new Rect(x + 4, y + 4, 1, 1)));
        group.Freeze();
        var drawing = new DrawingGroup();
        drawing.Children.Add(new GeometryDrawing(Brushes.White, null, new RectangleGeometry(new Rect(0, 0, n + 8, n + 8))));
        drawing.Children.Add(new GeometryDrawing(Brushes.Black, null, group));
        drawing.Freeze();
        var img = new System.Windows.Controls.Image { Source = new DrawingImage(drawing), Width = size, Height = size, Stretch = Stretch.Fill };
        RenderOptions.SetEdgeMode(img, EdgeMode.Aliased);
        return img;
    }
}
