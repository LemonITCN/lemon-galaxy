/*
 * LemonGalaxy XP 虚拟屏显示驱动（lgxpdd.dll，XPDM，在 win32k 里运行，32 位）。
 *
 * 最简单的帧缓冲驱动：表面是 GDI 管理的 32 位位图（内存向小端口要），不挂任何绘图函数，全部由 GDI 画；
 * 光标当成硬件光标接下来但不画（远端由被控端自己叠加光标，画面里不能再有一个）。
 */
#include <stddef.h>
#include <stdarg.h>
#include <windef.h>
#include <wingdi.h>
#include <winddi.h>
#include <devioctl.h>
#include <ntddvdeo.h>
#include "lgxp.h"

typedef struct _PDEV {
    HANDLE hDriver;
    HDEV hdev;
    HSURF hsurf;
    HPALETTE hpal;
    PVOID fb;
    ULONG mode;
    ULONG cx, cy;
} PDEV, *PPDEV;

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

static void CopyName(WCHAR *dst, const WCHAR *src, int n) {
    int i = 0;
    for (; i < n - 1 && src[i]; i++) dst[i] = src[i];
    dst[i] = 0;
}

/* 小端口的模式表（调用方 EngFreeMem），返回个数；失败返回 0 */
static ULONG QueryModes(HANDLE hDriver, VIDEO_MODE_INFORMATION **out) {
    VIDEO_NUM_MODES nm;
    DWORD ret = 0;
    VIDEO_MODE_INFORMATION *mi;
    *out = NULL;
    if (EngDeviceIoControl(hDriver, IOCTL_VIDEO_QUERY_NUM_AVAIL_MODES, NULL, 0, &nm, sizeof(nm), &ret) || nm.NumModes == 0 || nm.ModeInformationLength != sizeof(VIDEO_MODE_INFORMATION)) return 0;
    mi = EngAllocMem(FL_ZERO_MEMORY, nm.NumModes * sizeof(VIDEO_MODE_INFORMATION), LGXP_TAG);
    if (!mi) return 0;
    if (EngDeviceIoControl(hDriver, IOCTL_VIDEO_QUERY_AVAIL_MODES, NULL, 0, mi, nm.NumModes * sizeof(VIDEO_MODE_INFORMATION), &ret) || ret < sizeof(VIDEO_MODE_INFORMATION)) {
        EngFreeMem(mi);
        return 0;
    }
    *out = mi;
    return ret / sizeof(VIDEO_MODE_INFORMATION);
}

static const LOGFONTW FontSystem = {16, 7, 0, 0, 700, 0, 0, 0, ANSI_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, DEFAULT_QUALITY, VARIABLE_PITCH | FF_DONTCARE, L"System"};
static const LOGFONTW FontHelv = {12, 9, 0, 0, 400, 0, 0, 0, ANSI_CHARSET, OUT_DEFAULT_PRECIS, CLIP_STROKE_PRECIS, PROOF_QUALITY, VARIABLE_PITCH | FF_DONTCARE, L"MS Sans Serif"};
static const LOGFONTW FontCour = {12, 9, 0, 0, 400, 0, 0, 0, ANSI_CHARSET, OUT_DEFAULT_PRECIS, CLIP_STROKE_PRECIS, PROOF_QUALITY, FIXED_PITCH | FF_DONTCARE, L"Courier"};

DHPDEV APIENTRY DrvEnablePDEV(DEVMODEW *pdm, LPWSTR pwszLogAddress, ULONG cPat, HSURF *phsurfPatterns, ULONG cjCaps, ULONG *pdevcaps, ULONG cjDevInfo, DEVINFO *pdi, HDEV hdev, LPWSTR pwszDeviceName, HANDLE hDriver) {
    VIDEO_MODE_INFORMATION *modes, *m = NULL;
    ULONG n, i;
    PPDEV ppdev;
    GDIINFO gi;
    DEVINFO di;
    (void)pwszLogAddress;
    (void)cPat;
    (void)phsurfPatterns;
    (void)hdev;
    (void)pwszDeviceName;
    n = QueryModes(hDriver, &modes);
    if (n == 0) return NULL;
    for (i = 0; i < n; i++) {
        if (pdm && pdm->dmPelsWidth == modes[i].VisScreenWidth && pdm->dmPelsHeight == modes[i].VisScreenHeight && (pdm->dmBitsPerPel == 0 || pdm->dmBitsPerPel == 32)) {
            m = &modes[i];
            break;
        }
    }
    /* 没指定（全 0）时用第一个模式（首选） */
    if (!m && (!pdm || (pdm->dmPelsWidth == 0 && pdm->dmPelsHeight == 0))) m = &modes[0];
    if (!m) {
        EngFreeMem(modes);
        return NULL;
    }
    ppdev = EngAllocMem(FL_ZERO_MEMORY, sizeof(PDEV), LGXP_TAG);
    if (!ppdev) {
        EngFreeMem(modes);
        return NULL;
    }
    ppdev->hDriver = hDriver;
    ppdev->mode = m->ModeIndex;
    ppdev->cx = m->VisScreenWidth;
    ppdev->cy = m->VisScreenHeight;

    memset(&gi, 0, sizeof(gi));
    gi.ulVersion = 0x5000;
    gi.ulTechnology = DT_RASDISPLAY;
    gi.ulHorzSize = m->XMillimeter;
    gi.ulVertSize = m->YMillimeter;
    gi.ulHorzRes = ppdev->cx;
    gi.ulVertRes = ppdev->cy;
    gi.cBitsPixel = 32;
    gi.cPlanes = 1;
    gi.ulNumColors = (ULONG)-1;
    gi.flRaster = 0;
    gi.ulLogPixelsX = 96;
    gi.ulLogPixelsY = 96;
    gi.flTextCaps = TC_RA_ABLE;
    gi.ulDACRed = 8;
    gi.ulDACGreen = 8;
    gi.ulDACBlue = 8;
    gi.ulAspectX = 0x24;
    gi.ulAspectY = 0x24;
    gi.ulAspectXY = 0x33;
    gi.xStyleStep = 1;
    gi.yStyleStep = 1;
    gi.denStyleStep = 3;
    gi.ulNumPalReg = 0;
    gi.ciDevice.Red.x = 6700;
    gi.ciDevice.Red.y = 3300;
    gi.ciDevice.Red.Y = 0;
    gi.ciDevice.Green.x = 2100;
    gi.ciDevice.Green.y = 7100;
    gi.ciDevice.Green.Y = 0;
    gi.ciDevice.Blue.x = 1400;
    gi.ciDevice.Blue.y = 800;
    gi.ciDevice.Blue.Y = 0;
    gi.ciDevice.Cyan.x = 1750;
    gi.ciDevice.Cyan.y = 3950;
    gi.ciDevice.Cyan.Y = 0;
    gi.ciDevice.Magenta.x = 4050;
    gi.ciDevice.Magenta.y = 2050;
    gi.ciDevice.Magenta.Y = 0;
    gi.ciDevice.Yellow.x = 4400;
    gi.ciDevice.Yellow.y = 5200;
    gi.ciDevice.Yellow.Y = 0;
    gi.ciDevice.AlignmentWhite.x = 3127;
    gi.ciDevice.AlignmentWhite.y = 3290;
    gi.ciDevice.AlignmentWhite.Y = 0;
    gi.ciDevice.RedGamma = 20000;
    gi.ciDevice.GreenGamma = 20000;
    gi.ciDevice.BlueGamma = 20000;
    gi.ulDevicePelsDPI = 0;
    gi.ulPrimaryOrder = PRIMARY_ORDER_CBA;
    gi.ulHTPatternSize = HT_PATSIZE_4x4_M;
    gi.ulHTOutputFormat = HT_FORMAT_32BPP;
    gi.flHTFlags = HT_FLAG_ADDITIVE_PRIMS;
    gi.ulVRefresh = m->Frequency;
    gi.ulBltAlignment = 0;
    gi.ulPhysicalPixelCharacteristics = PPC_DEFAULT;
    gi.ulPhysicalPixelGamma = PPG_DEFAULT;
    memcpy(pdevcaps, &gi, cjCaps < sizeof(gi) ? cjCaps : sizeof(gi));

    memset(&di, 0, sizeof(di));
    di.flGraphicsCaps = GCAPS_OPAQUERECT;
    di.lfDefaultFont = FontSystem;
    di.lfAnsiVarFont = FontHelv;
    di.lfAnsiFixFont = FontCour;
    di.cFonts = 0;
    di.iDitherFormat = BMF_32BPP;
    di.cxDither = 0;
    di.cyDither = 0;
    ppdev->hpal = EngCreatePalette(PAL_BITFIELDS, 0, NULL, 0x00FF0000, 0x0000FF00, 0x000000FF);
    if (!ppdev->hpal) {
        EngFreeMem(modes);
        EngFreeMem(ppdev);
        return NULL;
    }
    di.hpalDefault = ppdev->hpal;
    memcpy(pdi, &di, cjDevInfo < sizeof(di) ? cjDevInfo : sizeof(di));
    EngFreeMem(modes);
    return (DHPDEV)ppdev;
}

VOID APIENTRY DrvCompletePDEV(DHPDEV dhpdev, HDEV hdev) {
    ((PPDEV)dhpdev)->hdev = hdev;
}

VOID APIENTRY DrvDisablePDEV(DHPDEV dhpdev) {
    PPDEV ppdev = (PPDEV)dhpdev;
    if (ppdev->hpal) EngDeletePalette(ppdev->hpal);
    EngFreeMem(ppdev);
}

HSURF APIENTRY DrvEnableSurface(DHPDEV dhpdev) {
    PPDEV ppdev = (PPDEV)dhpdev;
    VIDEO_MODE vm;
    DWORD ret = 0;
    ULONG size = ppdev->cx * ppdev->cy * 4;
    SIZEL sizl;
    HBITMAP hbm;
    vm.RequestedMode = ppdev->mode;
    if (EngDeviceIoControl(ppdev->hDriver, IOCTL_VIDEO_SET_CURRENT_MODE, &vm, sizeof(vm), NULL, 0, &ret)) return NULL;
    if (EngDeviceIoControl(ppdev->hDriver, IOCTL_LGXP_ALLOC_FB, &size, sizeof(size), &ppdev->fb, sizeof(ppdev->fb), &ret) || !ppdev->fb) return NULL;
    sizl.cx = ppdev->cx;
    sizl.cy = ppdev->cy;
    hbm = EngCreateBitmap(sizl, ppdev->cx * 4, BMF_32BPP, BMF_TOPDOWN, ppdev->fb);
    if (!hbm) {
        EngDeviceIoControl(ppdev->hDriver, IOCTL_LGXP_FREE_FB, &ppdev->fb, sizeof(ppdev->fb), NULL, 0, &ret);
        ppdev->fb = NULL;
        return NULL;
    }
    if (!EngAssociateSurface((HSURF)hbm, ppdev->hdev, 0)) {
        EngDeleteSurface((HSURF)hbm);
        EngDeviceIoControl(ppdev->hDriver, IOCTL_LGXP_FREE_FB, &ppdev->fb, sizeof(ppdev->fb), NULL, 0, &ret);
        ppdev->fb = NULL;
        return NULL;
    }
    ppdev->hsurf = (HSURF)hbm;
    return ppdev->hsurf;
}

VOID APIENTRY DrvDisableSurface(DHPDEV dhpdev) {
    PPDEV ppdev = (PPDEV)dhpdev;
    DWORD ret = 0;
    if (ppdev->hsurf) EngDeleteSurface(ppdev->hsurf);
    ppdev->hsurf = NULL;
    if (ppdev->fb) EngDeviceIoControl(ppdev->hDriver, IOCTL_LGXP_FREE_FB, &ppdev->fb, sizeof(ppdev->fb), NULL, 0, &ret);
    ppdev->fb = NULL;
}

BOOL APIENTRY DrvAssertMode(DHPDEV dhpdev, BOOL bEnable) {
    (void)dhpdev;
    (void)bEnable;
    return TRUE;
}

ULONG APIENTRY DrvGetModes(HANDLE hDriver, ULONG cjSize, DEVMODEW *pdm) {
    VIDEO_MODE_INFORMATION *modes;
    ULONG n = QueryModes(hDriver, &modes), i, out = 0;
    if (n == 0) return 0;
    if (pdm == NULL) {
        EngFreeMem(modes);
        return n * sizeof(DEVMODEW);
    }
    for (i = 0; i < n && (out + 1) * sizeof(DEVMODEW) <= cjSize; i++, out++) {
        DEVMODEW *d = &pdm[out];
        memset(d, 0, sizeof(*d));
        CopyName(d->dmDeviceName, L"lgxpdd", CCHDEVICENAME);
        d->dmSpecVersion = DM_SPECVERSION;
        d->dmDriverVersion = DM_SPECVERSION;
        d->dmSize = sizeof(DEVMODEW);
        d->dmBitsPerPel = 32;
        d->dmPelsWidth = modes[i].VisScreenWidth;
        d->dmPelsHeight = modes[i].VisScreenHeight;
        d->dmDisplayFrequency = modes[i].Frequency;
        d->dmFields = DM_BITSPERPEL | DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_DISPLAYFLAGS;
    }
    EngFreeMem(modes);
    return out * sizeof(DEVMODEW);
}

ULONG APIENTRY DrvSetPointerShape(SURFOBJ *pso, SURFOBJ *psoMask, SURFOBJ *psoColor, XLATEOBJ *pxlo, LONG xHot, LONG yHot, LONG x, LONG y, RECTL *prcl, FLONG fl) {
    (void)pso;
    (void)psoMask;
    (void)psoColor;
    (void)pxlo;
    (void)xHot;
    (void)yHot;
    (void)x;
    (void)y;
    (void)prcl;
    (void)fl;
    return SPS_ACCEPT_NOEXCLUDE;
}

VOID APIENTRY DrvMovePointer(SURFOBJ *pso, LONG x, LONG y, RECTL *prcl) {
    (void)pso;
    (void)x;
    (void)y;
    (void)prcl;
}

static DRVFN Funcs[] = {
    {INDEX_DrvEnablePDEV, (PFN)DrvEnablePDEV},
    {INDEX_DrvCompletePDEV, (PFN)DrvCompletePDEV},
    {INDEX_DrvDisablePDEV, (PFN)DrvDisablePDEV},
    {INDEX_DrvEnableSurface, (PFN)DrvEnableSurface},
    {INDEX_DrvDisableSurface, (PFN)DrvDisableSurface},
    {INDEX_DrvAssertMode, (PFN)DrvAssertMode},
    {INDEX_DrvGetModes, (PFN)DrvGetModes},
    {INDEX_DrvSetPointerShape, (PFN)DrvSetPointerShape},
    {INDEX_DrvMovePointer, (PFN)DrvMovePointer},
};

/* 映像入口：GDI 加载后直接调它 */
BOOL APIENTRY DrvEnableDriver(ULONG iEngineVersion, ULONG cj, DRVENABLEDATA *pded) {
    (void)iEngineVersion;
    if (cj < sizeof(DRVENABLEDATA)) return FALSE;
    pded->iDriverVersion = DDI_DRIVER_VERSION_NT5;
    pded->c = sizeof(Funcs) / sizeof(Funcs[0]);
    pded->pdrvfn = Funcs;
    return TRUE;
}
