#include "Driver.h"

#include <algorithm>
#include <cstdarg>
#include <cstdio>
#include <cstring>

using Microsoft::WRL::ComPtr;

namespace lg::display {

// MARK: - Logging

// Written to %ProgramData%\LemonGalaxy\Logs\LemonGalaxyDisplay.log when that folder exists and the
// driver host (LOCAL SERVICE) may write there; the installer creates it and grants LOCAL SERVICE.
void Log(const wchar_t* format, ...) {
    static std::mutex mutex;
    wchar_t message[512];
    va_list args;
    va_start(args, format);
    vswprintf_s(message, format, args);
    va_end(args);

    SYSTEMTIME now;
    GetLocalTime(&now);
    wchar_t line[600];
    swprintf_s(line, L"%02u:%02u:%02u.%03u %s\r\n", now.wHour, now.wMinute, now.wSecond, now.wMilliseconds, message);
    OutputDebugStringW(line);

    std::lock_guard<std::mutex> lock(mutex);
    wchar_t path[MAX_PATH];
    const DWORD length = GetEnvironmentVariableW(L"ProgramData", path, MAX_PATH);
    if (length == 0 || length > MAX_PATH - 48) return;
    wcscat_s(path, L"\\LemonGalaxy\\Logs\\LemonGalaxyDisplay.log");
    HANDLE file = CreateFileW(path, FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_ALWAYS,
                              FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file == INVALID_HANDLE_VALUE) return;
    LARGE_INTEGER size{};
    if (GetFileSizeEx(file, &size) && size.QuadPart > 4 * 1024 * 1024) {
        // Start over rather than grow without bound.
        SetFilePointer(file, 0, nullptr, FILE_BEGIN);
        SetEndOfFile(file);
    }
    char utf8[1200];
    const int bytes = WideCharToMultiByte(CP_UTF8, 0, line, -1, utf8, sizeof(utf8), nullptr, nullptr);
    if (bytes > 1) {
        DWORD written = 0;
        WriteFile(file, utf8, static_cast<DWORD>(bytes - 1), &written, nullptr);
    }
    CloseHandle(file);
}

// MARK: - Mode lists by EDID

// IddCx parses monitor descriptions without telling which monitor they belong to, so the modes of
// each EDID we hand out are kept here until the monitor goes away.
namespace {

struct EdidModes {
    Edid edid{};
    std::vector<LgDisplayMode> modes;
    uint32_t preferred = 0;
};

std::mutex g_edidMutex;
std::vector<EdidModes> g_edidModes;

void RegisterEdid(const Edid& edid, const std::vector<LgDisplayMode>& modes, uint32_t preferred) {
    std::lock_guard<std::mutex> lock(g_edidMutex);
    for (EdidModes& entry : g_edidModes) {
        if (entry.edid == edid) {
            entry.modes = modes;
            entry.preferred = preferred;
            return;
        }
    }
    g_edidModes.push_back(EdidModes{edid, modes, preferred});
}

void UnregisterEdid(const Edid& edid) {
    std::lock_guard<std::mutex> lock(g_edidMutex);
    g_edidModes.erase(std::remove_if(g_edidModes.begin(), g_edidModes.end(),
                                     [&](const EdidModes& entry) { return entry.edid == edid; }),
                      g_edidModes.end());
}

bool FindEdid(const void* data, size_t size, EdidModes* out) {
    if (size != sizeof(Edid)) return false;
    std::lock_guard<std::mutex> lock(g_edidMutex);
    for (const EdidModes& entry : g_edidModes) {
        if (std::memcmp(entry.edid.data(), data, size) == 0) {
            *out = entry;
            return true;
        }
    }
    return false;
}

void FillSignalInfo(DISPLAYCONFIG_VIDEO_SIGNAL_INFO& mode, DWORD width, DWORD height, DWORD vsync, bool monitorMode) {
    mode.totalSize.cx = mode.activeSize.cx = width;
    mode.totalSize.cy = mode.activeSize.cy = height;
    // Monitor modes report the full rate; target modes divide nothing.
    mode.AdditionalSignalInfo.vSyncFreqDivider = monitorMode ? 0 : 1;
    mode.AdditionalSignalInfo.videoStandard = 255;
    mode.vSyncFreq.Numerator = vsync;
    mode.vSyncFreq.Denominator = 1;
    mode.hSyncFreq.Numerator = vsync * height;
    mode.hSyncFreq.Denominator = 1;
    mode.scanLineOrdering = DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE;
    mode.pixelRate = static_cast<UINT64>(vsync) * width * height;
}

IDDCX_MONITOR_MODE MonitorMode(const LgDisplayMode& m, IDDCX_MONITOR_MODE_ORIGIN origin) {
    IDDCX_MONITOR_MODE mode = {};
    mode.Size = sizeof(mode);
    mode.Origin = origin;
    FillSignalInfo(mode.MonitorVideoSignalInfo, m.width, m.height, m.refreshRate, true);
    return mode;
}

IDDCX_TARGET_MODE TargetMode(const LgDisplayMode& m) {
    IDDCX_TARGET_MODE mode = {};
    mode.Size = sizeof(mode);
    FillSignalInfo(mode.TargetVideoSignalInfo.targetVideoSignalInfo, m.width, m.height, m.refreshRate, false);
    return mode;
}

// Same serial and slot, same container: Windows groups the monitor with it across sessions.
GUID ContainerId(uint32_t serial, uint32_t connector) {
    GUID id = {0x15278058, 0x6500, 0x4477, {0xaa, 0x21, 0, 0, 0, 0, 0, 0}};
    id.Data1 ^= connector;
    id.Data4[2] = static_cast<uint8_t>(serial >> 24);
    id.Data4[3] = static_cast<uint8_t>(serial >> 16);
    id.Data4[4] = static_cast<uint8_t>(serial >> 8);
    id.Data4[5] = static_cast<uint8_t>(serial);
    id.Data4[6] = static_cast<uint8_t>(connector);
    return id;
}

bool ValidMode(const LgDisplayMode& m) {
    return m.width >= 320 && m.width <= 16384 && m.height >= 240 && m.height <= 16384 && m.refreshRate >= 24 &&
           m.refreshRate <= 480;
}

}  // namespace

// MARK: - Direct3D

HRESULT Direct3DDevice::Init() {
    HRESULT hr = CreateDXGIFactory2(0, IID_PPV_ARGS(&factory));
    if (FAILED(hr)) return hr;
    hr = factory->EnumAdapterByLuid(adapterLuid, IID_PPV_ARGS(&adapter));
    if (FAILED(hr)) return hr;
    return D3D11CreateDevice(adapter.Get(), D3D_DRIVER_TYPE_UNKNOWN, nullptr, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                             nullptr, 0, D3D11_SDK_VERSION, &device, nullptr, &context);
}

// MARK: - Swap chain

SwapChainProcessor::SwapChainProcessor(IDDCX_SWAPCHAIN swapChain, std::shared_ptr<Direct3DDevice> device,
                                       HANDLE newFrameEvent)
    : swapChain_(swapChain), device_(std::move(device)), newFrameEvent_(newFrameEvent) {
    terminateEvent_.Attach(CreateEventW(nullptr, FALSE, FALSE, nullptr));
    thread_.Attach(CreateThread(nullptr, 0, RunThread, this, 0, nullptr));
}

SwapChainProcessor::~SwapChainProcessor() {
    SetEvent(terminateEvent_.Get());
    if (thread_.Get()) WaitForSingleObject(thread_.Get(), INFINITE);
}

DWORD CALLBACK SwapChainProcessor::RunThread(LPVOID argument) {
    reinterpret_cast<SwapChainProcessor*>(argument)->Run();
    return 0;
}

void SwapChainProcessor::Run() {
    // The OS expects the swap chain to be serviced promptly.
    DWORD task = 0;
    HANDLE taskHandle = AvSetMmThreadCharacteristicsW(L"Distribution", &task);
    RunCore();
    // The swap chain belongs to this thread once it was handed over; release it here.
    WdfObjectDelete(reinterpret_cast<WDFOBJECT>(swapChain_));
    swapChain_ = nullptr;
    if (taskHandle) AvRevertMmThreadCharacteristics(taskHandle);
}

void SwapChainProcessor::RunCore() {
    ComPtr<IDXGIDevice> dxgiDevice;
    HRESULT hr = device_->device.As(&dxgiDevice);
    if (FAILED(hr)) return;
    IDARG_IN_SWAPCHAINSETDEVICE setDevice = {};
    setDevice.pDevice = dxgiDevice.Get();
    hr = IddCxSwapChainSetDevice(swapChain_, &setDevice);
    if (FAILED(hr)) {
        Log(L"IddCxSwapChainSetDevice failed: 0x%08X", hr);
        return;
    }
    Log(L"swap chain running");
    uint64_t frames = 0;
    for (;;) {
        IDARG_OUT_RELEASEANDACQUIREBUFFER buffer = {};
        hr = IddCxSwapChainReleaseAndAcquireBuffer(swapChain_, &buffer);
        if (hr == E_PENDING) {
            HANDLE handles[] = {newFrameEvent_, terminateEvent_.Get()};
            const DWORD result = WaitForMultipleObjects(ARRAYSIZE(handles), handles, FALSE, 16);
            if (result == WAIT_OBJECT_0 || result == WAIT_TIMEOUT) continue;
            break;  // terminate, or the wait failed
        }
        if (FAILED(hr)) {
            // The swap chain was abandoned (mode change, monitor removed).
            Log(L"swap chain ended after %llu frames: 0x%08X", frames, hr);
            break;
        }
        if (++frames % 600 == 1) Log(L"swap chain frame %llu", frames);
        // Nothing to do with the picture; give the buffer back right away.
        ComPtr<IDXGIResource> surface;
        surface.Attach(buffer.MetaData.pSurface);
        surface.Reset();
        hr = IddCxSwapChainFinishedProcessingFrame(swapChain_);
        if (FAILED(hr)) break;
    }
}

// MARK: - Device context

void DeviceContext::InitAdapter() {
    if (adapter_) return;
    IDDCX_ADAPTER_CAPS caps = {};
    caps.Size = sizeof(caps);
    caps.MaxMonitorsSupported = LG_DISPLAY_MAX_MONITORS;
    caps.EndPointDiagnostics.Size = sizeof(caps.EndPointDiagnostics);
    caps.EndPointDiagnostics.GammaSupport = IDDCX_FEATURE_IMPLEMENTATION_NONE;
    caps.EndPointDiagnostics.TransmissionType = IDDCX_TRANSMISSION_TYPE_WIRED_OTHER;
    caps.EndPointDiagnostics.pEndPointFriendlyName = L"LemonGalaxy Display";
    caps.EndPointDiagnostics.pEndPointManufacturerName = L"LemonIT";
    caps.EndPointDiagnostics.pEndPointModelName = L"LemonGalaxy";
    IDDCX_ENDPOINT_VERSION version = {};
    version.Size = sizeof(version);
    version.MajorVer = 1;
    caps.EndPointDiagnostics.pFirmwareVersion = &version;
    caps.EndPointDiagnostics.pHardwareVersion = &version;

    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, DeviceContextWrapper);
    IDARG_IN_ADAPTER_INIT init = {};
    init.WdfDevice = device_;
    init.pCaps = &caps;
    init.ObjectAttributes = &attributes;
    IDARG_OUT_ADAPTER_INIT out = {};
    const NTSTATUS status = IddCxAdapterInitAsync(&init, &out);
    Log(L"adapter init: 0x%08X", status);
    if (NT_SUCCESS(status)) {
        adapter_ = out.AdapterObject;
        WdfObjectGet_DeviceContextWrapper(out.AdapterObject)->context = this;
    }
}

void DeviceContext::AdapterInitFinished(NTSTATUS status) {
    std::lock_guard<std::mutex> lock(mutex_);
    adapterReady_ = NT_SUCCESS(status);
    Log(L"adapter ready: 0x%08X", status);
}

LgDisplayInfo DeviceContext::Info() {
    std::lock_guard<std::mutex> lock(mutex_);
    LgDisplayInfo info = {};
    info.version = LG_DISPLAY_INTERFACE_VERSION;
    info.maxMonitors = LG_DISPLAY_MAX_MONITORS;
    info.adapterReady = adapterReady_ ? 1 : 0;
    for (const MonitorEntry& entry : monitors_) {
        if (entry.monitor) info.activeMonitors++;
    }
    return info;
}

NTSTATUS DeviceContext::AddMonitor(const LgDisplayAddRequest& request, WDFFILEOBJECT owner,
                                   LgDisplayAddResult* result) {
    if (request.version != LG_DISPLAY_INTERFACE_VERSION) return STATUS_REVISION_MISMATCH;
    if (request.connector >= LG_DISPLAY_MAX_MONITORS) return STATUS_INVALID_PARAMETER;
    if (request.modeCount == 0 || request.modeCount > LG_DISPLAY_MAX_MODES || request.preferredMode >= request.modeCount) {
        return STATUS_INVALID_PARAMETER;
    }
    std::vector<LgDisplayMode> modes(request.modes, request.modes + request.modeCount);
    for (const LgDisplayMode& mode : modes) {
        if (!ValidMode(mode)) return STATUS_INVALID_PARAMETER;
    }
    {
        std::lock_guard<std::mutex> lock(mutex_);
        if (!adapterReady_) return STATUS_DEVICE_NOT_READY;
    }
    // A connector holds one monitor: replace whatever is there.
    RemoveMonitor(request.connector);

    const LgDisplayMode& preferred = modes[request.preferredMode];
    EdidInfo edidInfo;
    wchar_t name[LG_DISPLAY_NAME_CHARS] = {};
    std::copy(request.name, request.name + LG_DISPLAY_NAME_CHARS - 1, name);
    edidInfo.name = name;
    edidInfo.productCode = static_cast<uint16_t>(0x1000 + request.connector);
    edidInfo.serial = request.serial ? request.serial : 1;
    edidInfo.widthMm = request.widthMm;
    edidInfo.heightMm = request.heightMm;
    edidInfo.width = preferred.width;
    edidInfo.height = preferred.height;
    edidInfo.refreshRate = preferred.refreshRate;
    const Edid edid = BuildEdid(edidInfo);
    RegisterEdid(edid, modes, request.preferredMode);

    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, MonitorContext);
    IDDCX_MONITOR_INFO info = {};
    info.Size = sizeof(info);
    info.MonitorType = DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI;
    info.ConnectorIndex = request.connector;
    info.MonitorDescription.Size = sizeof(info.MonitorDescription);
    info.MonitorDescription.Type = IDDCX_MONITOR_DESCRIPTION_TYPE_EDID;
    info.MonitorDescription.DataSize = static_cast<UINT>(edid.size());
    info.MonitorDescription.pData = const_cast<uint8_t*>(edid.data());
    info.MonitorContainerId = ContainerId(edidInfo.serial, request.connector);

    IDARG_IN_MONITORCREATE create = {};
    create.ObjectAttributes = &attributes;
    create.pMonitorInfo = &info;
    IDARG_OUT_MONITORCREATE created = {};
    NTSTATUS status = IddCxMonitorCreate(adapter_, &create, &created);
    if (!NT_SUCCESS(status)) {
        Log(L"monitor create failed on connector %u: 0x%08X", request.connector, status);
        UnregisterEdid(edid);
        return status;
    }
    MonitorContext* monitorContext = WdfObjectGet_MonitorContext(created.MonitorObject);
    monitorContext->device = this;
    monitorContext->connector = request.connector;
    monitorContext->edid = edid;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        MonitorEntry& entry = monitors_[request.connector];
        entry.monitor = created.MonitorObject;
        entry.owner = owner;
        entry.modes = modes;
        entry.preferredMode = request.preferredMode;
    }

    IDARG_OUT_MONITORARRIVAL arrival = {};
    status = IddCxMonitorArrival(created.MonitorObject, &arrival);
    if (!NT_SUCCESS(status)) {
        Log(L"monitor arrival failed on connector %u: 0x%08X", request.connector, status);
        {
            std::lock_guard<std::mutex> lock(mutex_);
            monitors_[request.connector] = MonitorEntry{};
        }
        WdfObjectDelete(reinterpret_cast<WDFOBJECT>(created.MonitorObject));
        UnregisterEdid(edid);
        return status;
    }
    result->version = LG_DISPLAY_INTERFACE_VERSION;
    result->targetId = arrival.OsTargetId;
    result->adapterLuidLow = arrival.OsAdapterLuid.LowPart;
    result->adapterLuidHigh = arrival.OsAdapterLuid.HighPart;
    Log(L"monitor %u added: %ux%u@%u, %u modes, target %u", request.connector, preferred.width, preferred.height,
        preferred.refreshRate, request.modeCount, arrival.OsTargetId);
    return STATUS_SUCCESS;
}

NTSTATUS DeviceContext::RemoveMonitor(uint32_t connector) {
    if (connector >= LG_DISPLAY_MAX_MONITORS) return STATUS_INVALID_PARAMETER;
    IDDCX_MONITOR monitor = nullptr;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        monitor = monitors_[connector].monitor;
    }
    if (!monitor) return STATUS_NOT_FOUND;
    // The monitor object must not be touched after it departed.
    const Edid edid = WdfObjectGet_MonitorContext(monitor)->edid;
    // Departure unassigns the swap chain through the callbacks, which take the lock themselves.
    const NTSTATUS status = IddCxMonitorDeparture(monitor);
    std::unique_ptr<SwapChainProcessor> leftover;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        leftover = std::move(monitors_[connector].processor);
        monitors_[connector] = MonitorEntry{};
    }
    leftover.reset();
    UnregisterEdid(edid);
    Log(L"monitor %u removed: 0x%08X", connector, status);
    return STATUS_SUCCESS;
}

void DeviceContext::RemoveMonitorsOwnedBy(WDFFILEOBJECT owner) {
    std::vector<uint32_t> connectors;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        for (uint32_t i = 0; i < monitors_.size(); ++i) {
            if (monitors_[i].monitor && monitors_[i].owner == owner) connectors.push_back(i);
        }
    }
    for (uint32_t connector : connectors) RemoveMonitor(connector);
}

std::vector<LgDisplayMode> DeviceContext::Modes(uint32_t connector) {
    std::lock_guard<std::mutex> lock(mutex_);
    return connector < monitors_.size() ? monitors_[connector].modes : std::vector<LgDisplayMode>{};
}

void DeviceContext::AssignSwapChain(uint32_t connector, IDDCX_SWAPCHAIN swapChain, LUID renderAdapter,
                                    HANDLE newFrameEvent) {
    Log(L"assign swap chain to monitor %u, render adapter %08X%08X", connector, renderAdapter.HighPart,
        renderAdapter.LowPart);
    auto device = std::make_shared<Direct3DDevice>(renderAdapter);
    const HRESULT hr = device->Init();
    if (FAILED(hr)) {
        Log(L"render device for monitor %u failed: 0x%08X", connector, hr);
        WdfObjectDelete(reinterpret_cast<WDFOBJECT>(swapChain));
        return;
    }
    auto processor = std::make_unique<SwapChainProcessor>(swapChain, device, newFrameEvent);
    std::unique_ptr<SwapChainProcessor> previous;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        if (connector < monitors_.size() && monitors_[connector].monitor) {
            previous = std::move(monitors_[connector].processor);
            monitors_[connector].processor = std::move(processor);
        }
    }
    // Destroyed outside the lock: joining waits for the processing thread.
    previous.reset();
    processor.reset();
}

void DeviceContext::UnassignSwapChain(uint32_t connector) {
    Log(L"unassign swap chain of monitor %u", connector);
    std::unique_ptr<SwapChainProcessor> processor;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        if (connector < monitors_.size()) processor = std::move(monitors_[connector].processor);
    }
    processor.reset();
}

}  // namespace lg::display

// MARK: - Driver entry points and DDI callbacks

using namespace lg::display;

extern "C" DRIVER_INITIALIZE DriverEntry;
EVT_WDF_DRIVER_DEVICE_ADD LgDeviceAdd;
EVT_WDF_DEVICE_D0_ENTRY LgDeviceD0Entry;
EVT_WDF_FILE_CLEANUP LgFileCleanup;
EVT_IDD_CX_DEVICE_IO_CONTROL LgDeviceIoControl;
EVT_IDD_CX_ADAPTER_INIT_FINISHED LgAdapterInitFinished;
EVT_IDD_CX_ADAPTER_COMMIT_MODES LgAdapterCommitModes;
EVT_IDD_CX_PARSE_MONITOR_DESCRIPTION LgParseMonitorDescription;
EVT_IDD_CX_MONITOR_GET_DEFAULT_DESCRIPTION_MODES LgMonitorGetDefaultModes;
EVT_IDD_CX_MONITOR_QUERY_TARGET_MODES LgMonitorQueryModes;
EVT_IDD_CX_MONITOR_ASSIGN_SWAPCHAIN LgMonitorAssignSwapChain;
EVT_IDD_CX_MONITOR_UNASSIGN_SWAPCHAIN LgMonitorUnassignSwapChain;

namespace {

const GUID kControlInterface = LG_DISPLAY_INTERFACE_GUID_INIT;

DeviceContext* ContextOfDevice(WDFDEVICE device) {
    return WdfObjectGet_DeviceContextWrapper(device)->context;
}

}  // namespace

extern "C" BOOL WINAPI DllMain(HINSTANCE, UINT, LPVOID) { return TRUE; }

_Use_decl_annotations_ extern "C" NTSTATUS DriverEntry(PDRIVER_OBJECT driverObject, PUNICODE_STRING registryPath) {
    WDF_DRIVER_CONFIG config;
    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    WDF_DRIVER_CONFIG_INIT(&config, LgDeviceAdd);
    const NTSTATUS status = WdfDriverCreate(driverObject, registryPath, &attributes, &config, WDF_NO_HANDLE);
    Log(L"driver loaded: 0x%08X", status);
    return status;
}

_Use_decl_annotations_ NTSTATUS LgDeviceAdd(WDFDRIVER, PWDFDEVICE_INIT deviceInit) {
    WDF_PNPPOWER_EVENT_CALLBACKS pnpPower;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&pnpPower);
    pnpPower.EvtDeviceD0Entry = LgDeviceD0Entry;
    WdfDeviceInitSetPnpPowerEventCallbacks(deviceInit, &pnpPower);

    // Monitors die with the handle that added them.
    WDF_FILEOBJECT_CONFIG fileConfig;
    WDF_FILEOBJECT_CONFIG_INIT(&fileConfig, WDF_NO_EVENT_CALLBACK, WDF_NO_EVENT_CALLBACK, LgFileCleanup);
    WdfDeviceInitSetFileObjectConfig(deviceInit, &fileConfig, WDF_NO_OBJECT_ATTRIBUTES);

    IDD_CX_CLIENT_CONFIG iddConfig;
    IDD_CX_CLIENT_CONFIG_INIT(&iddConfig);
    iddConfig.EvtIddCxDeviceIoControl = LgDeviceIoControl;
    iddConfig.EvtIddCxAdapterInitFinished = LgAdapterInitFinished;
    iddConfig.EvtIddCxParseMonitorDescription = LgParseMonitorDescription;
    iddConfig.EvtIddCxMonitorGetDefaultDescriptionModes = LgMonitorGetDefaultModes;
    iddConfig.EvtIddCxMonitorQueryTargetModes = LgMonitorQueryModes;
    iddConfig.EvtIddCxAdapterCommitModes = LgAdapterCommitModes;
    iddConfig.EvtIddCxMonitorAssignSwapChain = LgMonitorAssignSwapChain;
    iddConfig.EvtIddCxMonitorUnassignSwapChain = LgMonitorUnassignSwapChain;
    NTSTATUS status = IddCxDeviceInitConfig(deviceInit, &iddConfig);
    if (!NT_SUCCESS(status)) {
        Log(L"IddCxDeviceInitConfig failed: 0x%08X", status);
        return status;
    }

    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, DeviceContextWrapper);
    attributes.EvtCleanupCallback = [](WDFOBJECT object) {
        DeviceContextWrapper* wrapper = WdfObjectGet_DeviceContextWrapper(object);
        if (wrapper && wrapper->owner) {
            delete wrapper->context;
            wrapper->context = nullptr;
        }
    };
    WDFDEVICE device = nullptr;
    status = WdfDeviceCreate(&deviceInit, &attributes, &device);
    if (!NT_SUCCESS(status)) {
        Log(L"WdfDeviceCreate failed: 0x%08X", status);
        return status;
    }
    status = IddCxDeviceInitialize(device);
    if (!NT_SUCCESS(status)) {
        Log(L"IddCxDeviceInitialize failed: 0x%08X", status);
        return status;
    }

    DeviceContextWrapper* wrapper = WdfObjectGet_DeviceContextWrapper(device);
    wrapper->context = new DeviceContext(device);
    wrapper->owner = true;

    status = WdfDeviceCreateDeviceInterface(device, &kControlInterface, nullptr);
    Log(L"device added, control interface: 0x%08X", status);
    return status;
}

_Use_decl_annotations_ NTSTATUS LgDeviceD0Entry(WDFDEVICE device, WDF_POWER_DEVICE_STATE) {
    ContextOfDevice(device)->InitAdapter();
    return STATUS_SUCCESS;
}

_Use_decl_annotations_ VOID LgFileCleanup(WDFFILEOBJECT fileObject) {
    WDFDEVICE device = WdfFileObjectGetDevice(fileObject);
    if (DeviceContext* context = ContextOfDevice(device)) context->RemoveMonitorsOwnedBy(fileObject);
}

_Use_decl_annotations_ VOID LgDeviceIoControl(WDFDEVICE device, WDFREQUEST request, size_t, size_t, ULONG code) {
    DeviceContext* context = ContextOfDevice(device);
    NTSTATUS status = STATUS_INVALID_DEVICE_REQUEST;
    size_t written = 0;
    switch (code) {
        case IOCTL_LG_DISPLAY_GET_INFO: {
            LgDisplayInfo* out = nullptr;
            status = WdfRequestRetrieveOutputBuffer(request, sizeof(*out), reinterpret_cast<PVOID*>(&out), nullptr);
            if (NT_SUCCESS(status)) {
                *out = context->Info();
                written = sizeof(*out);
            }
            break;
        }
        case IOCTL_LG_DISPLAY_ADD: {
            LgDisplayAddRequest* in = nullptr;
            LgDisplayAddResult* out = nullptr;
            status = WdfRequestRetrieveInputBuffer(request, sizeof(*in), reinterpret_cast<PVOID*>(&in), nullptr);
            if (!NT_SUCCESS(status)) break;
            // Buffered I/O: input and output share one buffer, so copy the request first.
            const LgDisplayAddRequest copy = *in;
            status = WdfRequestRetrieveOutputBuffer(request, sizeof(*out), reinterpret_cast<PVOID*>(&out), nullptr);
            if (!NT_SUCCESS(status)) break;
            LgDisplayAddResult result = {};
            status = context->AddMonitor(copy, WdfRequestGetFileObject(request), &result);
            if (NT_SUCCESS(status)) {
                *out = result;
                written = sizeof(*out);
            }
            break;
        }
        case IOCTL_LG_DISPLAY_REMOVE: {
            LgDisplayRemoveRequest* in = nullptr;
            status = WdfRequestRetrieveInputBuffer(request, sizeof(*in), reinterpret_cast<PVOID*>(&in), nullptr);
            if (NT_SUCCESS(status)) status = in->version == LG_DISPLAY_INTERFACE_VERSION
                                                 ? context->RemoveMonitor(in->connector)
                                                 : STATUS_REVISION_MISMATCH;
            break;
        }
        default:
            break;
    }
    WdfRequestCompleteWithInformation(request, status, written);
}

_Use_decl_annotations_ NTSTATUS LgAdapterInitFinished(IDDCX_ADAPTER adapter, const IDARG_IN_ADAPTER_INIT_FINISHED* in) {
    if (DeviceContext* context = WdfObjectGet_DeviceContextWrapper(adapter)->context) {
        context->AdapterInitFinished(in->AdapterInitStatus);
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_ NTSTATUS LgAdapterCommitModes(IDDCX_ADAPTER, const IDARG_IN_COMMITMODES*) {
    return STATUS_SUCCESS;
}

_Use_decl_annotations_ NTSTATUS LgParseMonitorDescription(const IDARG_IN_PARSEMONITORDESCRIPTION* in,
                                                           IDARG_OUT_PARSEMONITORDESCRIPTION* out) {
    EdidModes entry;
    if (!FindEdid(in->MonitorDescription.pData, in->MonitorDescription.DataSize, &entry)) {
        return STATUS_INVALID_PARAMETER;
    }
    out->MonitorModeBufferOutputCount = static_cast<UINT>(entry.modes.size());
    if (in->MonitorModeBufferInputCount < entry.modes.size()) {
        // The OS first asks how many modes there are.
        return in->MonitorModeBufferInputCount > 0 ? STATUS_BUFFER_TOO_SMALL : STATUS_SUCCESS;
    }
    for (size_t i = 0; i < entry.modes.size(); ++i) {
        in->pMonitorModes[i] = MonitorMode(entry.modes[i], IDDCX_MONITOR_MODE_ORIGIN_MONITORDESCRIPTOR);
    }
    out->PreferredMonitorModeIdx = entry.preferred;
    return STATUS_SUCCESS;
}

_Use_decl_annotations_ NTSTATUS LgMonitorGetDefaultModes(IDDCX_MONITOR, const IDARG_IN_GETDEFAULTDESCRIPTIONMODES* in,
                                                          IDARG_OUT_GETDEFAULTDESCRIPTIONMODES* out) {
    // Every monitor comes with an EDID; this is only a fallback.
    const LgDisplayMode fallback = {1920, 1080, 60};
    out->DefaultMonitorModeBufferOutputCount = 1;
    if (in->DefaultMonitorModeBufferInputCount > 0) {
        in->pDefaultMonitorModes[0] = MonitorMode(fallback, IDDCX_MONITOR_MODE_ORIGIN_DRIVER);
        out->PreferredMonitorModeIdx = 0;
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_ NTSTATUS LgMonitorQueryModes(IDDCX_MONITOR monitor, const IDARG_IN_QUERYTARGETMODES* in,
                                                     IDARG_OUT_QUERYTARGETMODES* out) {
    MonitorContext* monitorContext = WdfObjectGet_MonitorContext(monitor);
    std::vector<LgDisplayMode> modes = monitorContext->device->Modes(monitorContext->connector);
    if (modes.empty()) modes.push_back(LgDisplayMode{1920, 1080, 60});
    out->TargetModeBufferOutputCount = static_cast<UINT>(modes.size());
    if (in->TargetModeBufferInputCount >= modes.size()) {
        for (size_t i = 0; i < modes.size(); ++i) in->pTargetModes[i] = TargetMode(modes[i]);
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_ NTSTATUS LgMonitorAssignSwapChain(IDDCX_MONITOR monitor, const IDARG_IN_SETSWAPCHAIN* in) {
    MonitorContext* monitorContext = WdfObjectGet_MonitorContext(monitor);
    monitorContext->device->AssignSwapChain(monitorContext->connector, in->hSwapChain, in->RenderAdapterLuid,
                                            in->hNextSurfaceAvailable);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_ NTSTATUS LgMonitorUnassignSwapChain(IDDCX_MONITOR monitor) {
    MonitorContext* monitorContext = WdfObjectGet_MonitorContext(monitor);
    monitorContext->device->UnassignSwapChain(monitorContext->connector);
    return STATUS_SUCCESS;
}
