// 与主控端共用的控件（Shared\Kit\Controls.cs）里有两个小函数用到内核绑定的类型（设备在线点、平台图标）。
// 被控端界面不链接内核，这里给出同名的最小类型，只为让共用源码编译通过（被控端界面不调用那两个函数）。
namespace LemonGalaxy.Core.Native
{
    internal enum Platform { Windows, Macos, Linux, Ios, Android, Harmonyos }

    internal sealed class DeviceInfo
    {
        public bool Online { get; set; }
        public bool LanSeen { get; set; }
    }
}
