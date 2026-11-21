// swift-tools-version:5.9
// LemonGalaxy iOS / iPadOS 客户端的公共部分（与界面无关）。
//
//   GalaxyCore   Rust 内核的 Swift 绑定：有内核产物时是 uniffi 生成的 Sources/Generated/galaxy_ffi.swift + Vendor/LemonGalaxyCore.xcframework，
//                没有时是 Sources/CoreMirror/BridgeMirror.swift（类型逐字相同、对象方法占位，只能配合模拟内核用）。
//   GalaxyKit    适配层（钥匙串、内核初始化、回调切主线程）、模拟内核、平台无关的逻辑（地址解析、设备树、AI 消息合并、按键编码、画面坐标换算）
//                与界面模型。iOS App（LemonGalaxy.xcodeproj）依赖它；macOS 上可以 swift test。
//   galaxy-core-smoke  macOS 上跑的真内核冒烟测试：用 xcframework 的 macOS 切片走一遍 GalaxyKit 的适配层。
//
// 内核产物由仓库根的 scripts/build-ffi-apple.sh --ios 生成（两者都在才接入）；设环境变量 LEMONGALAXY_NO_CORE=1 可强制只用模拟内核。
import PackageDescription
import Foundation

let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let env = ProcessInfo.processInfo.environment
let fm = FileManager.default
let hasRealCore = env["LEMONGALAXY_NO_CORE"] != "1"
    && fm.fileExists(atPath: packageDir + "/Vendor/LemonGalaxyCore.xcframework/Info.plist")
    && fm.fileExists(atPath: packageDir + "/Sources/Generated/galaxy_ffi.swift")

var coreDependencies: [Target.Dependency] = []
var kitSettings: [SwiftSetting] = []
var extraTargets: [Target] = []
if hasRealCore {
    extraTargets.append(.binaryTarget(name: "LemonGalaxyCore", path: "Vendor/LemonGalaxyCore.xcframework"))
    coreDependencies.append("LemonGalaxyCore")
    kitSettings.append(.define("LEMONGALAXY_REAL_CORE"))
}

let package = Package(
    name: "LemonGalaxyKit",
    platforms: [.iOS(.v16), .macOS(.v13)],
    products: [
        .library(name: "GalaxyKit", targets: ["GalaxyKit"]),
        .executable(name: "galaxy-core-smoke", targets: ["GalaxyCoreSmoke"]),
    ],
    targets: [
        .target(
            name: "GalaxyCore",
            dependencies: coreDependencies,
            path: hasRealCore ? "Sources/Generated" : "Sources/CoreMirror",
            linkerSettings: [
                // Rust 静态库（galaxy-ffi）用到的系统框架与库
                .linkedFramework("CoreFoundation"),
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("Security"),
                .linkedFramework("IOKit", .when(platforms: [.macOS])),
                // 远端声音播放（cpal：RemoteIO / CoreAudio 输出，iOS 上经 AVAudioSession 取输出配置）
                .linkedFramework("AudioToolbox"),
                .linkedFramework("CoreAudio"),
                .linkedFramework("AVFAudio"),
                .linkedLibrary("resolv"),
                .linkedLibrary("iconv"),
            ]
        ),
        .target(
            name: "GalaxyKit",
            dependencies: ["GalaxyCore"],
            path: "Sources/GalaxyKit",
            swiftSettings: kitSettings,
            linkerSettings: [
                .linkedFramework("VideoToolbox"),
                .linkedFramework("CoreMedia"),
                .linkedFramework("CoreVideo"),
            ]
        ),
        .executableTarget(
            name: "GalaxyCoreSmoke",
            dependencies: ["GalaxyKit"],
            path: "Sources/CoreSmoke",
            swiftSettings: kitSettings
        ),
        .testTarget(
            name: "GalaxyKitTests",
            dependencies: ["GalaxyKit"],
            path: "Tests/GalaxyKitTests"
        ),
    ] + extraTargets,
    swiftLanguageVersions: [.v5]
)
