#!/usr/bin/env python3
"""把 PE 映像的校验和（OptionalHeader.CheckSum）算好写进去，并核对 XP 驱动的几项要求。

内核加载驱动时会核对校验和；链接器（lld）默认写 0。算法同 CheckSumMappedFile：
按 16 位小端字累加（进位折回），校验和字段本身按 0 算，最后加上文件长度。
"""
import struct
import sys


def checksum(data: bytes, off: int) -> int:
    s = 0
    n = len(data)
    padded = data + (b"\0" if n % 2 else b"")
    for i in range(0, len(padded), 2):
        if off <= i < off + 4:
            continue
        s += padded[i] | (padded[i + 1] << 8)
        s = (s & 0xFFFF) + (s >> 16)
    s = (s & 0xFFFF) + (s >> 16)
    return (s + n) & 0xFFFFFFFF


def main() -> int:
    for path in sys.argv[1:]:
        data = bytearray(open(path, "rb").read())
        pe = struct.unpack_from("<I", data, 0x3C)[0]
        assert data[pe : pe + 4] == b"PE\0\0", path
        machine = struct.unpack_from("<H", data, pe + 4)[0]
        opt = pe + 24
        magic = struct.unpack_from("<H", data, opt)[0]
        assert machine == 0x14C and magic == 0x10B, f"{path} 不是 32 位映像"
        sub_major, sub_minor = struct.unpack_from("<HH", data, opt + 48)
        subsystem = struct.unpack_from("<H", data, opt + 68)[0]
        assert subsystem == 1, f"{path} 子系统不是 native"
        assert (sub_major, sub_minor) == (5, 1), f"{path} 子系统版本 {sub_major}.{sub_minor}，XP 要 5.01"
        off = opt + 64
        struct.pack_into("<I", data, off, 0)
        struct.pack_into("<I", data, off, checksum(bytes(data), off))
        open(path, "wb").write(data)
        print(f"{path}: 校验和 {struct.unpack_from('<I', data, off)[0]:#010x}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
