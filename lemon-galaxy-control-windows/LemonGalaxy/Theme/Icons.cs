using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using WpfPath = System.Windows.Shapes.Path;

namespace LemonGalaxy.UI;

/// <summary>
/// 矢量图标：24×24 网格上的路径（线条风格，圆头圆角）。不依赖 Segoe MDL2 / Fluent 图标字体（Windows 7 没有）。
/// 带 Fill 前缀的是填充图标，其余是描边图标。齿轮、平台标志等在代码里生成。
/// </summary>
internal static class Icons
{
    private const string Circle9 = "M3,12 a9,9 0 1 0 18,0 a9,9 0 1 0 -18,0 ";

    private static readonly Dictionary<string, string> Data = new()
    {
        ["server"] = "M5,3 h14 a2,2 0 0 1 2,2 v4 a2,2 0 0 1 -2,2 h-14 a2,2 0 0 1 -2,-2 v-4 a2,2 0 0 1 2,-2 Z M5,13 h14 a2,2 0 0 1 2,2 v4 a2,2 0 0 1 -2,2 h-14 a2,2 0 0 1 -2,-2 v-4 a2,2 0 0 1 2,-2 Z M7,7 h0.01 M7,17 h0.01",
        ["folder"] = "M3,7 a2,2 0 0 1 2,-2 h4 l2,2 h8 a2,2 0 0 1 2,2 v8 a2,2 0 0 1 -2,2 h-14 a2,2 0 0 1 -2,-2 Z",
        ["folderPlus"] = "M3,7 a2,2 0 0 1 2,-2 h4 l2,2 h8 a2,2 0 0 1 2,2 v8 a2,2 0 0 1 -2,2 h-14 a2,2 0 0 1 -2,-2 Z M12,10 v6 M9,13 h6",
        ["sparkles"] = "M10.5,3 L12.4,8.1 L17.5,10 L12.4,11.9 L10.5,17 L8.6,11.9 L3.5,10 L8.6,8.1 Z M18.5,14 L19.4,16.6 L22,17.5 L19.4,18.4 L18.5,21 L17.6,18.4 L15,17.5 L17.6,16.6 Z M18.5,2.5 v4 M16.5,4.5 h4",
        ["code"] = "M8,7 L3,12 L8,17 M16,7 L21,12 L16,17 M14,4 L10,20",
        ["display"] = "M4,3 h16 a2,2 0 0 1 2,2 v10 a2,2 0 0 1 -2,2 h-16 a2,2 0 0 1 -2,-2 v-10 a2,2 0 0 1 2,-2 Z M8,21 h8 M12,17 v4",
        ["laptop"] = "M5,5 h14 a1,1 0 0 1 1,1 v10 H4 V6 a1,1 0 0 1 1,-1 Z M2,19 h20",
        ["chevronLeft"] = "M15,6 L9,12 L15,18",
        ["chevronRight"] = "M9,6 L15,12 L9,18",
        ["chevronDown"] = "M6,9 L12,15 L18,9",
        ["chevronUp"] = "M6,15 L12,9 L18,15",
        ["plus"] = "M12,5 v14 M5,12 h14",
        ["minus"] = "M5,12 h14",
        ["refresh"] = "M21,12 a9,9 0 1 1 -9,-9 c2.52,0 4.93,1 6.74,2.74 L21,8 M21,3 v5 h-5",
        ["search"] = "M4,11 a7,7 0 1 0 14,0 a7,7 0 1 0 -14,0 M20,20 L16.2,16.2",
        ["close"] = "M6,6 L18,18 M18,6 L6,18",
        ["info"] = Circle9 + "M12,11 v5 M12,7.8 v0.01",
        ["terminal"] = "M4,17 L10,11 L4,5 M12,19 h8",
        ["key"] = "M3,15 a4,4 0 1 0 8,0 a4,4 0 1 0 -8,0 M9.8,12.2 L20,2 M16,6 L19,9 M18,4 L21,7",
        ["lock"] = "M5,11 h14 a2,2 0 0 1 2,2 v7 a2,2 0 0 1 -2,2 h-14 a2,2 0 0 1 -2,-2 v-7 a2,2 0 0 1 2,-2 Z M7,11 V7 a5,5 0 0 1 10,0 v4",
        ["eyeOff"] = "M9.9,4.7 A10,10 0 0 1 12,4.5 C15,4.5 19,6 22,12 a17,17 0 0 1 -2.2,3.2 M6.6,6.6 C4.6,7.9 3,9.8 2,12 C5,18 9,19.5 12,19.5 a9.7,9.7 0 0 0 5.4,-1.6 M9.9,9.9 a3,3 0 0 0 4.2,4.2 M2,2 L22,22",
        ["eye"] = "M2,12 C5,6 9,4.5 12,4.5 C15,4.5 19,6 22,12 C19,18 15,19.5 12,19.5 C9,19.5 5,18 2,12 Z M9,12 a3,3 0 1 0 6,0 a3,3 0 1 0 -6,0",
        ["pencil"] = "M17,3 a2.83,2.83 0 0 1 4,4 L7.5,20.5 L2,22 L3.5,16.5 Z M15,5 L19,9",
        ["trash"] = "M3,6 h18 M8,6 V4 h8 v2 M19,6 l-1,14 a2,2 0 0 1 -2,2 h-8 a2,2 0 0 1 -2,-2 L5,6 M10,11 v6 M14,11 v6",
        ["arrowUp"] = "M12,19 V5 M5,12 L12,5 L19,12",
        ["arrowLeft"] = "M19,12 H5 M12,19 L5,12 L12,5",
        ["arrowRight"] = "M5,12 h14 M12,5 l7,7 l-7,7",
        ["upCircle"] = Circle9 + "M12,16 V8 M8,12 L12,8 L16,12",
        ["downCircle"] = Circle9 + "M12,8 v8 M8,12 L12,16 L16,12",
        ["keyboard"] = "M4,5 h16 a2,2 0 0 1 2,2 v10 a2,2 0 0 1 -2,2 h-16 a2,2 0 0 1 -2,-2 v-10 a2,2 0 0 1 2,-2 Z M6,9 h0.01 M10,9 h0.01 M14,9 h0.01 M18,9 h0.01 M6,13 h0.01 M18,13 h0.01 M10,13 h4 M8,16 h8",
        ["command"] = "M15,6 v12 a3,3 0 1 0 3,-3 H6 a3,3 0 1 0 3,3 V6 a3,3 0 1 0 -3,3 h12 a3,3 0 1 0 -3,-3",
        ["clipboard"] = "M9,3 h6 a1,1 0 0 1 1,1 v2 a1,1 0 0 1 -1,1 h-6 a1,1 0 0 1 -1,-1 v-2 a1,1 0 0 1 1,-1 Z M16,5 h2 a2,2 0 0 1 2,2 v13 a2,2 0 0 1 -2,2 h-12 a2,2 0 0 1 -2,-2 v-13 a2,2 0 0 1 2,-2 h2",
        ["fullscreen"] = "M15,3 h6 v6 M9,21 H3 v-6 M21,3 l-7,7 M3,21 l7,-7",
        ["fit"] = "M3,8 V5 a2,2 0 0 1 2,-2 h3 M16,3 h3 a2,2 0 0 1 2,2 v3 M21,16 v3 a2,2 0 0 1 -2,2 h-3 M8,21 H5 a2,2 0 0 1 -2,-2 v-3",
        ["home"] = "M3,10 L12,3 L21,10 V20 a1,1 0 0 1 -1,1 h-5 v-6 h-6 v6 h-5 a1,1 0 0 1 -1,-1 Z",
        ["sidebar"] = "M5,4 h14 a2,2 0 0 1 2,2 v12 a2,2 0 0 1 -2,2 h-14 a2,2 0 0 1 -2,-2 v-12 a2,2 0 0 1 2,-2 Z M9,4 v16",
        ["shield"] = "M12,22 C12,22 20,18 20,12 V5 L12,2 L4,5 V12 C4,18 12,22 12,22 Z",
        ["shieldCheck"] = "M12,22 C12,22 20,18 20,12 V5 L12,2 L4,5 V12 C4,18 12,22 12,22 Z M9,12 L11,14 L15,10",
        ["shieldAlert"] = "M12,22 C12,22 20,18 20,12 V5 L12,2 L4,5 V12 C4,18 12,22 12,22 Z M12,8 v4 M12,15.5 v0.01",
        ["bolt"] = "M13,2 L3,14 h9 l-1,8 L21,10 h-9 Z",
        ["hand"] = "M18,11 V6 a2,2 0 0 0 -4,0 v5 M14,10 V4 a2,2 0 0 0 -4,0 v6 M10,10.5 V6 a2,2 0 0 0 -4,0 v8 M18,8 a2,2 0 1 1 4,0 v6 a8,8 0 0 1 -8,8 h-2 c-2.8,0 -4.5,-0.86 -5.99,-2.34 l-3.6,-3.6 a2,2 0 0 1 2.83,-2.82 L7,15",
        ["warning"] = "M10.3,3.9 L1.8,18 a2,2 0 0 0 1.7,3 h17 a2,2 0 0 0 1.7,-3 L13.7,3.9 a2,2 0 0 0 -3.4,0 Z M12,9 v4 M12,17 h0.01",
        ["check"] = "M20,6 L9,17 L4,12",
        ["checkCircle"] = Circle9 + "M8,12 l3,3 l5,-6",
        ["xCircle"] = Circle9 + "M15,9 L9,15 M9,9 L15,15",
        ["alertCircle"] = Circle9 + "M12,8 v4 M12,16 h0.01",
        ["ask"] = "M21,15 a2,2 0 0 1 -2,2 H7 l-4,4 V5 a2,2 0 0 1 2,-2 h14 a2,2 0 0 1 2,2 Z M9.5,8 a2.5,2.5 0 1 1 3.5,2.3 c-0.6,0.3 -1,0.8 -1,1.5 M12,14.5 h0.01",
        ["chat"] = "M21,15 a2,2 0 0 1 -2,2 H7 l-4,4 V5 a2,2 0 0 1 2,-2 h14 a2,2 0 0 1 2,2 Z",
        ["chatText"] = "M21,15 a2,2 0 0 1 -2,2 H7 l-4,4 V5 a2,2 0 0 1 2,-2 h14 a2,2 0 0 1 2,2 Z M7,8 h10 M7,12 h6",
        ["bulb"] = "M9,18 h6 M10,22 h4 M12,2 a7,7 0 0 0 -4,12.7 V16 h8 v-1.3 A7,7 0 0 0 12,2 Z",
        ["wifi"] = "M5,12.5 a10,10 0 0 1 14,0 M8.5,16 a5,5 0 0 1 7,0 M2,9 a15,15 0 0 1 20,0 M12,20 h0.01",
        ["direct"] = "M8,3 L4,7 L8,11 M4,7 h16 M16,21 l4,-4 l-4,-4 M20,17 H4",
        ["relay"] = "M6,3 v12 M18,9 a3,3 0 1 0 0,-6 a3,3 0 1 0 0,6 Z M6,21 a3,3 0 1 0 0,-6 a3,3 0 1 0 0,6 Z M18,9 a9,9 0 0 1 -9,9",
        ["globe"] = Circle9 + "M3,12 h18 M12,3 a14,14 0 0 1 0,18 a14,14 0 0 1 0,-18 Z",
        ["user"] = "M20,21 v-2 a4,4 0 0 0 -4,-4 H8 a4,4 0 0 0 -4,4 v2 M8,7 a4,4 0 1 0 8,0 a4,4 0 1 0 -8,0",
        ["userCircle"] = Circle9 + "M9,10 a3,3 0 1 0 6,0 a3,3 0 1 0 -6,0 M6.2,18.4 a6.5,6.5 0 0 1 11.6,0",
        ["users"] = "M16,21 v-2 a4,4 0 0 0 -4,-4 H6 a4,4 0 0 0 -4,4 v2 M5,7 a4,4 0 1 0 8,0 a4,4 0 1 0 -8,0 M22,21 v-2 a4,4 0 0 0 -3,-3.87 M16,3.13 a4,4 0 0 1 0,7.75",
        ["power"] = "M12,2 v10 M18.4,6.6 a9,9 0 1 1 -12.77,0.04",
        ["list"] = "M8,6 h13 M8,12 h13 M8,18 h13 M3,6 h0.01 M3,12 h0.01 M3,18 h0.01",
        ["file"] = "M14,2 H6 a2,2 0 0 0 -2,2 v16 a2,2 0 0 0 2,2 h12 a2,2 0 0 0 2,-2 V8 Z M14,2 v6 h6",
        ["fileText"] = "M14,2 H6 a2,2 0 0 0 -2,2 v16 a2,2 0 0 0 2,2 h12 a2,2 0 0 0 2,-2 V8 Z M14,2 v6 h6 M8,13 h8 M8,17 h8 M8,9 h2",
        ["fileCode"] = "M14,2 H6 a2,2 0 0 0 -2,2 v16 a2,2 0 0 0 2,2 h12 a2,2 0 0 0 2,-2 V8 Z M14,2 v6 h6 M10,12 l-2,2.5 l2,2.5 M14,12 l2,2.5 l-2,2.5",
        ["image"] = "M5,3 h14 a2,2 0 0 1 2,2 v14 a2,2 0 0 1 -2,2 H5 a2,2 0 0 1 -2,-2 V5 a2,2 0 0 1 2,-2 Z M7,9 a2,2 0 1 0 4,0 a2,2 0 1 0 -4,0 M21,15 l-5,-5 L5,21",
        ["archive"] = "M3,4 h18 v4 H3 Z M5,8 v11 a2,2 0 0 0 2,2 h10 a2,2 0 0 0 2,-2 V8 M10,12 h4",
        ["link"] = "M10,13 a5,5 0 0 0 7.54,0.54 l3,-3 a5,5 0 0 0 -7.07,-7.07 l-1.72,1.71 M14,11 a5,5 0 0 0 -7.54,-0.54 l-3,3 a5,5 0 0 0 7.07,7.07 l1.71,-1.71",
        ["hourglass"] = "M5,22 h14 M5,2 h14 M17,22 v-4.17 a2,2 0 0 0 -0.59,-1.42 L12,12 L7.59,16.41 A2,2 0 0 0 7,17.83 V22 M7,2 v4.17 a2,2 0 0 0 0.59,1.42 L12,12 l4.41,-4.41 A2,2 0 0 0 17,6.17 V2",
        ["ban"] = Circle9 + "M5.6,5.6 L18.4,18.4",
        ["copy"] = "M11,9 h9 a2,2 0 0 1 2,2 v9 a2,2 0 0 1 -2,2 h-9 a2,2 0 0 1 -2,-2 v-9 a2,2 0 0 1 2,-2 Z M5,15 H4 a2,2 0 0 1 -2,-2 V4 a2,2 0 0 1 2,-2 h9 a2,2 0 0 1 2,2 v1",
        ["more"] = "M5,12 h0.01 M12,12 h0.01 M19,12 h0.01",
        ["winMin"] = "M5,12 h14",
        ["winMax"] = "M5,5 h14 v14 h-14 Z",
        ["winRestore"] = "M8,8 h11 v11 h-11 Z M5,16 V5 h11",
        ["sliders"] = "M4,21 v-7 M4,10 V3 M12,21 v-9 M12,8 V3 M20,21 v-5 M20,12 V3 M1,14 h6 M9,8 h6 M17,16 h6",
        ["squarePen"] = "M12,3 H5 a2,2 0 0 0 -2,2 v14 a2,2 0 0 0 2,2 h14 a2,2 0 0 0 2,-2 v-7 M18.4,2.6 a2.1,2.1 0 0 1 3,3 L12,15 l-4,1 l1,-4 Z",
        ["columns"] = "M5,3 h14 a2,2 0 0 1 2,2 v14 a2,2 0 0 1 -2,2 H5 a2,2 0 0 1 -2,-2 V5 a2,2 0 0 1 2,-2 Z M12,3 v18",
        ["radio"] = "M4.9,19.1 a10,10 0 0 1 0,-14.2 M7.8,16.2 a6,6 0 0 1 0,-8.4 M16.2,7.8 a6,6 0 0 1 0,8.4 M19.1,4.9 a10,10 0 0 1 0,14.2 M10,12 a2,2 0 1 0 4,0 a2,2 0 1 0 -4,0",
        ["palette"] = "M12,22 a10,10 0 1 1 10,-10 c0,2.8 -2.2,3 -4,3 h-2 a2,2 0 0 0 -1.4,3.4 c0.6,0.6 0.4,1.6 -0.6,2.6 Z M7.5,11 h0.01 M10,7 h0.01 M15,7.5 h0.01",
        ["network"] = "M9,2 h6 v6 h-6 Z M2,16 h6 v6 h-6 Z M16,16 h6 v6 h-6 Z M5,16 v-3 h14 v3 M12,8 v5",
        ["reply"] = "M9,17 L4,12 L9,7 M20,18 v-2 a4,4 0 0 0 -4,-4 H4",
        ["wrench"] = "M14.7,6.3 a1,1 0 0 0 0,1.4 l1.6,1.6 a1,1 0 0 0 1.4,0 l3.77,-3.77 a6,6 0 0 1 -7.94,7.94 l-6.91,6.91 a2.12,2.12 0 0 1 -3,-3 l6.91,-6.91 a6,6 0 0 1 7.94,-7.94 Z",
        ["cursor"] = "M4,4 L11,21 L13.5,13.5 L21,11 Z",
        ["camera"] = "M3,8 a2,2 0 0 1 2,-2 h2 l2,-3 h6 l2,3 h2 a2,2 0 0 1 2,2 v11 a2,2 0 0 1 -2,2 H5 a2,2 0 0 1 -2,-2 Z M9,13 a3,3 0 1 0 6,0 a3,3 0 1 0 -6,0",
        ["phone"] = "M7,2 h10 a2,2 0 0 1 2,2 v16 a2,2 0 0 1 -2,2 H7 a2,2 0 0 1 -2,-2 V4 a2,2 0 0 1 2,-2 Z M11,18 h2",
        ["hexagon"] = "M12,2 L20.7,7 v10 L12,22 L3.3,17 V7 Z M12,8 L15.5,10 v4 L12,16 L8.5,14 v-4 Z",
        ["logout"] = "M9,21 H5 a2,2 0 0 1 -2,-2 V5 a2,2 0 0 1 2,-2 h4 M16,17 l5,-5 l-5,-5 M21,12 H9",
        ["download"] = "M21,15 v4 a2,2 0 0 1 -2,2 H5 a2,2 0 0 1 -2,-2 v-4 M7,10 l5,5 l5,-5 M12,15 V3",
        ["upload"] = "M21,15 v4 a2,2 0 0 1 -2,2 H5 a2,2 0 0 1 -2,-2 v-4 M17,8 l-5,-5 l-5,5 M12,3 v12",
        ["hash"] = "M4,9 h16 M4,15 h16 M10,3 L8,21 M16,3 l-2,18",
        ["clock"] = Circle9 + "M12,7 v5 l3,2",
        ["swap"] = "M7,16 V4 M3,8 l4,-4 l4,4 M17,8 v12 M21,16 l-4,4 l-4,-4",
        ["tag"] = "M12.6,2.6 A2,2 0 0 0 11.2,2 H4 a2,2 0 0 0 -2,2 v7.2 a2,2 0 0 0 0.6,1.4 l8.7,8.7 a2.4,2.4 0 0 0 3.4,0 l6.6,-6.6 a2.4,2.4 0 0 0 0,-3.4 Z M7,7.5 h0.01",
        ["lockScreen"] = "M5,11 h14 a2,2 0 0 1 2,2 v7 a2,2 0 0 1 -2,2 h-14 a2,2 0 0 1 -2,-2 v-7 a2,2 0 0 1 2,-2 Z M7,11 V7 a5,5 0 0 1 10,0 v4 M12,15 v3",
        ["skip"] = "M5,4 L15,12 L5,20 Z M19,5 v14",
        ["Fillstar"] = "M12,2.5 L14.9,8.6 L21.5,9.3 L16.6,13.8 L18,20.3 L12,17 L6,20.3 L7.4,13.8 L2.5,9.3 L9.1,8.6 Z",
        ["Fillplay"] = "M7,4 L20,12 L7,20 Z",
        ["Fillpause"] = "M6,4 h4 v16 h-4 Z M14,4 h4 v16 h-4 Z",
        ["Fillstop"] = "M6,6 h12 v12 h-12 Z",
        ["Filldot"] = "M8,12 a4,4 0 1 0 8,0 a4,4 0 1 0 -8,0 Z",
        ["FillfolderFill"] = "M3,7 a2,2 0 0 1 2,-2 h4 l2,2 h8 a2,2 0 0 1 2,2 v8 a2,2 0 0 1 -2,2 h-14 a2,2 0 0 1 -2,-2 Z",
        ["Fillapple"] = "M16.4,12.6 c0,-2.5 2,-3.6 2.1,-3.7 c-1.2,-1.7 -3,-1.9 -3.6,-1.9 c-1.5,-0.2 -3,0.9 -3.8,0.9 c-0.8,0 -2,-0.9 -3.3,-0.8 c-1.7,0 -3.3,1 -4.2,2.5 c-1.8,3.1 -0.5,7.7 1.3,10.2 c0.9,1.2 1.9,2.6 3.2,2.6 c1.3,-0.1 1.8,-0.8 3.3,-0.8 c1.5,0 2,0.8 3.3,0.8 c1.4,0 2.3,-1.3 3.1,-2.5 c1,-1.4 1.4,-2.8 1.4,-2.9 c0,0 -2.8,-1.1 -2.8,-4.4 Z M14,5.2 c0.7,-0.9 1.2,-2 1.1,-3.2 c-1,0 -2.3,0.7 -3,1.6 c-0.7,0.8 -1.2,2 -1.1,3.1 c1.1,0.1 2.3,-0.6 3,-1.5 Z",
    };

    private static readonly Dictionary<string, Geometry> Cache = new();

    private static readonly HashSet<string> Generated = new() { "gear", "windows", "linux", "android" };

    public static Geometry Get(string name)
    {
        if (Cache.TryGetValue(name, out var g)) return g;
        g = name switch
        {
            "gear" => Gear(),
            "windows" => WindowsLogo(),
            "linux" => LinuxLogo(),
            "android" => AndroidLogo(),
            _ => Data.TryGetValue(name, out var d) ? Geometry.Parse(d) : Data.TryGetValue("Fill" + name, out var f) ? Geometry.Parse(f) : Geometry.Parse(Data["info"]),
        };
        g.Freeze();
        Cache[name] = g;
        return g;
    }

    /// <summary>名字是否需要填充绘制</summary>
    private static bool Filled(string name) => name is "windows" or "linux" or "android" || !Data.ContainsKey(name) && Data.ContainsKey("Fill" + name);

    /// <summary>所有图标（selftest 逐个解析检查）</summary>
    public static IEnumerable<string> AllNames =>
        Data.Keys.Select(k => k.StartsWith("Fill", StringComparison.Ordinal) ? k.Substring(4) : k).Concat(Generated);

    public static FrameworkElement Make(string name, double size, Brush brush, double stroke = 2)
    {
        var path = new WpfPath { Data = Get(name), Width = 24, Height = 24, Stretch = Stretch.None, SnapsToDevicePixels = false };
        if (Filled(name)) path.Fill = brush;
        else
        {
            path.Stroke = brush;
            path.StrokeThickness = stroke;
            path.StrokeStartLineCap = PenLineCap.Round;
            path.StrokeEndLineCap = PenLineCap.Round;
            path.StrokeLineJoin = PenLineJoin.Round;
        }
        return new Viewbox { Width = size, Height = size, Child = path, Stretch = Stretch.Uniform, IsHitTestVisible = false };
    }

    /// <summary>齿轮：8 个齿的轮廓 + 中心圆（描边）</summary>
    private static Geometry Gear()
    {
        var fig = new PathFigure { IsClosed = true };
        const double cx = 12, cy = 12, outer = 10, inner = 7.6;
        Point P(double r, double deg) => new(cx + r * Math.Cos(deg * Math.PI / 180), cy + r * Math.Sin(deg * Math.PI / 180));
        var first = true;
        for (var i = 0; i < 8; i++)
        {
            var c = i * 45.0;
            var pts = new[] { P(inner, c - 16), P(outer, c - 9), P(outer, c + 9), P(inner, c + 16) };
            foreach (var p in pts)
            {
                if (first) { fig.StartPoint = p; first = false; }
                else fig.Segments.Add(new LineSegment(p, true));
            }
        }
        var g = new PathGeometry();
        g.Figures.Add(fig);
        var group = new GeometryGroup { FillRule = FillRule.EvenOdd };
        group.Children.Add(g);
        group.Children.Add(new EllipseGeometry(new Point(cx, cy), 3, 3));
        return group;
    }

    private static Geometry WindowsLogo()
    {
        var g = new GeometryGroup();
        const double s = 20, gap = 1.6, h = (s - gap) / 2, o = 2;
        for (var i = 0; i < 4; i++)
            g.Children.Add(new RectangleGeometry(new Rect(o + i % 2 * (h + gap), o + i / 2 * (h + gap), h, h)));
        return g;
    }

    /// <summary>简化的企鹅轮廓：身体、肚皮镂空、眼睛、两只脚（与 Mac 端同样的画法）</summary>
    private static Geometry LinuxLogo()
    {
        const double w = 20, h = 22, ox = 2, oy = 1;
        var body = new GeometryGroup { FillRule = FillRule.EvenOdd };
        body.Children.Add(new RectangleGeometry(new Rect(ox + w * 0.12, oy, w * 0.76, h * 0.9), w * 0.38, h * 0.4));
        body.Children.Add(new EllipseGeometry(new Rect(ox + w * 0.3, oy + h * 0.4, w * 0.4, h * 0.42)));
        body.Children.Add(new EllipseGeometry(new Rect(ox + w * 0.32, oy + h * 0.16, w * 0.12, h * 0.1)));
        body.Children.Add(new EllipseGeometry(new Rect(ox + w * 0.56, oy + h * 0.16, w * 0.12, h * 0.1)));
        var all = new GeometryGroup { FillRule = FillRule.Nonzero };
        all.Children.Add(body);
        all.Children.Add(new EllipseGeometry(new Rect(ox, oy + h * 0.82, w * 0.44, h * 0.18)));
        all.Children.Add(new EllipseGeometry(new Rect(ox + w * 0.56, oy + h * 0.82, w * 0.44, h * 0.18)));
        return all;
    }

    private static Geometry AndroidLogo()
    {
        // 半圆头 + 两根天线 + 两只眼睛（镂空）
        var head = Geometry.Parse("M2,19 A10,10 0 0 1 22,19 Z");
        var eyes = new GeometryGroup();
        eyes.Children.Add(new EllipseGeometry(new Point(8, 14.5), 1.3, 1.3));
        eyes.Children.Add(new EllipseGeometry(new Point(16, 14.5), 1.3, 1.3));
        var face = new CombinedGeometry(GeometryCombineMode.Exclude, head, eyes);
        var ant = new PathGeometry();
        ant.AddGeometry(new LineGeometry(new Point(7, 10), new Point(4.5, 5.5)).GetWidenedPathGeometry(new Pen(Brushes.Black, 1.6) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round }));
        ant.AddGeometry(new LineGeometry(new Point(17, 10), new Point(19.5, 5.5)).GetWidenedPathGeometry(new Pen(Brushes.Black, 1.6) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round }));
        var g = new GeometryGroup { FillRule = FillRule.Nonzero };
        g.Children.Add(face);
        g.Children.Add(ant);
        return g;
    }
}
