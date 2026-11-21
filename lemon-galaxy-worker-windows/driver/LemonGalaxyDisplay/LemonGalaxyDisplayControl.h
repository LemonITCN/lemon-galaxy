// LemonGalaxy 虚拟屏驱动的控制接口，驱动与调用方（worker 服务 / 测试工具）共用。
//
// 调用方打开下面的设备接口，按槽位（connector）增删虚拟屏。一块屏属于加它的那个句柄：句柄关闭
// （进程退出或崩溃）时驱动自动删掉它，不会留下幽灵屏。设备只对 SYSTEM 和管理员开放（见 INF）。
// 加屏后 Windows 不会自动把它扩展进桌面，调用方要自己 SetDisplayConfig 加上这条路径。
//
// 布局为 C 结构、8 字节对齐，Rust 侧按同样的字段顺序写 #[repr(C)] 即可。
#pragma once

#include <stdint.h>

#ifndef CTL_CODE
#include <winioctl.h>
#endif

// {BE3C34E4-6555-40C7-916D-A2DB09DAB193}
#define LG_DISPLAY_INTERFACE_GUID_INIT \
    { 0xbe3c34e4, 0x6555, 0x40c7, { 0x91, 0x6d, 0xa2, 0xdb, 0x09, 0xda, 0xb1, 0x93 } }

#define LG_DISPLAY_IOCTL(n) CTL_CODE(FILE_DEVICE_UNKNOWN, 0x900 + (n), METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA)
#define IOCTL_LG_DISPLAY_GET_INFO LG_DISPLAY_IOCTL(0)  // out: LgDisplayInfo
#define IOCTL_LG_DISPLAY_ADD LG_DISPLAY_IOCTL(1)       // in: LgDisplayAddRequest, out: LgDisplayAddResult
#define IOCTL_LG_DISPLAY_REMOVE LG_DISPLAY_IOCTL(2)    // in: LgDisplayRemoveRequest

#define LG_DISPLAY_INTERFACE_VERSION 1u
// 3 块给主控按需增加 + 1 块无头兜底
#define LG_DISPLAY_MAX_MONITORS 4u
#define LG_DISPLAY_MAX_MODES 64u
#define LG_DISPLAY_NAME_CHARS 14u  // EDID allows 13 characters plus the terminator

#pragma pack(push, 8)

typedef struct LgDisplayInfo {
    uint32_t version;         // LG_DISPLAY_INTERFACE_VERSION
    uint32_t maxMonitors;
    uint32_t activeMonitors;
    uint32_t adapterReady;    // 0 until the OS finished initialising the display adapter
} LgDisplayInfo;

typedef struct LgDisplayMode {
    uint32_t width;           // 320 .. 16384
    uint32_t height;          // 240 .. 16384
    uint32_t refreshRate;     // 24 .. 480 Hz
} LgDisplayMode;

typedef struct LgDisplayAddRequest {
    uint32_t version;
    uint32_t connector;        // 0 .. maxMonitors - 1; replaces a monitor already on it
    uint32_t serial;           // EDID serial: keep it stable per (controller, slot) so Windows remembers the layout
    uint32_t widthMm;          // physical size written into the EDID (DPI hints); 0 = unknown
    uint32_t heightMm;
    wchar_t name[LG_DISPLAY_NAME_CHARS];  // monitor name shown in Windows settings (ASCII is kept)
    uint32_t modeCount;        // 1 .. LG_DISPLAY_MAX_MODES
    uint32_t preferredMode;    // index into modes
    LgDisplayMode modes[LG_DISPLAY_MAX_MODES];
} LgDisplayAddRequest;

typedef struct LgDisplayAddResult {
    uint32_t version;
    uint32_t targetId;         // OS target id of the new monitor (DISPLAYCONFIG_PATH_TARGET_INFO::id)
    uint32_t adapterLuidLow;   // OS adapter LUID of the new monitor
    int32_t adapterLuidHigh;
} LgDisplayAddResult;

typedef struct LgDisplayRemoveRequest {
    uint32_t version;
    uint32_t connector;
} LgDisplayRemoveRequest;

#pragma pack(pop)
