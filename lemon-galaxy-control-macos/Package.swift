// swift-tools-version:5.9
// LemonGalaxy macOS 客户端（主控 + 被控）。
// 开发：swift build；打包成通用二进制 .app：scripts/build.sh [--debug] [--dmg]。
import PackageDescription
import Foundation

let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
// Rust 内核（galaxy-ffi）以 xcframework 形式放在 Vendor/ 下：存在才接入，没有就只用模拟内核（MockCore）编译。
// 接入真内核时同时把 uniffi 生成的 galaxy_ffi.swift 放到 Sources/LemonGalaxy/Core/Generated/，
// 并删除 Core/Bridge/ 里与之同名的手写类型（见 Core/Bridge/README.md）。
let hasRealCore = FileManager.default.fileExists(atPath: packageDir + "/Vendor/LemonGalaxyCore.xcframework")

var appDependencies: [Target.Dependency] = [
    .product(name: "SwiftTerm", package: "SwiftTerm"),
]
var appSwiftSettings: [SwiftSetting] = [
    .unsafeFlags(["-Osize"], .when(configuration: .release)),
]
var extraTargets: [Target] = []
if hasRealCore {
    extraTargets.append(.binaryTarget(name: "LemonGalaxyCore", path: "Vendor/LemonGalaxyCore.xcframework"))
    appDependencies.append("LemonGalaxyCore")
    appSwiftSettings.append(.define("LEMONGALAXY_REAL_CORE"))
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
            exclude: ["Core/Bridge/README.md", "Core/Generated/README.md"],
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
            ]
        ),
    ] + extraTargets,
    swiftLanguageVersions: [.v5]
)
