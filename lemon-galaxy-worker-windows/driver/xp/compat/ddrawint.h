/* mingw-w64 的 winddi.h 要 ddrawint.h，但 llvm-mingw 没带；本驱动不支持 DirectDraw，只用到指针，给不透明类型即可。 */
#pragma once
typedef struct _DD_CALLBACKS DD_CALLBACKS;
typedef struct _DD_SURFACECALLBACKS DD_SURFACECALLBACKS;
typedef struct _DD_PALETTECALLBACKS DD_PALETTECALLBACKS;
typedef struct _DD_HALINFO DD_HALINFO;
typedef struct _DD_DIRECTDRAW_GLOBAL DD_DIRECTDRAW_GLOBAL;
typedef struct _DD_SURFACE_LOCAL DD_SURFACE_LOCAL, *PDD_SURFACE_LOCAL;
typedef struct _VIDEOMEMORY VIDEOMEMORY;
