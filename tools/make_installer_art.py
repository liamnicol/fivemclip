#!/usr/bin/env python3
"""Render the NSIS installer bitmaps.

NSIS wants uncompressed 24-bit BMPs at fixed sizes, which no ordinary image
tool emits by default, so they are written here directly. Same reason as the
icons: the repo should be able to rebuild its own art without an image
toolchain installed.

  header.bmp   150x57   shown in the top strip of every installer page
  sidebar.bmp  164x314  shown on the welcome and finish pages
"""
import math
import os
import struct

INK = (0x0E, 0x10, 0x16)      # page background, matches the app
INK_TOP = (0x1A, 0x1E, 0x2B)  # lighter end of the sidebar gradient
RING = (0xF2, 0xF4, 0xF8)
DOT = (0xE5, 0x3A, 0x35)
SS = 3  # supersampling factor


def blend(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def record_mark(px, cx, cy, radius, n_scale):
    """Draw the app's record dot: a red disc inside a light ring."""
    ring_outer = radius
    ring_inner = radius * 0.79
    dot_r = radius * 0.56

    x0, x1 = int(cx - radius - 2), int(cx + radius + 2)
    y0, y1 = int(cy - radius - 2), int(cy + radius + 2)
    for y in range(max(y0, 0), min(y1, len(px))):
        for x in range(max(x0, 0), min(x1, len(px[0]))):
            d = math.hypot(x - cx, y - cy)
            if d <= dot_r:
                px[y][x] = DOT
            elif ring_inner <= d <= ring_outer:
                px[y][x] = RING


def downsample(px, width, height):
    out = [[(0, 0, 0)] * width for _ in range(height)]
    for y in range(height):
        for x in range(width):
            r = g = b = 0
            for sy in range(SS):
                for sx in range(SS):
                    pr, pg, pb = px[y * SS + sy][x * SS + sx]
                    r += pr
                    g += pg
                    b += pb
            n = SS * SS
            out[y][x] = (r // n, g // n, b // n)
    return out


def write_bmp(path, px, width, height):
    # 24-bit BMP: rows are bottom-up, BGR, each padded to a 4-byte boundary.
    row_padding = (4 - (width * 3) % 4) % 4
    body = bytearray()
    for y in range(height - 1, -1, -1):
        for x in range(width):
            r, g, b = px[y][x]
            body += bytes((b, g, r))
        body += b"\x00" * row_padding

    info = struct.pack(
        "<IiiHHIIiiII", 40, width, height, 1, 24, 0, len(body), 2835, 2835, 0, 0
    )
    header = struct.pack("<2sIHHI", b"BM", 14 + 40 + len(body), 0, 0, 54)
    with open(path, "wb") as f:
        f.write(header + info + body)


def header_image(width=150, height=57):
    w, h = width * SS, height * SS
    px = [[INK] * w for _ in range(h)]

    # A thin red rule along the bottom, so the strip reads as branded rather
    # than as a missing image.
    for y in range(h - 2 * SS, h):
        for x in range(w):
            px[y][x] = DOT

    record_mark(px, w * 0.5, h * 0.46, h * 0.30, h)
    return downsample(px, width, height), width, height


def sidebar_image(width=164, height=314):
    w, h = width * SS, height * SS
    px = [[INK] * w for _ in range(h)]

    for y in range(h):
        # Gradient concentrated in the top third; the lower half stays flat so
        # the installer's own text sits on an even ground.
        t = min(y / (h * 0.6), 1.0)
        row = blend(INK_TOP, INK, t)
        for x in range(w):
            px[y][x] = row

    record_mark(px, w * 0.5, h * 0.30, w * 0.26, h)

    for y in range(h - 3 * SS, h):
        for x in range(w):
            px[y][x] = DOT
    return downsample(px, width, height), width, height


def main():
    out = os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "installer"
    )
    os.makedirs(out, exist_ok=True)

    for name, builder in (("header.bmp", header_image), ("sidebar.bmp", sidebar_image)):
        px, w, h = builder()
        path = os.path.join(out, name)
        write_bmp(path, px, w, h)
        print(f"wrote {name} ({w}x{h}, {os.path.getsize(path)} bytes)")


if __name__ == "__main__":
    main()
