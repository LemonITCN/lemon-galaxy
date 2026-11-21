// LemonGalaxyDisplayTest：单独验证 LemonGalaxy 虚拟屏驱动（不依赖 worker）。
//
// 默认流程：加一块 1920x1080@60 的屏 -> 只把这条路径扩展进桌面 -> 报 GDI 名、显示器名和 DXGI 输出
// -> 用 DXGI 桌面复制抓一帧 -> 切到 2560x1440@60 再抓一帧（再试 3840x2160@144、1920x1200@120）
// -> 删屏并恢复原显示配置 -> 验证 4 块上限、第 5 块被拒、句柄关闭后驱动自动删屏。
//
// 要在控制台会话里以管理员身份运行（SSH 会话看不到桌面），例如计划任务 schtasks /it /rl highest。
// 是 Windows 子系统程序，不弹控制台窗口；结果写进 --out 指定的文件（从命令行运行时也打到控制台）。
//   LemonGalaxyDisplayTest.exe [--out <file>] [--info]
//     --info  只报驱动状态和当前显示配置，不加屏（SSH 里也能跑）
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <cfgmgr32.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <shellapi.h>
#include <wrl/client.h>

#include <algorithm>
#include <cstdarg>
#include <cstdio>
#include <set>
#include <string>
#include <tuple>
#include <vector>

#include "LemonGalaxyDisplayControl.h"

using Microsoft::WRL::ComPtr;

namespace {

FILE* g_out = nullptr;
bool g_console = false;
int g_failures = 0;

void Print(const char* format, ...) {
    char text[2048];
    va_list args;
    va_start(args, format);
    vsnprintf(text, sizeof(text), format, args);
    va_end(args);
    SYSTEMTIME now;
    GetLocalTime(&now);
    char line[2200];
    snprintf(line, sizeof(line), "%02u:%02u:%02u.%03u %s\n", now.wHour, now.wMinute, now.wSecond, now.wMilliseconds,
             text);
    if (g_out) {
        fputs(line, g_out);
        fflush(g_out);
    }
    if (g_console) {
        fputs(line, stdout);
        fflush(stdout);
    }
}

void Check(bool ok, const char* what) {
    Print("[%s] %s", ok ? "PASS" : "FAIL", what);
    if (!ok) ++g_failures;
}

std::string Utf8(const wchar_t* text) {
    if (!text || !*text) return {};
    const int bytes = WideCharToMultiByte(CP_UTF8, 0, text, -1, nullptr, 0, nullptr, nullptr);
    std::string out(bytes > 0 ? bytes - 1 : 0, '\0');
    if (bytes > 1) WideCharToMultiByte(CP_UTF8, 0, text, -1, out.data(), bytes, nullptr, nullptr);
    return out;
}

bool SameLuid(const LUID& a, const LUID& b) { return a.LowPart == b.LowPart && a.HighPart == b.HighPart; }

// MARK: - Driver

std::wstring InterfacePath() {
    GUID guid = LG_DISPLAY_INTERFACE_GUID_INIT;
    ULONG length = 0;
    if (CM_Get_Device_Interface_List_SizeW(&length, &guid, nullptr, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) !=
            CR_SUCCESS ||
        length <= 1) {
        return {};
    }
    std::vector<wchar_t> list(length);
    if (CM_Get_Device_Interface_ListW(&guid, nullptr, list.data(), length, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) !=
        CR_SUCCESS) {
        return {};
    }
    return std::wstring(list.data());
}

HANDLE OpenDriver() {
    const std::wstring path = InterfacePath();
    if (path.empty()) {
        Print("driver interface not found (driver not installed or device not started)");
        return INVALID_HANDLE_VALUE;
    }
    HANDLE handle = CreateFileW(path.c_str(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                                nullptr, OPEN_EXISTING, 0, nullptr);
    if (handle == INVALID_HANDLE_VALUE) Print("open %s failed: %lu", Utf8(path.c_str()).c_str(), GetLastError());
    return handle;
}

bool GetInfo(HANDLE handle, LgDisplayInfo* info) {
    DWORD returned = 0;
    *info = {};
    return DeviceIoControl(handle, IOCTL_LG_DISPLAY_GET_INFO, nullptr, 0, info, sizeof(*info), &returned, nullptr) &&
           returned == sizeof(*info);
}

void PrintInfo(HANDLE handle) {
    LgDisplayInfo info;
    if (GetInfo(handle, &info)) {
        Print("driver info: version %u, max %u monitors, %u active, adapter ready %u", info.version, info.maxMonitors,
              info.activeMonitors, info.adapterReady);
    } else {
        Print("GET_INFO failed: %lu", GetLastError());
    }
}

// 常见 16:9 / 16:10 分辨率 x 30/60/120/144 Hz，首选模式排第一。
std::vector<LgDisplayMode> CommonModes(LgDisplayMode preferred) {
    static const uint32_t sizes[][2] = {{1280, 720},  {1366, 768},  {1600, 900},  {1920, 1080}, {2560, 1440},
                                        {3200, 1800}, {3840, 2160}, {1280, 800},  {1440, 900},  {1680, 1050},
                                        {1920, 1200}, {2560, 1600}};
    static const uint32_t rates[] = {30, 60, 120, 144};
    std::vector<LgDisplayMode> modes{preferred};
    for (const auto& s : sizes) {
        for (uint32_t r : rates) {
            if (s[0] == preferred.width && s[1] == preferred.height && r == preferred.refreshRate) continue;
            if (modes.size() < LG_DISPLAY_MAX_MODES) modes.push_back({s[0], s[1], r});
        }
    }
    return modes;
}

bool AddMonitor(HANDLE handle, uint32_t connector, uint32_t serial, LgDisplayMode preferred, LgDisplayAddResult* result,
                DWORD* error) {
    LgDisplayAddRequest request = {};
    request.version = LG_DISPLAY_INTERFACE_VERSION;
    request.connector = connector;
    request.serial = serial;
    request.widthMm = 527;  // 约 24 英寸 16:9，Windows 按它算默认缩放
    request.heightMm = 296;
    wcsncpy_s(request.name, L"LemonGalaxy", _TRUNCATE);
    const std::vector<LgDisplayMode> modes = CommonModes(preferred);
    request.modeCount = static_cast<uint32_t>(modes.size());
    request.preferredMode = 0;
    std::copy(modes.begin(), modes.end(), request.modes);
    DWORD returned = 0;
    BOOL ok = FALSE;
    // 驱动刚启动时适配器还没就绪，稍等重试
    for (int attempt = 0; attempt < 30; ++attempt) {
        ok = DeviceIoControl(handle, IOCTL_LG_DISPLAY_ADD, &request, sizeof(request), result, sizeof(*result), &returned,
                             nullptr);
        if (ok || GetLastError() != ERROR_NOT_READY) break;
        Sleep(100);
    }
    *error = ok ? 0 : GetLastError();
    return ok != FALSE;
}

bool RemoveMonitor(HANDLE handle, uint32_t connector) {
    LgDisplayRemoveRequest request = {LG_DISPLAY_INTERFACE_VERSION, connector};
    DWORD returned = 0;
    return DeviceIoControl(handle, IOCTL_LG_DISPLAY_REMOVE, &request, sizeof(request), nullptr, 0, &returned, nullptr) !=
           FALSE;
}

// MARK: - Display configuration

struct Config {
    std::vector<DISPLAYCONFIG_PATH_INFO> paths;
    std::vector<DISPLAYCONFIG_MODE_INFO> modes;
};

bool Query(UINT32 flags, Config* config) {
    for (int attempt = 0; attempt < 5; ++attempt) {
        UINT32 pathCount = 0, modeCount = 0;
        if (GetDisplayConfigBufferSizes(flags, &pathCount, &modeCount) != ERROR_SUCCESS) return false;
        config->paths.resize(pathCount);
        config->modes.resize(modeCount);
        const LONG rc =
            QueryDisplayConfig(flags, &pathCount, config->paths.data(), &modeCount, config->modes.data(), nullptr);
        if (rc == ERROR_INSUFFICIENT_BUFFER) continue;
        if (rc != ERROR_SUCCESS) return false;
        config->paths.resize(pathCount);
        config->modes.resize(modeCount);
        return true;
    }
    return false;
}

std::wstring SourceName(const DISPLAYCONFIG_PATH_INFO& path) {
    DISPLAYCONFIG_SOURCE_DEVICE_NAME source = {};
    source.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    source.header.size = sizeof(source);
    source.header.adapterId = path.sourceInfo.adapterId;
    source.header.id = path.sourceInfo.id;
    if (DisplayConfigGetDeviceInfo(&source.header) != ERROR_SUCCESS) return {};
    return source.viewGdiDeviceName;
}

DISPLAYCONFIG_TARGET_DEVICE_NAME TargetName(const DISPLAYCONFIG_PATH_INFO& path) {
    DISPLAYCONFIG_TARGET_DEVICE_NAME target = {};
    target.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
    target.header.size = sizeof(target);
    target.header.adapterId = path.targetInfo.adapterId;
    target.header.id = path.targetInfo.id;
    DisplayConfigGetDeviceInfo(&target.header);
    return target;
}

// 活动路径的摘要，用来比较“恢复后是不是和原来一样”。
using PathKey = std::tuple<std::wstring, std::wstring, LONG, LONG, UINT32, UINT32>;

std::set<PathKey> Summary(const Config& config) {
    std::set<PathKey> keys;
    for (const DISPLAYCONFIG_PATH_INFO& path : config.paths) {
        if (!(path.flags & DISPLAYCONFIG_PATH_ACTIVE)) continue;
        LONG x = 0, y = 0;
        UINT32 w = 0, h = 0;
        const UINT32 idx = path.sourceInfo.modeInfoIdx;
        if (idx < config.modes.size() && config.modes[idx].infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE) {
            const DISPLAYCONFIG_SOURCE_MODE& m = config.modes[idx].sourceMode;
            x = m.position.x;
            y = m.position.y;
            w = m.width;
            h = m.height;
        }
        keys.insert({SourceName(path), TargetName(path).monitorDevicePath, x, y, w, h});
    }
    return keys;
}

void PrintConfig(const char* title, const Config& config) {
    Print("%s: %zu active path(s)", title, Summary(config).size());
    for (const PathKey& key : Summary(config)) {
        Print("  %s  %ux%u at (%ld,%ld)  %s", Utf8(std::get<0>(key).c_str()).c_str(), std::get<4>(key), std::get<5>(key),
              std::get<2>(key), std::get<3>(key), Utf8(std::get<1>(key).c_str()).c_str());
    }
}

bool FindPath(UINT32 flags, LUID adapter, UINT32 targetId, DISPLAYCONFIG_PATH_INFO* found) {
    Config config;
    if (!Query(flags, &config)) return false;
    for (const DISPLAYCONFIG_PATH_INFO& path : config.paths) {
        if (path.targetInfo.id == targetId && SameLuid(path.targetInfo.adapterId, adapter)) {
            if ((flags & QDC_ONLY_ACTIVE_PATHS) && !(path.flags & DISPLAYCONFIG_PATH_ACTIVE)) continue;
            *found = path;
            return true;
        }
    }
    return false;
}

// 只把这块屏的路径加进现有拓扑（扩展），其它屏的配置原样保留；不写进显示数据库。
LONG Activate(LUID adapter, UINT32 targetId) {
    Config all;
    if (!Query(QDC_ALL_PATHS, &all)) return -1;
    std::vector<DISPLAYCONFIG_PATH_INFO> next;
    for (const DISPLAYCONFIG_PATH_INFO& path : all.paths) {
        if (path.flags & DISPLAYCONFIG_PATH_ACTIVE) {
            if (path.targetInfo.id == targetId && SameLuid(path.targetInfo.adapterId, adapter)) return ERROR_SUCCESS;
            next.push_back(path);
        }
    }
    const DISPLAYCONFIG_PATH_INFO* candidate = nullptr;
    for (const DISPLAYCONFIG_PATH_INFO& path : all.paths) {
        if (path.targetInfo.id != targetId || !SameLuid(path.targetInfo.adapterId, adapter)) continue;
        if (!path.targetInfo.targetAvailable) continue;
        const bool sourceUsed = std::any_of(next.begin(), next.end(), [&](const DISPLAYCONFIG_PATH_INFO& active) {
            return active.sourceInfo.id == path.sourceInfo.id &&
                   SameLuid(active.sourceInfo.adapterId, path.sourceInfo.adapterId);
        });
        if (!sourceUsed) {
            candidate = &path;
            break;
        }
    }
    if (!candidate) return ERROR_NOT_FOUND;
    DISPLAYCONFIG_PATH_INFO added = *candidate;
    added.flags = DISPLAYCONFIG_PATH_ACTIVE;
    added.sourceInfo.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
    added.targetInfo.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
    next.push_back(added);
    return SetDisplayConfig(static_cast<UINT32>(next.size()), next.data(), static_cast<UINT32>(all.modes.size()),
                            all.modes.data(), SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES);
}

template <typename F>
bool WaitFor(DWORD timeoutMs, F condition) {
    const ULONGLONG end = GetTickCount64() + timeoutMs;
    for (;;) {
        if (condition()) return true;
        if (GetTickCount64() >= end) return false;
        Sleep(100);
    }
}

bool CurrentMode(const std::wstring& name, DEVMODEW* mode) {
    *mode = {};
    mode->dmSize = sizeof(*mode);
    return EnumDisplaySettingsExW(name.c_str(), ENUM_CURRENT_SETTINGS, mode, 0) != FALSE;
}

void PrintModeList(const std::wstring& name) {
    std::set<std::tuple<DWORD, DWORD, DWORD>> modes;
    DEVMODEW mode = {};
    mode.dmSize = sizeof(mode);
    for (DWORD i = 0; EnumDisplaySettingsExW(name.c_str(), i, &mode, 0); ++i) {
        modes.insert({mode.dmPelsWidth, mode.dmPelsHeight, mode.dmDisplayFrequency});
    }
    std::set<std::pair<DWORD, DWORD>> sizes;
    std::set<DWORD> rates;
    for (const auto& m : modes) {
        sizes.insert({std::get<0>(m), std::get<1>(m)});
        rates.insert(std::get<2>(m));
    }
    std::string sizeText, rateText;
    for (const auto& s : sizes) sizeText += std::to_string(s.first) + "x" + std::to_string(s.second) + " ";
    for (DWORD r : rates) rateText += std::to_string(r) + " ";
    Print("modes offered by Windows: %zu (w x h x Hz); sizes: %s; rates: %s", modes.size(), sizeText.c_str(),
          rateText.c_str());
    Check(modes.count({3840u, 2160u, 144u}) && modes.count({2560u, 1600u, 120u}) && modes.count({1920u, 1080u, 30u}),
          "mode list includes 3840x2160@144, 2560x1600@120, 1920x1080@30");
}

LONG SetMode(const std::wstring& name, DWORD width, DWORD height, DWORD hz) {
    DEVMODEW mode = {};
    mode.dmSize = sizeof(mode);
    mode.dmPelsWidth = width;
    mode.dmPelsHeight = height;
    mode.dmDisplayFrequency = hz;
    mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
    // 0 = 只改当前，不写注册表
    return ChangeDisplaySettingsExW(name.c_str(), &mode, nullptr, 0, nullptr);
}

// MARK: - DXGI

struct Output {
    ComPtr<IDXGIAdapter1> adapter;
    ComPtr<IDXGIOutput1> output;
    DXGI_ADAPTER_DESC1 adapterDesc{};
    DXGI_OUTPUT_DESC outputDesc{};
};

bool FindOutput(const std::wstring& name, Output* found) {
    ComPtr<IDXGIFactory1> factory;
    if (FAILED(CreateDXGIFactory1(IID_PPV_ARGS(&factory)))) return false;
    ComPtr<IDXGIAdapter1> adapter;
    for (UINT a = 0; factory->EnumAdapters1(a, &adapter) != DXGI_ERROR_NOT_FOUND; ++a) {
        ComPtr<IDXGIOutput> output;
        for (UINT o = 0; adapter->EnumOutputs(o, &output) != DXGI_ERROR_NOT_FOUND; ++o) {
            DXGI_OUTPUT_DESC desc;
            if (SUCCEEDED(output->GetDesc(&desc)) && _wcsicmp(desc.DeviceName, name.c_str()) == 0) {
                found->adapter = adapter;
                output.As(&found->output);
                adapter->GetDesc1(&found->adapterDesc);
                found->outputDesc = desc;
                return found->output != nullptr;
            }
            output.Reset();
        }
        adapter.Reset();
    }
    return false;
}

// 抓一帧：遇到 ACCESS_LOST（改模式后第一次常见）就重建复制会话再试。
bool GrabFrame(const std::wstring& name, const char* label) {
    for (int attempt = 1; attempt <= 6; ++attempt) {
        Output out;
        if (!FindOutput(name, &out)) {
            Print("%s: DXGI output %s not found (attempt %d)", label, Utf8(name.c_str()).c_str(), attempt);
            Sleep(300);
            continue;
        }
        ComPtr<ID3D11Device> device;
        ComPtr<ID3D11DeviceContext> context;
        HRESULT hr = D3D11CreateDevice(out.adapter.Get(), D3D_DRIVER_TYPE_UNKNOWN, nullptr, 0, nullptr, 0,
                                       D3D11_SDK_VERSION, &device, nullptr, &context);
        if (FAILED(hr)) {
            Print("%s: D3D11CreateDevice failed 0x%08lX", label, hr);
            return false;
        }
        ComPtr<IDXGIOutputDuplication> duplication;
        hr = out.output->DuplicateOutput(device.Get(), &duplication);
        if (FAILED(hr)) {
            Print("%s: DuplicateOutput failed 0x%08lX (attempt %d)", label, hr, attempt);
            Sleep(300);
            continue;
        }
        for (int frameTry = 0; frameTry < 5; ++frameTry) {
            DXGI_OUTDUPL_FRAME_INFO info = {};
            ComPtr<IDXGIResource> resource;
            const ULONGLONG start = GetTickCount64();
            hr = duplication->AcquireNextFrame(1000, &info, &resource);
            if (hr == DXGI_ERROR_WAIT_TIMEOUT) continue;
            if (hr == DXGI_ERROR_ACCESS_LOST) {
                Print("%s: AcquireNextFrame ACCESS_LOST (attempt %d), recreating duplication", label, attempt);
                break;
            }
            if (FAILED(hr)) {
                Print("%s: AcquireNextFrame failed 0x%08lX", label, hr);
                break;
            }
            if (info.LastPresentTime.QuadPart == 0) {
                duplication->ReleaseFrame();
                continue;  // 只有光标变化，没有画面
            }
            ComPtr<ID3D11Texture2D> texture;
            resource.As(&texture);
            D3D11_TEXTURE2D_DESC desc = {};
            texture->GetDesc(&desc);
            D3D11_TEXTURE2D_DESC stagingDesc = desc;
            stagingDesc.Usage = D3D11_USAGE_STAGING;
            stagingDesc.BindFlags = 0;
            stagingDesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            stagingDesc.MiscFlags = 0;
            stagingDesc.MipLevels = 1;
            stagingDesc.ArraySize = 1;
            ComPtr<ID3D11Texture2D> staging;
            double meanB = 0, meanG = 0, meanR = 0, nonBlack = 0;
            uint64_t samples = 0;
            if (SUCCEEDED(device->CreateTexture2D(&stagingDesc, nullptr, &staging))) {
                context->CopyResource(staging.Get(), texture.Get());
                D3D11_MAPPED_SUBRESOURCE mapped = {};
                if (SUCCEEDED(context->Map(staging.Get(), 0, D3D11_MAP_READ, 0, &mapped))) {
                    for (UINT y = 0; y < desc.Height; y += 8) {
                        const uint8_t* row = static_cast<const uint8_t*>(mapped.pData) + static_cast<size_t>(y) * mapped.RowPitch;
                        for (UINT x = 0; x < desc.Width; x += 8) {
                            const uint8_t* p = row + x * 4;
                            meanB += p[0];
                            meanG += p[1];
                            meanR += p[2];
                            if (p[0] > 8 || p[1] > 8 || p[2] > 8) nonBlack += 1;
                            ++samples;
                        }
                    }
                    context->Unmap(staging.Get(), 0);
                }
            }
            duplication->ReleaseFrame();
            if (samples) {
                meanB /= samples;
                meanG /= samples;
                meanR /= samples;
                nonBlack = nonBlack * 100.0 / samples;
            }
            Print("%s: frame %ux%u format %u from %s (adapter LUID %08lX:%08lX), waited %llu ms, attempt %d, "
                  "mean BGR (%.0f,%.0f,%.0f), non-black %.1f%%",
                  label, desc.Width, desc.Height, desc.Format, Utf8(out.adapterDesc.Description).c_str(),
                  out.adapterDesc.AdapterLuid.HighPart, out.adapterDesc.AdapterLuid.LowPart, GetTickCount64() - start,
                  attempt, meanB, meanG, meanR, nonBlack);
            return true;
        }
        Sleep(200);
    }
    return false;
}

// MARK: - Test

void RestoreIfChanged(const Config& original) {
    Config now;
    if (!Query(QDC_ONLY_ACTIVE_PATHS, &now)) {
        Print("QueryDisplayConfig failed while restoring");
        return;
    }
    if (Summary(now) == Summary(original)) {
        Print("display configuration already equals the original");
        return;
    }
    PrintConfig("configuration differs, restoring; current", now);
    const LONG rc = SetDisplayConfig(static_cast<UINT32>(original.paths.size()),
                                     const_cast<DISPLAYCONFIG_PATH_INFO*>(original.paths.data()),
                                     static_cast<UINT32>(original.modes.size()),
                                     const_cast<DISPLAYCONFIG_MODE_INFO*>(original.modes.data()),
                                     SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES);
    Print("SetDisplayConfig(original): %ld", rc);
}

void MainTest(const Config& original) {
    HANDLE handle = OpenDriver();
    Check(handle != INVALID_HANDLE_VALUE, "open driver control interface");
    if (handle == INVALID_HANDLE_VALUE) return;
    PrintInfo(handle);

    LgDisplayAddResult result = {};
    DWORD error = 0;
    const ULONGLONG addStart = GetTickCount64();
    const bool added = AddMonitor(handle, 0, 0x4C470001, {1920, 1080, 60}, &result, &error);
    Check(added, "ADD connector 0, 1920x1080@60 preferred (48 modes)");
    if (!added) {
        Print("ADD failed: %lu", error);
        CloseHandle(handle);
        return;
    }
    LUID adapter = {result.adapterLuidLow, result.adapterLuidHigh};
    Print("added: target id %u, adapter LUID %08lX:%08lX, took %llu ms", result.targetId, adapter.HighPart,
          adapter.LowPart, GetTickCount64() - addStart);

    DISPLAYCONFIG_PATH_INFO path = {};
    const bool listed = WaitFor(5000, [&] { return FindPath(QDC_ALL_PATHS, adapter, result.targetId, &path); });
    Check(listed, "new target appears in QueryDisplayConfig(QDC_ALL_PATHS)");
    const bool autoActive = FindPath(QDC_ONLY_ACTIVE_PATHS, adapter, result.targetId, &path);
    Print("target active right after ADD (before SetDisplayConfig): %s", autoActive ? "yes" : "no");

    const ULONGLONG activateStart = GetTickCount64();
    const LONG rc = Activate(adapter, result.targetId);
    Print("SetDisplayConfig(extend with only this path): %ld", rc);
    const bool active =
        WaitFor(5000, [&] { return FindPath(QDC_ONLY_ACTIVE_PATHS, adapter, result.targetId, &path); });
    Check(rc == ERROR_SUCCESS && active, "monitor attached to the desktop");
    std::wstring name;
    if (active) {
        Print("activation took %llu ms", GetTickCount64() - activateStart);
        name = SourceName(path);
        const DISPLAYCONFIG_TARGET_DEVICE_NAME target = TargetName(path);
        Print("GDI name %s, monitor \"%s\", EDID manufacturer 0x%04X product 0x%04X, output technology %d",
              Utf8(name.c_str()).c_str(), Utf8(target.monitorFriendlyDeviceName).c_str(), target.edidManufactureId,
              target.edidProductCodeId, target.outputTechnology);
        Print("monitor device path %s", Utf8(target.monitorDevicePath).c_str());
        Config now;
        if (Query(QDC_ONLY_ACTIVE_PATHS, &now)) PrintConfig("configuration with virtual monitor", now);
        DEVMODEW mode;
        if (CurrentMode(name, &mode)) {
            Print("current mode %lux%lu@%lu at (%ld,%ld)", mode.dmPelsWidth, mode.dmPelsHeight, mode.dmDisplayFrequency,
                  mode.dmPosition.x, mode.dmPosition.y);
            Check(mode.dmPelsWidth == 1920 && mode.dmPelsHeight == 1080 && mode.dmDisplayFrequency == 60,
                  "initial mode is 1920x1080@60");
        }
        Output out;
        if (FindOutput(name, &out)) {
            const RECT& r = out.outputDesc.DesktopCoordinates;
            Print("DXGI output %s on adapter \"%s\" (LUID %08lX:%08lX), desktop (%ld,%ld)-(%ld,%ld), attached %d",
                  Utf8(out.outputDesc.DeviceName).c_str(), Utf8(out.adapterDesc.Description).c_str(),
                  out.adapterDesc.AdapterLuid.HighPart, out.adapterDesc.AdapterLuid.LowPart, r.left, r.top, r.right,
                  r.bottom, out.outputDesc.AttachedToDesktop);
            Check(true, "DXGI enumerates the virtual monitor's output");
        } else {
            Check(false, "DXGI enumerates the virtual monitor's output");
        }
        PrintModeList(name);
        Check(GrabFrame(name, "grab @1920x1080"), "DXGI duplication frame at 1920x1080");

        const struct {
            DWORD w, h, hz;
            bool grab;
        } switches[] = {{2560, 1440, 60, true}, {3840, 2160, 144, false}, {1920, 1200, 120, false}};
        for (const auto& s : switches) {
            const ULONGLONG start = GetTickCount64();
            const LONG cds = SetMode(name, s.w, s.h, s.hz);
            DEVMODEW mode2;
            const bool ok = cds == DISP_CHANGE_SUCCESSFUL && CurrentMode(name, &mode2) && mode2.dmPelsWidth == s.w &&
                            mode2.dmPelsHeight == s.h && mode2.dmDisplayFrequency == s.hz;
            char what[128];
            snprintf(what, sizeof(what), "switch to %lux%lu@%lu (ChangeDisplaySettingsEx %ld, %llu ms)", s.w, s.h, s.hz,
                     cds, GetTickCount64() - start);
            Check(ok, what);
            if (s.grab) {
                snprintf(what, sizeof(what), "grab @%lux%lu", s.w, s.h);
                Check(GrabFrame(name, what), "DXGI duplication frame after mode switch");
            }
        }
    }

    const ULONGLONG removeStart = GetTickCount64();
    const bool removed = RemoveMonitor(handle, 0);
    Check(removed, "REMOVE connector 0");
    // 删掉后目标可能还留在 QDC_ALL_PATHS 里，但不再活动、targetAvailable 为 0
    const bool gone = WaitFor(5000, [&] {
        if (FindPath(QDC_ONLY_ACTIVE_PATHS, adapter, result.targetId, &path)) return false;
        return !FindPath(QDC_ALL_PATHS, adapter, result.targetId, &path) || !path.targetInfo.targetAvailable;
    });
    Check(gone, "target no longer active nor available");
    Print("removal took %llu ms", GetTickCount64() - removeStart);
    PrintInfo(handle);
    Sleep(500);
    RestoreIfChanged(original);
    CloseHandle(handle);
}

void LimitTest(const Config& original) {
    HANDLE handle = OpenDriver();
    if (handle == INVALID_HANDLE_VALUE) return;
    int ok = 0;
    for (uint32_t c = 0; c < LG_DISPLAY_MAX_MONITORS; ++c) {
        LgDisplayAddResult result = {};
        DWORD error = 0;
        if (AddMonitor(handle, c, 0x4C470010 + c, {1920, 1080, 60}, &result, &error)) {
            ++ok;
            Print("limit test: connector %u -> target %u", c, result.targetId);
        } else {
            Print("limit test: connector %u failed %lu", c, error);
        }
    }
    LgDisplayInfo info;
    Check(ok == 4 && GetInfo(handle, &info) && info.activeMonitors == 4, "4 monitors at once (not attached to desktop)");
    {
        LgDisplayAddResult result = {};
        DWORD error = 0;
        const bool fifth = AddMonitor(handle, LG_DISPLAY_MAX_MONITORS, 0x4C470020, {1920, 1080, 60}, &result, &error);
        Print("fifth monitor (connector 4): %s, error %lu", fifth ? "accepted" : "rejected", error);
        Check(!fifth && error == ERROR_INVALID_PARAMETER, "fifth monitor rejected");
    }
    Config now;
    if (Query(QDC_ONLY_ACTIVE_PATHS, &now)) {
        Print("active paths with 4 unattached virtual monitors: %zu (original %zu)", Summary(now).size(),
              Summary(original).size());
    }
    // 不发 REMOVE，直接关句柄：驱动应自动删掉这 4 块
    CloseHandle(handle);
    handle = OpenDriver();
    if (handle == INVALID_HANDLE_VALUE) return;
    const bool cleaned = WaitFor(3000, [&] { return GetInfo(handle, &info) && info.activeMonitors == 0; });
    Check(cleaned, "closing the handle removed all its monitors");
    PrintInfo(handle);
    CloseHandle(handle);
    Sleep(500);
    RestoreIfChanged(original);
}

}  // namespace

int WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR, int) {
    int argc = 0;
    LPWSTR* argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    bool infoOnly = false;
    for (int i = 1; i < argc; ++i) {
        if (wcscmp(argv[i], L"--out") == 0 && i + 1 < argc) {
            _wfopen_s(&g_out, argv[++i], L"wb");
        } else if (wcscmp(argv[i], L"--info") == 0) {
            infoOnly = true;
        }
    }
    if (AttachConsole(ATTACH_PARENT_PROCESS)) {
        FILE* f = nullptr;
        freopen_s(&f, "CONOUT$", "w", stdout);
        g_console = true;
    }
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

    // 兜底：卡住超过 90 秒就退出；句柄随进程关闭，驱动会自动删屏。
    CreateThread(
        nullptr, 0,
        [](LPVOID) -> DWORD {
            Sleep(90000);
            Print("watchdog: test took too long, exiting (driver removes the monitors with the handle)");
            ExitProcess(3);
        },
        nullptr, 0, nullptr);

    DWORD sessionId = 0;
    ProcessIdToSessionId(GetCurrentProcessId(), &sessionId);
    HDESK desk = OpenInputDesktop(0, FALSE, GENERIC_READ);
    Print("LemonGalaxyDisplayTest: session %lu, input desktop %s", sessionId, desk ? "accessible" : "not accessible");
    if (desk) CloseDesktop(desk);

    Config original;
    const bool haveOriginal = Query(QDC_ONLY_ACTIVE_PATHS, &original);
    if (haveOriginal) {
        PrintConfig("original configuration", original);
    } else {
        Print("QueryDisplayConfig failed (not in an interactive session?)");
    }

    if (infoOnly) {
        HANDLE handle = OpenDriver();
        if (handle != INVALID_HANDLE_VALUE) {
            Print("driver interface %s", Utf8(InterfacePath().c_str()).c_str());
            PrintInfo(handle);
            CloseHandle(handle);
        }
        return handle == INVALID_HANDLE_VALUE ? 1 : 0;
    }
    if (!haveOriginal) return 2;  // 拿不到原配置就无法恢复，不做改动

    Print("interface %s", Utf8(InterfacePath().c_str()).c_str());
    MainTest(original);
    LimitTest(original);

    Config last;
    if (Query(QDC_ONLY_ACTIVE_PATHS, &last)) {
        PrintConfig("final configuration", last);
        Check(Summary(last) == Summary(original), "final display configuration equals the original");
    }
    Print("done: %d failure(s)", g_failures);
    if (g_out) fclose(g_out);
    return g_failures ? 1 : 0;
}
