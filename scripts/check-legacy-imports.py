#!/usr/bin/env python3
"""核对 Windows 程序的导入表：每个导入的函数在目标系统的 DLL 里都要有，否则程序在那个系统上根本启动不了。

    python3 scripts/check-legacy-imports.py 导入表.txt 目标系统导出表.txt

导入表.txt：`dumpbin /imports 程序.exe` 或 `llvm-readobj --coff-imports 程序.exe` 的输出。
目标系统导出表.txt：YY-Thunks 发布包里的 Config/<x86|x64>/<系统版本>.txt（[dll名] 下面一行一个「序号=函数名」），
例如 Config/x86/5.1.2600.txt 是 Windows XP，6.0.6000.txt 是 Vista，6.1.7600.txt 是 Windows 7。
YY-Thunks 补上的函数链接后就不在导入表里了，所以这里只看最终导入表。
"""

import re
import sys


def load_exports(path):
    exports, cur = {}, None
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.strip()
            if line.startswith("[") and line.endswith("]"):
                cur = exports.setdefault(line[1:-1].lower(), set())
            elif cur is not None and "=" in line:
                cur.add(line.split("=", 1)[1].strip())
    return exports


def load_imports(path):
    """返回 [(dll, 函数名或 #序号)]，兼容 dumpbin 与 llvm-readobj 两种输出。"""
    imports, dll = [], None
    with open(path, encoding="utf-8", errors="replace") as f:
        lines = f.read().splitlines()
    if any(l.strip().startswith("Import {") for l in lines):
        for l in lines:
            s = l.strip()
            m = re.match(r"Name: (\S+)$", s)
            if m:
                dll = m.group(1)
                continue
            m = re.match(r"Symbol: (\S*) \((\d+)\)$", s)
            if m and dll:
                imports.append((dll, m.group(1) or "#" + m.group(2)))
        return imports
    section = False
    for l in lines:
        s = l.strip()
        if "following imports" in s or "following delay load imports" in s:
            section = True
            continue
        if not section or not s:
            continue
        if s == "Summary":
            break
        if re.match(r"^\S+\.(dll|drv|exe|ocx|cpl|sys)$", s, re.I) and l.startswith("    ") and not l.startswith("     "):
            dll = s
            continue
        if dll is None:
            continue
        m = re.match(r"^Ordinal\s+(\d+)$", s)
        if m:
            imports.append((dll, "#" + m.group(1)))
            continue
        m = re.match(r"^[0-9A-Fa-f]+\s+(\S+)$", s)
        if m and not re.search(r"(Import Address Table|Import Name Table|time date stamp|Index of first forwarder|Bound Import)", s):
            imports.append((dll, m.group(1)))
    return imports


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    exports = load_exports(sys.argv[2])
    imports = load_imports(sys.argv[1])
    if not imports:
        print("没读到任何导入，导入表文件格式不对？")
        return 2
    missing = []
    for dll, fn in imports:
        have = exports.get(dll.lower())
        if have is None:
            missing.append(f"{dll}（目标系统没有这个 DLL）: {fn}")
        elif not fn.startswith("#") and fn not in have:
            missing.append(f"{dll}: {fn}")
    dlls = sorted({d.lower() for d, _ in imports})
    print(f"导入 {len(imports)} 个函数，来自 {len(dlls)} 个 DLL：{' '.join(dlls)}")
    if missing:
        print(f"目标系统（{sys.argv[2]}）没有下面这些，程序在那里启动不了：")
        for m in missing:
            print("  " + m)
        return 1
    print("导入表检查通过：目标系统都有")
    return 0


if __name__ == "__main__":
    sys.exit(main())
