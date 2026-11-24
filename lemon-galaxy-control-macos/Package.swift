// swift-tools-version:5.9
// LemonGalaxy macOS 客户端（主控 + 被控）。
// 开发：swift build；打包成通用二进制 .app：scripts/build.sh [--debug] [--dmg]。
import PackageDescription
import Foundation

let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
// Rust 内核（galaxy-ffi）：Vendor/LemonGalaxyCore.xcframework（静态库 + C 头 + modulemap，模块 galaxy_ffiFFI）
// 加 uniffi 生成的 Sources/LemonGalaxy/Core/Generated/galaxy_ffi.swift，两者都在才接入（仓库根 scripts/build-ffi-apple.sh 生成）。
// 接入后默认用真内核，--mock / --selftest 仍用模拟内核。
// 没有内核产物（或设环境变量 LEMONGALAXY_NO_CORE=1）时只编译模拟内核，类型来自 Core/Bridge/BridgeMirror.swift。
let env = ProcessInfo.processInfo.environment
let generatedPath = packageDir + "/Sources/LemonGalaxy/Core/Generated/galaxy_ffi.swift"
let hasGenerated = FileManager.default.fileExists(atPath: generatedPath)
let hasRealCore = env["LEMONGALAXY_NO_CORE"] != "1"
    && FileManager.default.fileExists(atPath: packageDir + "/Vendor/LemonGalaxyCore.xcframework")
    && hasGenerated

var appDependencies: [Target.Dependency] = [
    .product(name: "SwiftTerm", package: "SwiftTerm"),
    "CGVirtualDisplayBridge",
]
var appSwiftSettings: [SwiftSetting] = [
    .unsafeFlags(["-Osize"], .when(configuration: .release)),
]
var appExcludes = ["Core/Bridge/README.md", "Core/Generated/README.md"]
var extraTargets: [Target] = []
if hasRealCore {
    extraTargets.append(.binaryTarget(name: "LemonGalaxyCore", path: "Vendor/LemonGalaxyCore.xcframework"))
    appDependencies.append("LemonGalaxyCore")
    appSwiftSettings.append(.define("LEMONGALAXY_REAL_CORE"))
} else if hasGenerated {
    appExcludes.append("Core/Generated/galaxy_ffi.swift")
}

let package = Package(
    name: "LemonGalaxy",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "LemonGalaxy", targets: ["LemonGalaxy"]),
    ],
    dependencies: [
        // 固定 1.2.x：这一版没有任何第三方依赖，CI 与离线构建都更稳；新版（1.5 起）带 argument-parser 等依赖与构建插件
        .package(url: "https://github.com/migueldeicaza/SwiftTerm", .upToNextMinor(from: "1.2.5")),
    ],
    targets: [
        .executableTarget(
            name: "LemonGalaxy",
            dependencies: appDependencies,
            path: "Sources/LemonGalaxy",
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
        // 虚拟屏：CoreGraphics 私有类 CGVirtualDisplay 的 Objective-C 薄封装（运行时取类，不静态链接私有符号）。
        // 纯被控端（lemon-galaxy-worker-macos）用符号链接共用同一份源码
        .target(
            name: "CGVirtualDisplayBridge",
            path: "Sources/CGVirtualDisplayBridge",
            linkerSettings: [.linkedFramework("CoreGraphics")]
        ),
    ] + extraTargets,
    swiftLanguageVersions: [.v5]
)
