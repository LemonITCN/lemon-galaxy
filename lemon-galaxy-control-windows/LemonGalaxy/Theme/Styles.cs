using System.Windows;
using System.Windows.Markup;
using System.Windows.Media;

namespace LemonGalaxy.UI;

/// <summary>
/// 系统控件的模板（右键菜单、菜单项、滚动条、提示、列表项、焦点框）。
/// 不写需要编译的 XAML：这里的 XAML 字符串在运行时由 XamlReader 解析，颜色用 DynamicResource，换主题时只更新资源里的画刷。
/// </summary>
internal static class Styles
{
    public static string? LoadError;

    private const string Xaml = """
<ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                    xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml">

  <Style x:Key="lg.FocusVisual">
    <Setter Property="Control.Template">
      <Setter.Value>
        <ControlTemplate>
          <Border Margin="-2" CornerRadius="7" BorderThickness="2" BorderBrush="{DynamicResource lg.FocusRing}" SnapsToDevicePixels="True"/>
        </ControlTemplate>
      </Setter.Value>
    </Setter>
  </Style>

  <Style TargetType="{x:Type ContextMenu}">
    <Setter Property="OverridesDefaultStyle" Value="True"/>
    <Setter Property="SnapsToDevicePixels" Value="True"/>
    <Setter Property="HasDropShadow" Value="False"/>
    <Setter Property="FontFamily" Value="{DynamicResource lg.UiFont}"/>
    <Setter Property="FontSize" Value="13"/>
    <Setter Property="Foreground" Value="{DynamicResource lg.Text}"/>
    <Setter Property="Template">
      <Setter.Value>
        <ControlTemplate TargetType="{x:Type ContextMenu}">
          <Border Margin="0,0,10,10" Background="{DynamicResource lg.Card}" BorderBrush="{DynamicResource lg.Border}" BorderThickness="1" CornerRadius="8" Padding="0,4,0,4" MinWidth="160">
            <Border.Effect>
              <DropShadowEffect BlurRadius="14" ShadowDepth="3" Direction="270" Opacity="0.22" Color="Black"/>
            </Border.Effect>
            <ScrollViewer VerticalScrollBarVisibility="Auto" HorizontalScrollBarVisibility="Disabled" MaxHeight="520">
              <StackPanel IsItemsHost="True" KeyboardNavigation.DirectionalNavigation="Cycle"/>
            </ScrollViewer>
          </Border>
        </ControlTemplate>
      </Setter.Value>
    </Setter>
  </Style>

  <Style x:Key="{x:Static MenuItem.SeparatorStyleKey}" TargetType="{x:Type Separator}">
    <Setter Property="OverridesDefaultStyle" Value="True"/>
    <Setter Property="Template">
      <Setter.Value>
        <ControlTemplate TargetType="{x:Type Separator}">
          <Border Height="1" Margin="10,4,10,4" Background="{DynamicResource lg.Separator}"/>
        </ControlTemplate>
      </Setter.Value>
    </Setter>
  </Style>

  <Style TargetType="{x:Type MenuItem}">
    <Setter Property="OverridesDefaultStyle" Value="True"/>
    <Setter Property="Foreground" Value="{DynamicResource lg.Text}"/>
    <Setter Property="FontSize" Value="13"/>
    <Setter Property="Template">
      <Setter.Value>
        <ControlTemplate TargetType="{x:Type MenuItem}">
          <Grid SnapsToDevicePixels="True">
            <Border x:Name="Bd" Background="Transparent" CornerRadius="5" Margin="4,0,4,0" Padding="8,5,10,5">
              <Grid>
                <Grid.ColumnDefinitions>
                  <ColumnDefinition Width="20"/>
                  <ColumnDefinition Width="*"/>
                  <ColumnDefinition Width="Auto"/>
                </Grid.ColumnDefinitions>
                <Path x:Name="Check" Grid.Column="0" Visibility="Hidden" Data="M1,6 L4.5,9.5 L11,2" Stroke="{DynamicResource lg.Accent}" StrokeThickness="1.8"
                      StrokeStartLineCap="Round" StrokeEndLineCap="Round" StrokeLineJoin="Round" VerticalAlignment="Center" HorizontalAlignment="Left"/>
                <ContentPresenter Grid.Column="1" ContentSource="Header" RecognizesAccessKey="False" VerticalAlignment="Center"/>
                <Path x:Name="Arrow" Grid.Column="2" Visibility="Collapsed" Data="M0,0 L4,4 L0,8" Stroke="{DynamicResource lg.Secondary}" StrokeThickness="1.5"
                      Margin="16,0,0,0" VerticalAlignment="Center"/>
              </Grid>
            </Border>
            <Popup x:Name="PART_Popup" Placement="Right" HorizontalOffset="-4" VerticalOffset="-5" AllowsTransparency="True" Focusable="False" PopupAnimation="None"
                   IsOpen="{Binding IsSubmenuOpen, RelativeSource={RelativeSource TemplatedParent}}">
              <Border Margin="0,0,10,10" Background="{DynamicResource lg.Card}" BorderBrush="{DynamicResource lg.Border}" BorderThickness="1" CornerRadius="8" Padding="0,4,0,4" MinWidth="150">
                <Border.Effect>
                  <DropShadowEffect BlurRadius="14" ShadowDepth="3" Direction="270" Opacity="0.22" Color="Black"/>
                </Border.Effect>
                <StackPanel IsItemsHost="True" KeyboardNavigation.DirectionalNavigation="Cycle"/>
              </Border>
            </Popup>
          </Grid>
          <ControlTemplate.Triggers>
            <Trigger Property="IsHighlighted" Value="True">
              <Setter TargetName="Bd" Property="Background" Value="{DynamicResource lg.Hover2}"/>
            </Trigger>
            <Trigger Property="IsChecked" Value="True">
              <Setter TargetName="Check" Property="Visibility" Value="Visible"/>
            </Trigger>
            <Trigger Property="HasItems" Value="True">
              <Setter TargetName="Arrow" Property="Visibility" Value="Visible"/>
            </Trigger>
            <Trigger Property="IsEnabled" Value="False">
              <Setter Property="Opacity" Value="0.45"/>
            </Trigger>
          </ControlTemplate.Triggers>
        </ControlTemplate>
      </Setter.Value>
    </Setter>
  </Style>

  <Style x:Key="lg.Thumb" TargetType="{x:Type Thumb}">
    <Setter Property="OverridesDefaultStyle" Value="True"/>
    <Setter Property="IsTabStop" Value="False"/>
    <Setter Property="Template">
      <Setter.Value>
        <ControlTemplate TargetType="{x:Type Thumb}">
          <Border x:Name="T" Background="{DynamicResource lg.ScrollThumb}" CornerRadius="3" Margin="2"/>
          <ControlTemplate.Triggers>
            <Trigger Property="IsMouseOver" Value="True">
              <Setter TargetName="T" Property="Background" Value="{DynamicResource lg.ScrollThumbHover}"/>
            </Trigger>
            <Trigger Property="IsDragging" Value="True">
              <Setter TargetName="T" Property="Background" Value="{DynamicResource lg.ScrollThumbHover}"/>
            </Trigger>
          </ControlTemplate.Triggers>
        </ControlTemplate>
      </Setter.Value>
    </Setter>
  </Style>

  <ControlTemplate x:Key="lg.VScroll" TargetType="{x:Type ScrollBar}">
    <Grid Background="Transparent">
      <Track x:Name="PART_Track" IsDirectionReversed="True">
        <Track.Thumb>
          <Thumb Style="{StaticResource lg.Thumb}" MinHeight="24"/>
        </Track.Thumb>
      </Track>
    </Grid>
  </ControlTemplate>

  <ControlTemplate x:Key="lg.HScroll" TargetType="{x:Type ScrollBar}">
    <Grid Background="Transparent">
      <Track x:Name="PART_Track" IsDirectionReversed="False">
        <Track.Thumb>
          <Thumb Style="{StaticResource lg.Thumb}" MinWidth="24"/>
        </Track.Thumb>
      </Track>
    </Grid>
  </ControlTemplate>

  <Style TargetType="{x:Type ScrollBar}">
    <Setter Property="OverridesDefaultStyle" Value="True"/>
    <Setter Property="SnapsToDevicePixels" Value="True"/>
    <Setter Property="Width" Value="10"/>
    <Setter Property="Height" Value="Auto"/>
    <Setter Property="Template" Value="{StaticResource lg.VScroll}"/>
    <Style.Triggers>
      <Trigger Property="Orientation" Value="Horizontal">
        <Setter Property="Width" Value="Auto"/>
        <Setter Property="Height" Value="10"/>
        <Setter Property="Template" Value="{StaticResource lg.HScroll}"/>
      </Trigger>
    </Style.Triggers>
  </Style>

  <Style TargetType="{x:Type ToolTip}">
    <Setter Property="OverridesDefaultStyle" Value="True"/>
    <Setter Property="HasDropShadow" Value="False"/>
    <Setter Property="FontFamily" Value="{DynamicResource lg.UiFont}"/>
    <Setter Property="FontSize" Value="12"/>
    <Setter Property="Foreground" Value="{DynamicResource lg.Text}"/>
    <Setter Property="Template">
      <Setter.Value>
        <ControlTemplate TargetType="{x:Type ToolTip}">
          <Border Background="{DynamicResource lg.Card}" BorderBrush="{DynamicResource lg.Border}" BorderThickness="1" CornerRadius="5" Padding="8,4,8,5">
            <ContentPresenter/>
          </Border>
        </ControlTemplate>
      </Setter.Value>
    </Setter>
  </Style>

  <Style TargetType="{x:Type ListBox}">
    <Setter Property="Background" Value="Transparent"/>
    <Setter Property="BorderThickness" Value="0"/>
    <Setter Property="Padding" Value="0"/>
    <Setter Property="ScrollViewer.HorizontalScrollBarVisibility" Value="Disabled"/>
    <Setter Property="VirtualizingPanel.IsVirtualizing" Value="True"/>
    <Setter Property="VirtualizingPanel.VirtualizationMode" Value="Recycling"/>
    <Setter Property="FocusVisualStyle" Value="{x:Null}"/>
  </Style>

  <Style TargetType="{x:Type ListBoxItem}">
    <Setter Property="OverridesDefaultStyle" Value="True"/>
    <Setter Property="FocusVisualStyle" Value="{x:Null}"/>
    <Setter Property="Padding" Value="0"/>
    <Setter Property="HorizontalContentAlignment" Value="Stretch"/>
    <Setter Property="VerticalContentAlignment" Value="Stretch"/>
    <Setter Property="Foreground" Value="{DynamicResource lg.Text}"/>
    <Setter Property="Template">
      <Setter.Value>
        <ControlTemplate TargetType="{x:Type ListBoxItem}">
          <Border x:Name="Bd" Background="Transparent" CornerRadius="5" Margin="4,0,4,0" SnapsToDevicePixels="True">
            <ContentPresenter HorizontalAlignment="{TemplateBinding HorizontalContentAlignment}" VerticalAlignment="{TemplateBinding VerticalContentAlignment}"/>
          </Border>
          <ControlTemplate.Triggers>
            <Trigger Property="IsMouseOver" Value="True">
              <Setter TargetName="Bd" Property="Background" Value="{DynamicResource lg.Hover}"/>
            </Trigger>
            <Trigger Property="IsSelected" Value="True">
              <Setter TargetName="Bd" Property="Background" Value="{DynamicResource lg.Selection}"/>
            </Trigger>
          </ControlTemplate.Triggers>
        </ControlTemplate>
      </Setter.Value>
    </Setter>
  </Style>
</ResourceDictionary>
""";

    /// <summary>把模板放进应用资源。解析失败时记下错误（selftest 会报告并以失败退出），界面退回系统默认样式。</summary>
    public static void Install(Application app)
    {
        try
        {
            var dict = (ResourceDictionary)XamlReader.Parse(Xaml);
            app.Resources.MergedDictionaries.Add(dict);
        }
        catch (Exception e)
        {
            LoadError = e.ToString();
            Log.Error("控件模板解析失败：" + e);
        }
        ApplyColors();
    }

    /// <summary>模板里用到的颜色（DynamicResource），换主题时更新</summary>
    public static void ApplyColors()
    {
        var app = Application.Current;
        if (app == null) return;
        var p = Th.P;
        var r = app.Resources;
        r["lg.Card"] = Th.B(p.Card);
        r["lg.Border"] = Th.B(p.Border);
        r["lg.Separator"] = Th.B(p.Separator);
        r["lg.Text"] = Th.B(p.Text);
        r["lg.Secondary"] = Th.B(p.SecondaryText);
        r["lg.Accent"] = Th.B(p.Accent);
        r["lg.Hover"] = Th.B(p.Hover);
        r["lg.Hover2"] = Th.Hover2;
        r["lg.Selection"] = Th.B(p.Selection);
        r["lg.FocusRing"] = Th.B(p.Accent, 0.55);
        r["lg.ScrollThumb"] = Th.B(p.IsDark ? Palettes.A(Colors.White, 0.18) : Palettes.A(Colors.Black, 0.2));
        r["lg.ScrollThumbHover"] = Th.B(p.IsDark ? Palettes.A(Colors.White, 0.3) : Palettes.A(Colors.Black, 0.34));
        r["lg.UiFont"] = Th.UiFont;
    }

    public static Style? FocusVisual => Application.Current?.TryFindResource("lg.FocusVisual") as Style;
}
