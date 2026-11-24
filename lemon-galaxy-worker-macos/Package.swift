// swift-tools-version:5.9
// LemonGalaxyWorker：macOS 纯被控端（菜单栏常驻 + 设置窗口，可同时登录多个账号）。
// 开发：swift build；打包成通用二进制 .app：scripts/build.sh [--debug] [--dmg]。
//
// 源码布局（target 根目录是 Sources/）：
//   Sources/LemonGalaxyWorker/   被控端自己的代码（Unattended/ 是无人值守：后台服务、会话代理、本机通信，见 docs/spec.md 3.0.1）
//   Sources/Shared/              指向 Mac 主控端（lemon-galaxy-control-macos）源码的符号链接：
//                                截屏编码与键鼠注入（MacHostPlatform / VideoEncoder / HIDKeyMap）、虚拟屏（VirtualDisplays）、
//                                无头兜底屏设置（HeadlessDisplaySetting）、钥匙串（KeychainStore）、
//                                NSLock 小工具（Util）、配色（Palette）、语言包底座（L10n）、通用小组件（Components）、
//                                无内核时的类型镜像（BridgeMirror）。两端共用同一份实现，不另起分叉。
//   CSources/CGVirtualDisplayBridge/  虚拟屏（CGVirtualDisplay 私有接口）的 Objective-C 封装，同样是指向主控端源码的符号链接
//   Sources/Generated/           uniffi 生成的 galaxy_ffi.swift（仓库根 scripts/build-ffi-apple.sh 生成，不进仓库）
//
// Rust 内核：Vendor/LemonGalaxyCore.xcframework 与 Sources/Generated/galaxy_ffi.swift 都在时接入，定义 LEMONGALAXY_REAL_CORE，
// 默认用真内核（--mock、LEMONGALAXY_MOCK=1、--selftest 用模拟内核）。
// 没有内核产物（或环境变量 LEMONGALAXY_NO_CORE=1）时只编译模拟内核，类型来自 Shared/BridgeMirror.swift。
import PackageDescription
import Foundation

let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let env = ProcessInfo.processInfo.environment
let generatedPath = packageDir + "/Sources/Generated/galaxy_ffi.swift"
let hasGenerated = FileManager.default.fileExists(atPath: generatedPath)
let hasRealCore = env["LEMONGALAXY_NO_CORE"] != "1"
    && FileManager.default.fileExists(atPath: packageDir + "/Vendor/LemonGalaxyCore.xcframework")
    && hasGenerated

var appDependencies: [Target.Dependency] = ["CGVirtualDisplayBridge"]
var appSwiftSettings: [SwiftSetting] = [
    .unsafeFlags(["-Osize"], .when(configuration: .release)),
]
var appExcludes: [String] = []
var extraTargets: [Target] = []
if hasRealCore {
    extraTargets.append(.binaryTarget(name: "LemonGalaxyCore", path: "Vendor/LemonGalaxyCore.xcframework"))
    appDependencies.append("LemonGalaxyCore")
    appSwiftSettings.append(.define("LEMONGALAXY_REAL_CORE"))
} else if hasGenerated {
    appExcludes.append("Generated/galaxy_ffi.swift")
}

let package = Package(
    name: "LemonGalaxyWorker",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "LemonGalaxyWorker", targets: ["LemonGalaxyWorker"]),
    ],
    targets: [
        .executableTarget(
            name: "LemonGalaxyWorker",
            dependencies: appDependencies,
            path: "Sources",
            exclude: appExcludes,
            swiftSettings: appSwiftSettings,
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("SwiftUI"),
                .linkedFramework("ScreenCaptureKit"),
                .linkedFramework("VideoToolbox"),
                .linkedFramework("CoreMedia"),
                .linkedFramework("CoreVideo"),
                .linkedFramework("ApplicationServices"),
                .linkedFramework("Security"),
                .linkedFramework("Carbon"),
                .linkedFramework("ServiceManagement"),
                .linkedFramework("UserNotifications"),
                // Rust 静态库（galaxy-ffi）用到的系统框架与库
                .linkedFramework("CoreFoundation"),
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("IOKit"),
                .linkedLibrary("resolv"),
                .linkedLibrary("iconv"),
                // 远端声音播放（cpal）；Opus 编解码已静态编进内核
                .linkedFramework("AudioToolbox"),
                // CoreAudio 弱链接：cpal 引用了 macOS 14.2 才有的 AudioHardwareCreateProcessTap 等，
                // 强链接时 macOS 13 一启动就因找不到符号退出；弱链接后这些符号在旧系统上为空，cpal 不会去调
                .unsafeFlags(["-Xlinker", "-weak_framework", "-Xlinker", "CoreAudio"]),
            ]
        ),
        // 虚拟屏：CGVirtualDisplay 私有类的 Objective-C 薄封装。主程序 target 的根是 Sources/，C 语言族的 target 不能放在它下面，
        // 所以放在 CSources/；里面是指向 Mac 主控端同名目录的符号链接，两端共用同一份源码
        .target(
            name: "CGVirtualDisplayBridge",
            path: "CSources/CGVirtualDisplayBridge",
            linkerSettings: [.linkedFramework("CoreGraphics")]
        ),
    ] + extraTargets,
    swiftLanguageVersions: [.v5]
)
