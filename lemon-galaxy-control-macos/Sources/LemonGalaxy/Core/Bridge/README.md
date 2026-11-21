# 内核接口层（Bridge）

本目录是 `docs/ffi.md` 在 Swift 里的镜像，外形与 uniffi 0.28 生成的代码一致：

- 记录 → `struct`，枚举 → `enum`（分支首字母小写），`Vec<u8>` → `Data`，方法与字段驼峰；
- 对象 → `XxxProtocol` 协议 + `open class Xxx`，带 `init(noPointer:)`，模拟内核（`Core/Mock`）继承这些类覆盖方法；
- 回调接口 → `protocol Xxx: AnyObject`。

界面只认这些类型，经 `Core/CoreProvider.swift` 拿到实现，不直接依赖 MockCore 内部。

## 接入真内核

1. 把 Rust 产出的 `LemonGalaxyCore.xcframework`（静态库 + uniffi 生成的 C 头与 modulemap）放到 `Vendor/`。
   `Package.swift` 检测到它就加 binaryTarget，并定义编译条件 `LEMONGALAXY_REAL_CORE`。
2. 把 uniffi 生成的 `galaxy_ffi.swift` 放到 `Sources/LemonGalaxy/Core/Generated/`。
3. 本目录的 `BridgeTypes.swift`、`BridgeObjects.swift` 整个包在 `#if !LEMONGALAXY_REAL_CORE` 里，自动让位给生成代码；
   `Proposed.swift`（建议内核补充的接口）保留。
4. 以 `--real` 或环境变量 `LEMONGALAXY_MOCK=0` 启动即用真内核；稳定后把 `CoreProvider.defaultUsesMock` 改为 `false`。

ffi.md 没写字段、这里先定下的类型（ExecRequest、ClipboardData、HostStatus 等）在文件里标了「ffi.md 未定义」，内核实现时请按同名同字段导出，或在这里同步修改。
