#!/usr/bin/env python3
"""Generate dependency-free PNG icons for the iii Console PWA."""

from __future__ import annotations

import binascii
import struct
import zlib
from pathlib import Path

BACKGROUND = (242, 240, 237)
INK = (10, 10, 10)
ACCENT = (184, 66, 15)
SUPERSAMPLE = 4


def rounded_rect(pixels: bytearray, size: int, x0: int, y0: int, x1: int, y1: int, radius: int, color: tuple[int, int, int]) -> None:
    for y in range(y0, y1):
        for x in range(x0, x1):
            cx = min(max(x, x0 + radius), x1 - radius - 1)
            cy = min(max(y, y0 + radius), y1 - radius - 1)
            if (x - cx) ** 2 + (y - cy) ** 2 <= radius**2:
                offset = (y * size + x) * 3
                pixels[offset : offset + 3] = bytes(color)


def circle(pixels: bytearray, size: int, cx: int, cy: int, radius: int, color: tuple[int, int, int]) -> None:
    for y in range(cy - radius, cy + radius + 1):
        for x in range(cx - radius, cx + radius + 1):
            if (x - cx) ** 2 + (y - cy) ** 2 <= radius**2:
                offset = (y * size + x) * 3
                pixels[offset : offset + 3] = bytes(color)


def chunk(kind: bytes, data: bytes) -> bytes:
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', binascii.crc32(kind + data) & 0xFFFFFFFF)


def render(target_size: int, output: Path) -> None:
    size = target_size * SUPERSAMPLE
    pixels = bytearray(BACKGROUND * (size * size))

    radius = round(size * 0.045)
    stem_top = round(size * 0.42)
    stem_bottom = round(size * 0.75)
    stem_width = round(size * 0.105)
    dot_y = round(size * 0.29)
    dot_radius = round(size * 0.055)

    for center, color in zip((0.32, 0.50, 0.68), (INK, ACCENT, INK), strict=True):
        center_x = round(size * center)
        rounded_rect(
            pixels,
            size,
            center_x - stem_width // 2,
            stem_top,
            center_x + stem_width // 2,
            stem_bottom,
            radius,
            color,
        )
        circle(pixels, size, center_x, dot_y, dot_radius, color)

    rows = []
    area = SUPERSAMPLE * SUPERSAMPLE
    for target_y in range(target_size):
        row = bytearray([0])
        for target_x in range(target_size):
            sums = [0, 0, 0]
            for dy in range(SUPERSAMPLE):
                for dx in range(SUPERSAMPLE):
                    source_x = target_x * SUPERSAMPLE + dx
                    source_y = target_y * SUPERSAMPLE + dy
                    offset = (source_y * size + source_x) * 3
                    for channel in range(3):
                        sums[channel] += pixels[offset + channel]
            row.extend(round(value / area) for value in sums)
        rows.append(bytes(row))

    payload = b'\x89PNG\r\n\x1a\n'
    payload += chunk(b'IHDR', struct.pack('>IIBBBBB', target_size, target_size, 8, 2, 0, 0, 0))
    payload += chunk(b'IDAT', zlib.compress(b''.join(rows), level=9))
    payload += chunk(b'IEND', b'')
    output.write_bytes(payload)


def main() -> None:
    icon_dir = Path(__file__).resolve().parent.parent / 'public' / 'icons'
    icon_dir.mkdir(parents=True, exist_ok=True)
    for size, name in (
        (192, 'icon-192.png'),
        (512, 'icon-512.png'),
        (512, 'icon-maskable-512.png'),
        (180, 'apple-touch-icon.png'),
    ):
        render(size, icon_dir / name)


if __name__ == '__main__':
    main()
