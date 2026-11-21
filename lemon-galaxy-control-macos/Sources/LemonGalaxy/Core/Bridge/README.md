# 内核接口层（Bridge）

界面只认 uniffi 生成的内核类型（`GalaxyClient`、`PeerSession`、`HostService` …），经 `Core/CoreProvider.swift` 拿到实现，不直接依赖 MockCore 内部。

- **真内核**：`Vendor/LemonGalaxyCore.xcframework` 与 `Core/Generated/galaxy_ffi.swift` 都在时，`Package.swift` 接入静态库并定义 `LEMONGALAXY_REAL_CORE`。
  两者由仓库根的 `scripts/build-ffi-apple.sh [--debug]` 生成。默认用真内核；`--mock`、`LEMONGALAXY_MOCK=1` 与 `--selftest` 用模拟内核。
- **模拟内核**（`Core/Mock`）：继承生成的 `open class`（`init(noHandle:)`）覆盖全部方法，真内核与镜像两种编译方式下写法相同。
- **镜像** `BridgeMirror.swift`：没有内核产物（或 `LEMONGALAXY_NO_CORE=1`）时用它编译，只有模拟内核可用。
  它由 `scripts/gen-bridge-mirror.py` 从 `galaxy_ffi.swift` 自动生成（记录、枚举、协议逐字相同，对象方法为占位），
  每次重新生成内核绑定后运行一次：`python3 scripts/gen-bridge-mirror.py`。
- `Proposed.swift`：内核还没有、界面已经准备好的接口（目前是 SSH 直连）。界面用 `client as? GalaxyClientSshProposed` 判断，真内核下对应按钮置灰。

冒烟测试（只用真内核，内存 SecureStore，不开窗口）：
`LemonGalaxy --core-smoke <服务器地址> <用户名> <密码>`
