#!/usr/bin/env python3
"""Generate the ucli AppImage/desktop icon as a 256x256 RGBA PNG.

Pure stdlib (zlib + struct) so there is no imagemagick/rsvg dependency. Draws a
tokyo-night-ish dark rounded panel with a terminal ">" prompt and an underscore
cursor — a clean, recognizable "CLI" mark.

Usage:  python3 make-icon.py <out.png> [size]
"""
import sys, zlib, struct


def lerp(a, b, t):
    return int(a + (b - a) * t)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "ucli.png"
    size = int(sys.argv[2]) if len(sys.argv) > 2 else 256
    W = H = size

    # palette (tokyo-night inspired)
    bg_top = (26, 27, 38)      # #1a1b26
    bg_bot = (15, 17, 23)      # #0f1117
    panel = (36, 40, 59)       # #24283b
    accent = (122, 162, 247)   # #7aa2f7
    accent2 = (187, 154, 247)  # #bb9af7

    def in_rounded(x, y, m, r):
        """True if (x,y) is inside the panel inset by m with corner radius r."""
        x0, y0, x1, y1 = m, m, W - m, H - m
        if x < x0 + r and y < y0 + r:
            dx, dy = x0 + r - x, y0 + r - y
            return dx * dx + dy * dy <= r * r
        if x > x1 - r and y < y0 + r:
            dx, dy = x - (x1 - r), y0 + r - y
            return dx * dx + dy * dy <= r * r
        if x < x0 + r and y > y1 - r:
            dx, dy = x0 + r - x, y - (y1 - r)
            return dx * dx + dy * dy <= r * r
        if x > x1 - r and y > y1 - r:
            dx, dy = x - (x1 - r), y - (y1 - r)
            return dx * dx + dy * dy <= r * r
        return x0 <= x <= x1 and y0 <= y <= y1

    def in_triangle(px, py, v):
        """Barycentric point-in-triangle test for vertices v=[(x,y)x3]."""
        (x0, y0), (x1, y1), (x2, y2) = v
        d = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2)
        if d == 0:
            return False
        a = ((y1 - y2) * (px - x2) + (x2 - x1) * (py - y2)) / d
        b = ((y2 - y0) * (px - x2) + (x0 - x2) * (py - y2)) / d
        c = 1 - a - b
        return a >= 0 and b >= 0 and c >= 0

    margin = size // 8
    radius = size // 7
    # ">" prompt triangle: a right-pointing chevron centered-left.
    p = size * 5 // 8
    tri = [(W * 30 // 100, H * 38 // 100),
           (W * 30 // 100, H * 62 // 100),
           (W * 50 // 100, H * 50 // 100)]
    # underscore cursor block under/after the prompt.
    cur = (W * 54 // 100, H * 60 // 100, W * 70 // 100, H * 65 // 100)

    raw = bytearray()
    for y in range(H):
        raw.append(0)  # PNG filter type 0 (None) for this scanline
        for x in range(W):
            # vertical gradient background
            t = y / max(H - 1, 1)
            r, g, b = lerp(bg_top[0], bg_bot[0], t), lerp(bg_top[1], bg_bot[1], t), lerp(bg_top[2], bg_bot[2], t)
            a = 255
            if in_rounded(x, y, margin, radius):
                r, g, b = panel
                # title bar band at the top of the panel
                if y - margin < size // 14:
                    r, g, b = lerp(panel[0], 255, 0.04), lerp(panel[1], 255, 0.04), lerp(panel[2], 255, 0.04)
                # three little window dots in the title bar
                dot_y = margin + size // 28
                for dx, col in ((margin + size // 12, (245, 189, 230)),
                               (margin + size // 7, (224, 160, 122)),
                               (margin + size // 5, (122, 162, 247))):
                    if (x - dx) ** 2 + (y - dot_y) ** 2 <= (size // 64) ** 2:
                        r, g, b = col
                # prompt chevron
                if in_triangle(x, y, tri):
                    r, g, b = accent
                # underscore cursor
                if cur[0] <= x <= cur[2] and cur[1] <= y <= cur[3]:
                    r, g, b = accent2
            raw += bytes((r, g, b, a))

    def chunk(typ, data):
        c = typ + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0)  # 8-bit RGBA
    idat = zlib.compress(bytes(raw), 9)
    png = sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")
    with open(out, "wb") as f:
        f.write(png)
    print(f"wrote {out} ({len(png)} bytes, {W}x{H} RGBA)")


if __name__ == "__main__":
    main()
