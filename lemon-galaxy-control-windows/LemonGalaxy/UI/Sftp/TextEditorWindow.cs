using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace LemonGalaxy.UI;

/// <summary>文本文件在线编辑：读入 → 编辑 → 保存回原处（本机或远端）</summary>
internal sealed class TextEditorModel : Observable
{
    public readonly IFileSource Source;
    public readonly string Path;
    public string Text = "", Original = "";
    public bool Loading = true, Saving;
    public string? Error;

    public TextEditorModel(IFileSource source, string path) { Source = source; Path = path; }
    public bool Dirty => Text != Original;

    public async Task Load()
    {
        try
        {
            Text = await Source.ReadText(Path);
            Original = Text;
        }
        catch (Exception e) { Error = Fmt.Error(e); }
        Loading = false;
        Notify();
    }

    public async Task Save()
    {
        Saving = true;
        Notify();
        try
        {
            await Source.WriteText(Path, Text);
            Original = Text;
            Error = null;
        }
        catch (Exception e) { Error = Fmt.Error(e); }
        Saving = false;
        Notify();
    }
}

internal static class TextEditorWindow
{
    public static void Open(AppModel app, IFileSource source, string path, string title)
    {
        var model = new TextEditorModel(source, path);
        var win = new Window
        {
            Title = title + " - LemonGalaxy", Width = 820, Height = 600, Background = Th.Content, WindowStartupLocation = WindowStartupLocation.CenterOwner,
            Owner = Application.Current?.MainWindow, Content = Build(app, model),
        };
        if (MainWindow.AppIcon is { } icon) win.Icon = icon;
        win.Closing += (_, e) =>
        {
            if (!model.Dirty) return;
            var r = MessageBox.Show(win, L.T("editor.unsavedClose"), title, MessageBoxButton.YesNoCancel, MessageBoxImage.Question);
            if (r == MessageBoxResult.Cancel) e.Cancel = true;
            else if (r == MessageBoxResult.Yes) Ui.Run(model.Save);
        };
        win.Show();
        Ui.Run(model.Load);
    }

    public static FrameworkElement Build(AppModel app, TextEditorModel model)
    {
        var box = new TextBox
        {
            AcceptsReturn = true, AcceptsTab = true, TextWrapping = TextWrapping.NoWrap, BorderThickness = new Thickness(0), Background = Th.Content, Foreground = Th.Text,
            FontFamily = TerminalFonts.Family(app.Settings.TerminalFont), FontSize = Math.Max(11, app.Settings.TerminalFontSize - 1), Padding = new Thickness(10, 8, 10, 8),
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, CaretBrush = Th.Text, SelectionBrush = Th.Accent,
            FocusVisualStyle = null, Text = model.Text,
        };
        var updating = false;
        box.TextChanged += (_, _) => { if (updating) return; model.Text = box.Text; model.Notify(); };
        model.Watch(box, () =>
        {
            if (box.Text == model.Text) return;
            updating = true;
            box.Text = model.Text;
            updating = false;
        });
        box.PreviewKeyDown += (_, e) =>
        {
            if (e.Key == Key.S && Keyboard.Modifiers.HasFlag(ModifierKeys.Control)) { if (model.Dirty) Ui.Run(model.Save); e.Handled = true; }
        };
        var bar = new Region(() =>
        {
            var save = K.PrimaryButton(L.T("common.save") + "  Ctrl+S", () => Ui.Run(model.Save));
            save.IsEnabled = model.Dirty && !model.Saving;
            var path = K.Text(model.Path, 12, Th.Secondary, mono: true);
            path.TextTrimming = TextTrimming.CharacterEllipsis;
            return new Border
            {
                Height = 44, Padding = new Thickness(12, 0, 10, 0), Background = Th.List, BorderBrush = Th.Separator, BorderThickness = new Thickness(0, 0, 0, 1),
                Child = K.Row(8, K.Icon("fileText", 15), K.Grow(path),
                    model.Dirty ? K.Text(L.T("editor.unsaved"), 12, Th.Warning) : null,
                    model.Error != null ? K.Text(model.Error, 12, Th.Danger) : null,
                    model.Loading || model.Saving ? K.Spinner(14) : null,
                    save),
            };
        }).On(model);
        var dock = new DockPanel { Background = Th.Content };
        DockPanel.SetDock(bar, Dock.Top);
        dock.Children.Add(bar);
        dock.Children.Add(box);
        return dock;
    }
}
