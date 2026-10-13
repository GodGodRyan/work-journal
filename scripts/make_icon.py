#!/usr/bin/env python3
"""生成 1024x1024 应用图标（纯标准库，无第三方依赖）。
图案：蓝→靛渐变背景 + 白色圆形 + 蓝色对勾（寓意"完成记录"）。
用法：python3 scripts/make_icon.py [输出路径，默认 app-icon.png]
"""
import struct
import sys
import zlib

W = H = 1024


def chunk(tag: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


def blend(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


top = (37, 99, 235)     # blue-600
bottom = (99, 102, 241) # indigo-500
cx = cy = 512
R = 260  # 白色圆形半径

rows = []
for y in range(H):
    row = bytearray()
    t = y / (H - 1)
    bg = blend(top, bottom, t)
    for x in range(W):
        if (x - cx) ** 2 + (y - cy) ** 2 <= R * R:
            px = (255, 255, 255)
        else:
            px = bg
        row += bytes(px)
    rows.append(b"\x00" + bytes(row))  # 每行行首为滤波器字节（0 = None）

# 对勾（两段粗线段：点到线段距离 <= 34 即着色）
def seg_dist(px, py, x1, y1, x2, y2):
    dx, dy = x2 - x1, y2 - y1
    l2 = dx * dx + dy * dy
    t = max(0.0, min(1.0, ((px - x1) * dx + (py - y1) * dy) / l2))
    qx, qy = x1 + dx * t, y1 + dy * t
    return ((px - qx) ** 2 + (py - qy) ** 2) ** 0.5

segs = [((350, 520), (470, 640)), ((470, 640), (690, 400))]
color = (37, 99, 235)
for y in range(H):
    row = bytearray(rows[y])
    for x in range(W):
        if (x - cx) ** 2 + (y - cy) ** 2 > R * R:
            continue
        for (x1, y1), (x2, y2) in segs:
            if seg_dist(x, y, x1, y1, x2, y2) <= 34:
                row[1 + x * 3 : 1 + x * 3 + 3] = bytes(color)  # 跳过行首滤波器字节
                break
    rows[y] = bytes(row)

png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(b"".join(rows), 9))
    + chunk(b"IEND", b"")
)
out = sys.argv[1] if len(sys.argv) > 1 else "app-icon.png"
with open(out, "wb") as f:
    f.write(png)
print(f"已生成图标：{out}（{len(png)} bytes）")
