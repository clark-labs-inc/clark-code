#!/usr/bin/env python3
"""Generate a placeholder app icon (1024x1024 RGBA PNG) with no dependencies.

Charcoal rounded-square tile with a mint gradient ring — a stand-in mark until
real branding lands. Run: python3 scripts/gen_icon.py src-tauri/icon-source.png
"""
import struct
import sys
import zlib

S = 1024


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def rounded(x, y, x0, y0, x1, y1, r):
    """True if (x,y) is inside the rounded rect [x0,x1]x[y0,y1] with radius r."""
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    if x0 <= x <= x1 and y0 <= y <= y1:
        if (x - cx) ** 2 + (y - cy) ** 2 <= r * r or (
            x0 + r <= x <= x1 - r or y0 + r <= y <= y1 - r
        ):
            return True
    return False


CHARCOAL = (13, 17, 23)
MINT = (61, 215, 168)
TEAL = (43, 179, 137)

rows = []
cx, cy = S / 2, S / 2
for y in range(S):
    row = bytearray()
    row.append(0)  # PNG filter: none
    for x in range(S):
        # tile background (rounded square, full-ish bleed)
        if rounded(x, y, 96, 96, S - 96, S - 96, 200):
            a = 255
            # mint ring centered, charcoal interior
            d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
            if 250 <= d <= 330:
                t = (d - 250) / 80
                r, g, b = lerp(MINT, TEAL, t)
            elif d < 250:
                r, g, b = CHARCOAL
            else:
                # charcoal tile body with faint mint tint near edges
                r, g, b = CHARCOAL
            # notch to suggest a "C": carve the ring on the right wedge
            if 250 <= d <= 330 and (x - cx) > 60 and abs(y - cy) < 110:
                r, g, b = CHARCOAL
        else:
            r, g, b, a = 0, 0, 0, 0
        row += bytes((r, g, b, a))
    rows.append(bytes(row))

raw = b"".join(rows)
comp = zlib.compress(raw, 9)


def chunk(typ, data):
    body = typ + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", S, S, 8, 6, 0, 0, 0))
    + chunk(b"IDAT", comp)
    + chunk(b"IEND", b"")
)

out = sys.argv[1] if len(sys.argv) > 1 else "icon-source.png"
with open(out, "wb") as f:
    f.write(png)
print(f"wrote {out} ({len(png)} bytes)")
