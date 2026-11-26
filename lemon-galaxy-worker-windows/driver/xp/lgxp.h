/*
 * LemonGalaxy XP 虚拟屏驱动：显示驱动（lgxpdd.dll）与小端口（lgxpmp.sys）共用的定义。
 *
 * 一个根枚举设备 Root\LemonGalaxyDisplayXP，小端口用 DualView 再建 2 个副视图，共 3 块屏
 * （\\.\DISPLAYn 各一个，平时不接入桌面；被控端按需用 ChangeDisplaySettingsEx 接入 / 断开）。
 * 帧缓冲由小端口从系统分页池分配，显示驱动在上面建 GDI 管理的位图，所有绘制交给 GDI。
 */
#pragma once

/* 自定义控制码：显示驱动经 EngDeviceIoControl 找小端口分配 / 释放帧缓冲 */
#define LGXP_CTL(n) (((0x00000023) << 16) | ((0x900 + (n)) << 2)) /* FILE_DEVICE_VIDEO, METHOD_BUFFERED, FILE_ANY_ACCESS */
#define IOCTL_LGXP_ALLOC_FB LGXP_CTL(0) /* 入：ULONG 字节数；出：PVOID */
#define IOCTL_LGXP_FREE_FB LGXP_CTL(1)  /* 入：PVOID */

/* 被控端写进每块屏的显示设备键（EnumDisplayDevices 的 DeviceKey）的首选分辨率，接入前写好 */
#define LGXP_REG_WIDTH L"LemonGalaxy.Width"
#define LGXP_REG_HEIGHT L"LemonGalaxy.Height"

#define LGXP_VIEWS 3
#define LGXP_MIN_W 640
#define LGXP_MIN_H 480
#define LGXP_MAX_W 3840
#define LGXP_MAX_H 2400
#define LGXP_REFRESH 60
#define LGXP_TAG 0x7078474C /* 'LGxp' */
