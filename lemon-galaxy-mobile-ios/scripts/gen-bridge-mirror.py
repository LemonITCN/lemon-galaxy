#!/usr/bin/env python3
"""从 uniffi 生成的 Sources/Generated/galaxy_ffi.swift 生成「无内核镜像」Sources/CoreMirror/BridgeMirror.swift。

用途：没有 Vendor/LemonGalaxyCore.xcframework 时（没编 Rust 的机器、CI 只想编界面），工程仍能只用模拟内核编译。
镜像里的记录、枚举、协议与生成代码逐字相同；对象类只保留 init(noHandle:)、构造器与方法签名（方法体是占位），
模拟内核继承它们，与继承真生成类的写法完全一样。Package.swift 按有无内核产物二选一，两份不会同时参与编译。

每次重新生成内核绑定（仓库根 scripts/build-ffi-apple.sh --ios）后运行一次：
    python3 scripts/gen-bridge-mirror.py
"""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "Sources/Generated/galaxy_ffi.swift"
OUT = ROOT / "Sources/CoreMirror/BridgeMirror.swift"

text = SRC.read_text(encoding="utf-8")
# uniffi 0.32 把错误类型写成「public」换行「enum GalaxyError」，先并成一行
text = re.sub(r"public[ \t]*\n(enum|struct) ", r"public \1 ", text)
lines = text.split("\n")


def block_at(i):
    """从第 i 行起按大括号配对，返回块结束行号（含）。"""
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
types = []          # 记录与枚举（含错误）
protocols = []      # 协议（对象协议 + 回调协议）
objects = {}        # 对象类名 -> (构造器签名列表, 静态构造方法签名列表)
functions = []      # 顶层导出函数签名

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
        end = block_at(i)
        inits, statics = [], []
        for l in lines[i + 1:end]:
            s = l.strip()
            if s.startswith("public convenience init(") and s.endswith("{"):
                inits.append(s[:-1].rstrip())
            elif s.startswith("public static func ") and s.endswith("{"):
                statics.append(s[:-1].rstrip())
        objects[m.group(1)] = (inits, statics)
        i = end + 1
        continue
    m = re.match(r"public func (\w+)\(.*\{", line)
    # 只要内核导出的顶层函数（core_*）；FfiConverter*、uniffi* 是绑定内部用的
    if m and not m.group(1).startswith(("FfiConverter", "uniffi")):
        functions.append(line[:line.index("{")].rstrip())
    i += 1

proto_methods = {}
for name, body in protocols:
    sigs = []
    for l in body[1:-1]:
        s = l.strip()
        if s.startswith("func "):
            sigs.append(s)
    proto_methods[name] = sigs

obj_src = []
for name, (inits, statics) in objects.items():
    parts = [f"open class {name}: {name}Protocol, @unchecked Sendable {{",
             "    public struct NoHandle { public init() {} }",
             "    public init(noHandle: NoHandle) {}",
             "    required public init(unsafeFromHandle handle: UInt64) {}"]
    for sig in inits:
        parts.append(f"    {sig} {{ self.init(noHandle: NoHandle()); bridgeUnimplemented() }}")
    for sig in statics:
        parts.append(f"    {sig} {{ bridgeUnimplemented() }}")
    for sig in proto_methods.get(name + "Protocol", []):
        parts.append(f"    open {sig.rstrip()} {{ bridgeUnimplemented() }}")
    parts.append("}")
    obj_src.append("\n".join(parts))

SPECIAL = {
    "coreVersion": '"0.0.0-mirror"',
    "coreLogPath": '""',
}
fn_src = []
for sig in functions:
    name = re.match(r"public func (\w+)", sig).group(1)
    if name in SPECIAL:
        fn_src.append(f"{sig} {{ {SPECIAL[name]} }}")
    elif "->" in sig:
        fn_src.append(f"{sig} {{ bridgeUnimplemented() }}")
    else:
        fn_src.append(f"{sig} {{}}")

header = '''// 由 scripts/gen-bridge-mirror.py 从 Sources/Generated/galaxy_ffi.swift 生成，不要手改。
// 没有接入 Rust 内核（无 Vendor/LemonGalaxyCore.xcframework）时 Package.swift 用它代替真绑定：
// 类型与真绑定逐字相同，对象方法是占位，只有模拟内核的子类会被创建。
import Foundation

@inline(never)
func bridgeUnimplemented(_ function: String = #function) -> Never {
    fatalError("内核接口 \\(function) 没有实现：当前没有接入 Rust 内核，只能用模拟内核")
}
'''
body = header
body += "\n// MARK: - 顶层函数（占位）\n\n" + "\n".join(fn_src)
body += "\n\n// MARK: - 记录与枚举\n\n" + "\n\n".join(types)
body += "\n\n// MARK: - 协议（对象与回调）\n\n" + "\n\n".join("\n".join(b) for _, b in protocols)
body += "\n\n// MARK: - 对象（占位）\n\n" + "\n\n".join(obj_src) + "\n"
# 去掉 uniffi 的文档可见性标注，镜像里不需要
body = re.sub(r"#if swift\(>=5\.8\)\n@_documentation\(visibility: private\)\n#endif\n", "", body)
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(body, encoding="utf-8")
print(f"{OUT.relative_to(ROOT)}: {len(types)} 段类型, {len(protocols)} 个协议, {len(objects)} 个对象, {len(functions)} 个函数")
