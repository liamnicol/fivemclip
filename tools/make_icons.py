#!/usr/bin/env python3
"""Render FiveMClip's icon set.

Hand-rasterised so the repo needs no image toolchain to rebuild its icons:
a dark rounded tile with a record dot, which is the one shape that still
reads at 16 pixels in a system tray.
"""
import math
import os
import struct
import zlib

BG = (0x14, 0x17, 0x21)
RING = (0xF2, 0xF4, 0xF8)
DOT = (0xE5, 0x3A, 0x35)
SS = 4  # supersampling factor


def render(size):
    n = size * SS
    px = [[(0, 0, 0, 0)] * n for _ in range(n)]
    radius = n * 0.22
    cx = cy = (n - 1) / 2.0
    ring_outer = n * 0.34
    ring_inner = n * 0.27
    dot_r = n * 0.19

    for y in range(n):
        for x in range(n):
            # Rounded-square coverage.
            dx = max(radius - x, x - (n - 1 - radius), 0)
            dy = max(radius - y, y - (n - 1 - radius), 0)
            if math.hypot(dx, dy) > radius:
                continue

            d = math.hypot(x - cx, y - cy)
            if d <= dot_r:
                px[y][x] = DOT + (255,)
            elif ring_inner <= d <= ring_outer:
                px[y][x] = RING + (255,)
            else:
                px[y][x] = BG + (255,)
    return downsample(px, size)


def downsample(px, size):
    out = bytearray()
    for y in range(size):
        out.append(0)  # PNG filter: none
        for x in range(size):
            r = g = b = a = 0
            for sy in range(SS):
                for sx in range(SS):
                    pr, pg, pb, pa = px[y * SS + sy][x * SS + sx]
                    r += pr * pa
                    g += pg * pa
                    b += pb * pa
                    a += pa
            if a:
                out += bytes((r // a, g // a, b // a, a // (SS * SS)))
            else:
                out += b"\x00\x00\x00\x00"
    return bytes(out)


def png(size, raw):
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def ico(entries):
    # Windows Vista and later read PNG payloads inside .ico directly, which
    # avoids hand-building a BMP with an AND mask for every size.
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    directory = b""
    payload = b""
    for size, blob in entries:
        directory += struct.pack(
            "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(blob), offset
        )
        offset += len(blob)
        payload += blob
    return header + directory + payload


def main():
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "icons")
    os.makedirs(out, exist_ok=True)

    blobs = {size: png(size, render(size)) for size in (16, 32, 48, 64, 128, 256)}

    for name, size in [
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 256),
    ]:
        with open(os.path.join(out, name), "wb") as f:
            f.write(blobs[size])

    with open(os.path.join(out, "icon.ico"), "wb") as f:
        f.write(ico([(s, blobs[s]) for s in (16, 32, 48, 64, 128, 256)]))

    print("wrote", sorted(os.listdir(out)))


if __name__ == "__main__":
    main()
