#!/usr/bin/env python3
"""从 uniffi 生成的 galaxy_ffi.swift 生成「无内核镜像」Core/Bridge/BridgeMirror.swift。

用途：没有 Vendor/LemonGalaxyCore.xcframework 时（例如没编 Rust 的机器、只想看界面），工程仍能只用模拟内核编译。
镜像里的记录、枚举、协议与生成代码逐字相同；对象类只保留 init(noHandle:) 和方法签名（方法体是占位），
MockCore 继承它们，与继承真生成类的写法完全一样。整个文件包在 #if !LEMONGALAXY_REAL_CORE 里。

每次重新生成内核绑定（scripts/build-ffi-apple.sh）后运行一次：
    python3 scripts/gen-bridge-mirror.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "Sources/LemonGalaxy/Core/Generated/galaxy_ffi.swift"
OUT = ROOT / "Sources/LemonGalaxy/Core/Bridge/BridgeMirror.swift"

text = SRC.read_text(encoding="utf-8")
lines = text.split("\n")


def block_at(i):
    """从第 i 行（含 '{'）起按大括号配对，返回块结束行号（含）。"""
    depth = 0
    started = False
    for j in range(i, len(lines)):
        for ch in lines[j]:
            if ch == "{":
                depth += 1
                started = True
            elif ch == "}":
                depth -= 1
        if started and depth == 0:
            return j
    raise ValueError(f"大括号不配对：第 {i + 1} 行")


converters = set(re.findall(r"public struct FfiConverterType(\w+)\b", text))
out = []
types = []          # 记录与枚举
protocols = []      # 协议（对象协议 + 回调协议）
objects = []        # 对象类名

i = 0
while i < len(lines):
    line = lines[i]
    m = re.match(r"public (struct|enum) (\w+)\b", line)
    if m and m.group(2) in converters:
        end = block_at(i)
        types.append("\n".join(lines[i:end + 1]))
        i = end + 1
        continue
    m = re.match(r"extension (\w+): (Equatable, Hashable|Foundation\.LocalizedError) \{", line)
    if m and m.group(1) in converters:
        end = block_at(i)
        types.append("\n".join(lines[i:end + 1]))
        i = end + 1
        continue
    m = re.match(r"extension (\w+): Sendable \{\}", line)
    if m and m.group(1) in converters:
        types.append("#if compiler(>=6)\n" + line + "\n#endif")
        i += 1
        continue
    m = re.match(r"public protocol (\w+): AnyObject, Sendable \{", line)
    if m:
        end = block_at(i)
        protocols.append((m.group(1), lines[i:end + 1]))
        i = end + 1
        continue
    m = re.match(r"open class (\w+): (\w+)Protocol, @unchecked Sendable \{", line)
    if m and m.group(1) == m.group(2):
        objects.append(m.group(1))
    i += 1

proto_methods = {}
for name, body in protocols:
    sigs = []
    for l in body[1:-1]:
        s = l.strip()
        if s.startswith("func "):
            sigs.append(s)
    proto_methods[name] = sigs


def stub_body(sig):
    return "{ bridgeUnimplemented() }"


obj_src = []
for name in objects:
    methods = proto_methods.get(name + "Protocol", [])
    parts = [f"open class {name}: {name}Protocol, @unchecked Sendable {{",
             "    public struct NoHandle { public init() {} }",
             "    public init(noHandle: NoHandle) {}",
             "    required public init(unsafeFromHandle handle: UInt64) {}"]
    for sig in methods:
        parts.append(f"    open {sig.rstrip()} {stub_body(sig)}")
    parts.append("}")
    obj_src.append("\n".join(parts))

header = '''// 由 scripts/gen-bridge-mirror.py 从 Core/Generated/galaxy_ffi.swift 生成，不要手改。
// 没有接入 Rust 内核（无 Vendor/LemonGalaxyCore.xcframework）时用它编译：类型与真绑定逐字相同，对象方法是占位，
// 只有 MockCore 的子类会被创建。接入真内核后（编译条件 LEMONGALAXY_REAL_CORE）本文件整体不参与编译。
#if !LEMONGALAXY_REAL_CORE
import Foundation

@inline(never)
func bridgeUnimplemented(_ function: String = #function) -> Never {
    fatalError("内核接口 \\(function) 没有实现：当前没有接入 Rust 内核，只能用 MockCore")
}

public func coreVersion() -> String { "0.0.0-mirror" }
public func coreInit(config: CoreConfig) {}
public func coreSetLocale(locale: String) {}
public func coreLogPath() -> String { "" }
'''
body = header + "\n// MARK: - 记录与枚举\n\n" + "\n\n".join(types)
body += "\n\n// MARK: - 协议（对象与回调）\n\n" + "\n\n".join("\n".join(b) for _, b in protocols)
body += "\n\n// MARK: - 对象（占位）\n\n" + "\n\n".join(obj_src)
body += "\n#endif\n"
# 去掉 uniffi 的文档可见性标注，镜像里不需要
body = re.sub(r"#if swift\(>=5\.8\)\n@_documentation\(visibility: private\)\n#endif\n", "", body)
OUT.write_text(body, encoding="utf-8")
print(f"{OUT.relative_to(ROOT)}: {len(types)} 段类型, {len(protocols)} 个协议, {len(objects)} 个对象")
