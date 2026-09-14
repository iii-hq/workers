#!/usr/bin/env python3
"""Generate PNG fallbacks from the canonical iii.dev favicon geometry."""

from __future__ import annotations

import binascii
import struct
import zlib
from pathlib import Path

SOURCE_WIDTH = 933.61
SOURCE_HEIGHT = 1050.31
SUPERSAMPLE = 4
TRANSPARENT = (0, 0, 0, 0)
BACKGROUND = (242, 240, 237, 255)
INK = (0, 0, 0, 255)

# Six rectangles from https://iii.dev/favicon.svg.
BRAND_RECTS = (
    (0.0, 0.0, 233.4, 233.4),
    (0.0, 350.1, 233.4, 1050.31),
    (350.1, 0.0, 583.5, 233.4),
    (350.1, 350.1, 583.5, 1050.31),
    (700.21, 0.0, 933.61, 233.4),
    (700.21, 350.1, 933.61, 1050.31),
)


def fill_rect(
    pixels: bytearray,
    size: int,
    bounds: tuple[int, int, int, int],
    color: tuple[int, int, int, int],
) -> None:
    """Fill one axis-aligned rectangle in an RGBA canvas."""
    x0, y0, x1, y1 = bounds
    row = bytes(color) * max(0, x1 - x0)
    for y in range(max(0, y0), min(size, y1)):
        start = (y * size + max(0, x0)) * 4
        pixels[start : start + len(row)] = row


def png_chunk(kind: bytes, data: bytes) -> bytes:
    """Encode one checksummed PNG chunk."""
    checksum = binascii.crc32(kind + data) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", checksum)


def render(
    target_size: int,
    output: Path,
    *,
    background: tuple[int, int, int, int],
    logo_height_ratio: float,
) -> None:
    """Render the official iii geometry to a square antialiased PNG."""
    size = target_size * SUPERSAMPLE
    pixels = bytearray(background * (size * size))
    logo_height = size * logo_height_ratio
    scale = logo_height / SOURCE_HEIGHT
    offset_x = (size - SOURCE_WIDTH * scale) / 2
    offset_y = (size - SOURCE_HEIGHT * scale) / 2

    for source_x0, source_y0, source_x1, source_y1 in BRAND_RECTS:
        bounds = (
            round(offset_x + source_x0 * scale),
            round(offset_y + source_y0 * scale),
            round(offset_x + source_x1 * scale),
            round(offset_y + source_y1 * scale),
        )
        fill_rect(pixels, size, bounds, INK)

    rows = []
    area = SUPERSAMPLE * SUPERSAMPLE
    for target_y in range(target_size):
        row = bytearray([0])
        for target_x in range(target_size):
            sums = [0, 0, 0, 0]
            for dy in range(SUPERSAMPLE):
                for dx in range(SUPERSAMPLE):
                    source_x = target_x * SUPERSAMPLE + dx
                    source_y = target_y * SUPERSAMPLE + dy
                    offset = (source_y * size + source_x) * 4
                    for channel in range(4):
                        sums[channel] += pixels[offset + channel]
            row.extend(round(value / area) for value in sums)
        rows.append(bytes(row))

    payload = b"\x89PNG\r\n\x1a\n"
    payload += png_chunk(
        b"IHDR",
        struct.pack(">IIBBBBB", target_size, target_size, 8, 6, 0, 0, 0),
    )
    payload += png_chunk(b"IDAT", zlib.compress(b"".join(rows), level=9))
    payload += png_chunk(b"IEND", b"")
    output.write_bytes(payload)


def main() -> None:
    """Write transparent fallbacks and safe-area launcher variants."""
    icon_dir = Path(__file__).resolve().parent.parent / "public" / "icons"
    icon_dir.mkdir(parents=True, exist_ok=True)
    for size, name in ((192, "iii-192.png"), (512, "iii-512.png")):
        render(
            size,
            icon_dir / name,
            background=TRANSPARENT,
            logo_height_ratio=1.0,
        )
    for size, name in (
        (512, "iii-maskable-512.png"),
        (180, "iii-apple-touch.png"),
    ):
        render(
            size,
            icon_dir / name,
            background=BACKGROUND,
            logo_height_ratio=0.62,
        )


if __name__ == "__main__":
    main()
