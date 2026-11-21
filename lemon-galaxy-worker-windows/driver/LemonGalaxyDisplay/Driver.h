// LemonGalaxy virtual display driver (IddCx, user mode).
//
// One adapter that starts without monitors. The worker adds up to four monitors (three on demand
// for controllers, one for headless hosts) through the control interface
// (LemonGalaxyDisplayControl.h) and removes them again; monitors of a handle that closes are removed
// automatically. Frames are consumed and released right away: the worker reads the picture with
// DXGI desktop duplication, not from here.
//
// 移植自 LemonLink 的 LemonLinkDisplay（同一作者），改了名字、接口 GUID、硬件 ID 与屏数上限。
// 结构参考了微软 Windows-driver-samples 的 IndirectDisplay 示例。
#pragma once

#define NOMINMAX
#include <windows.h>
#include <bugcodes.h>
#include <wudfwdm.h>
#include <wdf.h>
#include <iddcx.h>

#include <avrt.h>
#include <d3d11_2.h>
#include <dxgi1_5.h>
#include <wrl.h>

#include <array>
#include <memory>
#include <mutex>
#include <vector>

#include "Edid.h"
#include "LemonGalaxyDisplayControl.h"

namespace lg::display {

void Log(const wchar_t* format, ...);

// D3D device on the GPU the OS renders this monitor with.
struct Direct3DDevice {
    explicit Direct3DDevice(LUID adapterLuid) : adapterLuid(adapterLuid) {}
    HRESULT Init();

    LUID adapterLuid{};
    Microsoft::WRL::ComPtr<IDXGIFactory5> factory;
    Microsoft::WRL::ComPtr<IDXGIAdapter1> adapter;
    Microsoft::WRL::ComPtr<ID3D11Device> device;
    Microsoft::WRL::ComPtr<ID3D11DeviceContext> context;
};

// Takes every frame the OS hands this monitor and gives it straight back.
class SwapChainProcessor {
public:
    SwapChainProcessor(IDDCX_SWAPCHAIN swapChain, std::shared_ptr<Direct3DDevice> device, HANDLE newFrameEvent);
    ~SwapChainProcessor();

private:
    static DWORD CALLBACK RunThread(LPVOID argument);
    void Run();
    void RunCore();

    IDDCX_SWAPCHAIN swapChain_;
    std::shared_ptr<Direct3DDevice> device_;
    HANDLE newFrameEvent_;
    Microsoft::WRL::Wrappers::HandleT<Microsoft::WRL::Wrappers::HandleTraits::HANDLENullTraits> thread_;
    Microsoft::WRL::Wrappers::Event terminateEvent_;
};

struct MonitorEntry {
    IDDCX_MONITOR monitor = nullptr;
    WDFFILEOBJECT owner = nullptr;
    std::vector<LgDisplayMode> modes;
    uint32_t preferredMode = 0;
    std::unique_ptr<SwapChainProcessor> processor;
};

class DeviceContext {
public:
    explicit DeviceContext(WDFDEVICE device) : device_(device) {}

    void InitAdapter();
    void AdapterInitFinished(NTSTATUS status);

    LgDisplayInfo Info();
    NTSTATUS AddMonitor(const LgDisplayAddRequest& request, WDFFILEOBJECT owner, LgDisplayAddResult* result);
    NTSTATUS RemoveMonitor(uint32_t connector);
    void RemoveMonitorsOwnedBy(WDFFILEOBJECT owner);

    std::vector<LgDisplayMode> Modes(uint32_t connector);
    void AssignSwapChain(uint32_t connector, IDDCX_SWAPCHAIN swapChain, LUID renderAdapter, HANDLE newFrameEvent);
    void UnassignSwapChain(uint32_t connector);

private:
    WDFDEVICE device_;
    IDDCX_ADAPTER adapter_ = nullptr;
    bool adapterReady_ = false;
    std::mutex mutex_;
    std::array<MonitorEntry, LG_DISPLAY_MAX_MONITORS> monitors_;
};

}  // namespace lg::display

// Context of the WDF device (owner) and of the IddCx adapter (a second pointer to the same object).
struct DeviceContextWrapper {
    lg::display::DeviceContext* context;
    bool owner;
};

struct MonitorContext {
    lg::display::DeviceContext* device;
    uint32_t connector;
    lg::Edid edid;
};

WDF_DECLARE_CONTEXT_TYPE(DeviceContextWrapper);
WDF_DECLARE_CONTEXT_TYPE(MonitorContext);
