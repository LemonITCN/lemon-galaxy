# Termux 终端库（随仓库提供的修改版）

来源：<https://github.com/termux/termux-app> 标签 `v0.118.3`（提交 `5b657c6adf4304e5198951ce815fe0205dcac29c`）的
`terminal-emulator` 与 `terminal-view` 两个模块。这两个模块按 Apache License 2.0 发布（见同目录 `LICENSE`；
termux-app 仓库其余部分为 GPLv3，本工程没有使用）。

## 为什么不直接用 JitPack 上的版本

JitPack 上的 `com.github.termux.termux-app:terminal-view` / `terminal-emulator` 可以下载，但原版
`TerminalSession` 是 `final` 类，且固定在本机 fork 子进程、通过伪终端读写。LemonGalaxy 的终端在远端被控设备上，
字节流经 Rust 内核的端到端通道传输（`TerminalChannel` / `TerminalSink`），原版无法接入。

## 改动清单（Apache-2.0 第 4 条要求标注）

- `terminal-emulator/.../TerminalSession.java`：去掉 JNI 子进程、伪终端与读写线程；构造参数改为 `Transport`
  （写入、改尺寸、关闭三个回调）；新增 `appendRemoteOutput()`、`onRemoteExit()` 两个入口；`getPid()` 恒为 0、`getCwd()` 恒为 null。
- 删除 `terminal-emulator/.../JNI.java` 与 `src/main/jni/`（不再需要本地库 `libtermux.so`）。
- 删除两个模块的测试目录。
- 两个 `AndroidManifest.xml` 去掉 `package` 属性（改由 Gradle `namespace` 指定）；构建脚本改写为 `build.gradle.kts`。
- `terminal-view` 新增 `res/values-zh/strings.xml`（选中文字浮动菜单的中文文案）。
- 两个模块的 `proguard-rules.pro` 改为保留 `com.termux.**`。

其余源码保持原样。
