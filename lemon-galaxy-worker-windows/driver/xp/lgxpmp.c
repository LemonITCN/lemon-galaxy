/*
 * LemonGalaxy XP 虚拟屏小端口（lgxpmp.sys，XP 显示驱动模型 XPDM，32 位）。
 *
 * 没有硬件：只报模式表、按显示驱动的要求分配 / 释放帧缓冲（系统分页池）。
 * 主视图之外用 DualView 建 2 个副视图，每个视图一个 \Device\VideoN、一个注册表键。
 * 模式表 = 被控端写进注册表的首选分辨率（有就排第一）+ 常见分辨率，都是 32 位色、60 Hz。
 */
#include <ntdef.h>
#include <dderror.h>
#include <devioctl.h>
#include <miniport.h>
#include <ntddvdeo.h>
#include <video.h>
#include "lgxp.h"

typedef struct _LGXP_EXT {
    ULONG View;           /* 0 主视图，1..2 副视图 */
    ULONG Secondaries;    /* 主视图：已建的副视图数 */
    ULONG CurrentMode;    /* 最近一次 SET_CURRENT_MODE 的模式号 */
} LGXP_EXT, *PLGXP_EXT;

#define MAX_MODES 20

static const USHORT StdModes[][2] = {
    {1024, 768}, {800, 600}, {1152, 864}, {1280, 720}, {1280, 800}, {1280, 1024}, {1366, 768}, {1440, 900},
    {1600, 900}, {1600, 1200}, {1680, 1050}, {1920, 1080}, {1920, 1200}, {2560, 1440}, {2560, 1600},
};

/* 编译器可能为结构清零 / 拷贝生成这两个调用，内核里没有 C 运行库 */
void *memset(void *d, int c, size_t n) {
    unsigned char *p = d;
    while (n--) *p++ = (unsigned char)c;
    return d;
}
void *memcpy(void *d, const void *s, size_t n) {
    unsigned char *a = d;
    const unsigned char *b = s;
    while (n--) *a++ = *b++;
    return d;
}

static VP_STATUS NTAPI RegUlong(PVOID Ext, PVOID Ctx, PWSTR Name, PVOID Data, ULONG Len) {
    (void)Ext;
    (void)Name;
    if (Len < sizeof(ULONG) || Data == NULL) return ERROR_INVALID_PARAMETER;
    *(PULONG)Ctx = *(PULONG)Data;
    return NO_ERROR;
}

typedef struct { ULONG W, H; } MODE;

/* 本视图的模式表 */
static ULONG BuildModes(PLGXP_EXT Ext, MODE *Out) {
    ULONG n = 0, w = 0, h = 0, i, k;
    if (VideoPortGetRegistryParameters(Ext, LGXP_REG_WIDTH, FALSE, RegUlong, &w) != NO_ERROR) w = 0;
    if (VideoPortGetRegistryParameters(Ext, LGXP_REG_HEIGHT, FALSE, RegUlong, &h) != NO_ERROR) h = 0;
    if (w >= LGXP_MIN_W && w <= LGXP_MAX_W && h >= LGXP_MIN_H && h <= LGXP_MAX_H) {
        Out[n].W = w;
        Out[n].H = h;
        n++;
    }
    for (i = 0; i < sizeof(StdModes) / sizeof(StdModes[0]) && n < MAX_MODES; i++) {
        BOOLEAN dup = FALSE;
        for (k = 0; k < n; k++)
            if (Out[k].W == StdModes[i][0] && Out[k].H == StdModes[i][1]) dup = TRUE;
        if (!dup) {
            Out[n].W = StdModes[i][0];
            Out[n].H = StdModes[i][1];
            n++;
        }
    }
    return n;
}

static void FillMode(VIDEO_MODE_INFORMATION *m, ULONG index, const MODE *md) {
    VideoPortZeroMemory(m, sizeof(*m));
    m->Length = sizeof(*m);
    m->ModeIndex = index;
    m->VisScreenWidth = md->W;
    m->VisScreenHeight = md->H;
    m->ScreenStride = md->W * 4;
    m->NumberOfPlanes = 1;
    m->BitsPerPlane = 32;
    m->Frequency = LGXP_REFRESH;
    /* 按 96 DPI 折算毫米 */
    m->XMillimeter = md->W * 254 / 960;
    m->YMillimeter = md->H * 254 / 960;
    m->NumberRedBits = 8;
    m->NumberGreenBits = 8;
    m->NumberBlueBits = 8;
    m->RedMask = 0x00FF0000;
    m->GreenMask = 0x0000FF00;
    m->BlueMask = 0x000000FF;
    m->AttributeFlags = VIDEO_MODE_GRAPHICS | VIDEO_MODE_COLOR | VIDEO_MODE_NO_OFF_SCREEN;
    m->VideoMemoryBitmapWidth = md->W;
    m->VideoMemoryBitmapHeight = md->H;
}

static VP_STATUS NTAPI FindAdapter(PVOID HwExt, PVOID Ctx, PWSTR Arg, PVIDEO_PORT_CONFIG_INFO Config, PUCHAR Again) {
    PLGXP_EXT ext = HwExt;
    (void)Ctx;
    (void)Arg;
    (void)Config;
    *Again = FALSE;
    ext->View = 0;
    /* 副视图（DualView）：失败也不影响主视图 */
    while (ext->Secondaries < LGXP_VIEWS - 1) {
        PVOID sec = NULL;
        if (VideoPortCreateSecondaryDisplay(HwExt, &sec, VIDEO_DUALVIEW_REMOVABLE) != NO_ERROR || sec == NULL) break;
        ext->Secondaries++;
        ((PLGXP_EXT)sec)->View = ext->Secondaries;
        ((PLGXP_EXT)sec)->Secondaries = 0;
    }
    return NO_ERROR;
}

static BOOLEAN NTAPI Initialize(PVOID HwExt) {
    (void)HwExt;
    return TRUE;
}

static BOOLEAN NTAPI StartIO(PVOID HwExt, PVIDEO_REQUEST_PACKET rp) {
    PLGXP_EXT ext = HwExt;
    MODE modes[MAX_MODES];
    ULONG n, i;
    rp->StatusBlock->Status = NO_ERROR;
    rp->StatusBlock->Information = 0;
    switch (rp->IoControlCode) {
    case IOCTL_VIDEO_QUERY_NUM_AVAIL_MODES: {
        VIDEO_NUM_MODES *nm = rp->OutputBuffer;
        if (rp->OutputBufferLength < sizeof(VIDEO_NUM_MODES)) {
            rp->StatusBlock->Status = ERROR_INSUFFICIENT_BUFFER;
            break;
        }
        nm->NumModes = BuildModes(ext, modes);
        nm->ModeInformationLength = sizeof(VIDEO_MODE_INFORMATION);
        rp->StatusBlock->Information = sizeof(VIDEO_NUM_MODES);
        break;
    }
    case IOCTL_VIDEO_QUERY_AVAIL_MODES: {
        VIDEO_MODE_INFORMATION *mi = rp->OutputBuffer;
        n = BuildModes(ext, modes);
        if (rp->OutputBufferLength < n * sizeof(VIDEO_MODE_INFORMATION)) n = rp->OutputBufferLength / sizeof(VIDEO_MODE_INFORMATION);
        for (i = 0; i < n; i++) FillMode(&mi[i], i, &modes[i]);
        rp->StatusBlock->Information = n * sizeof(VIDEO_MODE_INFORMATION);
        break;
    }
    case IOCTL_VIDEO_SET_CURRENT_MODE:
        if (rp->InputBufferLength < sizeof(VIDEO_MODE)) {
            rp->StatusBlock->Status = ERROR_INSUFFICIENT_BUFFER;
            break;
        }
        ext->CurrentMode = ((VIDEO_MODE *)rp->InputBuffer)->RequestedMode & ~VIDEO_MODE_NO_ZERO_MEMORY;
        break;
    case IOCTL_VIDEO_QUERY_CURRENT_MODE:
        n = BuildModes(ext, modes);
        if (rp->OutputBufferLength < sizeof(VIDEO_MODE_INFORMATION)) {
            rp->StatusBlock->Status = ERROR_INSUFFICIENT_BUFFER;
            break;
        }
        FillMode(rp->OutputBuffer, ext->CurrentMode < n ? ext->CurrentMode : 0, &modes[ext->CurrentMode < n ? ext->CurrentMode : 0]);
        rp->StatusBlock->Information = sizeof(VIDEO_MODE_INFORMATION);
        break;
    case IOCTL_VIDEO_RESET_DEVICE:
        break;
    case IOCTL_LGXP_ALLOC_FB: {
        ULONG size;
        PVOID p;
        if (rp->InputBufferLength < sizeof(ULONG) || rp->OutputBufferLength < sizeof(PVOID)) {
            rp->StatusBlock->Status = ERROR_INSUFFICIENT_BUFFER;
            break;
        }
        size = *(PULONG)rp->InputBuffer;
        if (size == 0 || size > LGXP_MAX_W * LGXP_MAX_H * 4) {
            rp->StatusBlock->Status = ERROR_INVALID_PARAMETER;
            break;
        }
        p = VideoPortAllocatePool(HwExt, VpPagedPool, size, LGXP_TAG);
        if (p == NULL) {
            rp->StatusBlock->Status = ERROR_NOT_ENOUGH_MEMORY;
            break;
        }
        VideoPortZeroMemory(p, size);
        *(PVOID *)rp->OutputBuffer = p;
        rp->StatusBlock->Information = sizeof(PVOID);
        break;
    }
    case IOCTL_LGXP_FREE_FB:
        if (rp->InputBufferLength < sizeof(PVOID)) {
            rp->StatusBlock->Status = ERROR_INSUFFICIENT_BUFFER;
            break;
        }
        if (*(PVOID *)rp->InputBuffer) VideoPortFreePool(HwExt, *(PVOID *)rp->InputBuffer);
        break;
    default:
        rp->StatusBlock->Status = ERROR_INVALID_FUNCTION;
        break;
    }
    return TRUE;
}

static VP_STATUS NTAPI GetPowerState(PVOID HwExt, ULONG HwId, PVIDEO_POWER_MANAGEMENT p) {
    (void)HwExt;
    (void)HwId;
    (void)p;
    return NO_ERROR;
}

static VP_STATUS NTAPI SetPowerState(PVOID HwExt, ULONG HwId, PVIDEO_POWER_MANAGEMENT p) {
    (void)HwExt;
    (void)HwId;
    (void)p;
    return NO_ERROR;
}

static VP_STATUS NTAPI GetChildDescriptor(PVOID HwExt, PVIDEO_CHILD_ENUM_INFO Info, PVIDEO_CHILD_TYPE Type, PUCHAR Desc, PULONG UId, PULONG Unused) {
    (void)HwExt;
    (void)Info;
    (void)Type;
    (void)Desc;
    (void)UId;
    (void)Unused;
    return ERROR_NO_MORE_DEVICES;
}

ULONG NTAPI DriverEntry(PVOID Context1, PVOID Context2) {
    VIDEO_HW_INITIALIZATION_DATA init;
    VideoPortZeroMemory(&init, sizeof(init));
    init.HwInitDataSize = sizeof(init);
    init.HwFindAdapter = FindAdapter;
    init.HwInitialize = Initialize;
    init.HwStartIO = StartIO;
    init.HwDeviceExtensionSize = sizeof(LGXP_EXT);
    init.HwGetPowerState = GetPowerState;
    init.HwSetPowerState = SetPowerState;
    init.HwGetVideoChildDescriptor = GetChildDescriptor;
    return VideoPortInitialize(Context1, Context2, &init, NULL);
}
