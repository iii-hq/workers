"""Tiny, real screenshot images for tests, built with the standard library.

Each builder makes a complete, decodable image of one solid colour (`seed`
picks the colour, so two seeds give two different digests): a PNG with a
compressed IDAT, a baseline greyscale JPEG whose Huffman tables have one code
each (every 8x8 block is two zero bits), and a lossless WebP whose five prefix
codes have one symbol each (every pixel costs no bits). A 1080x2424 image is
a few hundred bytes to a few KiB, so no binary fixture is committed.
"""
from __future__ import annotations

import struct
import zlib


def png(width: int, height: int, seed: int = 0) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    pixel = bytes((seed * 37 % 256, seed * 59 % 256, seed * 83 % 256))
    row = b"\x00" + pixel * width
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(row * height, 9))
        + chunk(b"IEND", b"")
    )


def jpeg(width: int, height: int, seed: int = 0, *, progressive_marker: bool = False) -> bytes:
    """A baseline JPEG; `progressive_marker` labels the frame SOF2 (header tests only)."""
    def segment(marker: int, data: bytes) -> bytes:
        return bytes((0xFF, marker)) + struct.pack(">H", len(data) + 2) + data

    blocks = -(-width // 8) * -(-height // 8)
    bits = 2 * blocks  # DC category 0 ("0"), then end of block ("0")
    scan = bytes(bits // 8) + (bytes((0xFF >> (bits % 8),)) if bits % 8 else b"")
    one_code = bytes((1,) + (0,) * 15) + b"\x00"  # one code of length 1, symbol 0
    return (
        b"\xff\xd8"
        + segment(0xE0, b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00")
        + segment(0xFE, f"seed {seed}".encode())
        + segment(0xDB, b"\x00" + bytes((1,) * 64))
        + segment(0xC2 if progressive_marker else 0xC0, struct.pack(">BHHB", 8, height, width, 1) + b"\x01\x11\x00")
        + segment(0xC4, b"\x00" + one_code)
        + segment(0xC4, b"\x10" + one_code)
        + segment(0xDA, b"\x01\x01\x00\x00\x3f\x00")
        + scan
        + b"\xff\xd9"
    )


def webp(width: int, height: int, seed: int = 0) -> bytes:
    """A lossless (VP8L) WebP."""
    fields: list[tuple[int, int]] = [((width - 1) | (height - 1) << 14, 32)]
    fields += [(0, 1), (0, 1), (0, 1)]  # no transform, no colour cache, no meta prefix codes
    green, red, blue = seed * 37 % 256, seed * 59 % 256, seed * 83 % 256
    for symbol in (green, red, blue, 255):  # simple code, one symbol, eight bits
        fields += [(1, 1), (0, 1), (1, 1), (symbol, 8)]
    fields += [(1, 1), (0, 1), (0, 1), (0, 1)]  # distance: one symbol, one bit
    value = position = 0
    for field, size in fields:
        value |= field << position
        position += size
    data = b"\x2f" + value.to_bytes(-(-position // 8), "little")
    chunk = b"VP8L" + struct.pack("<I", len(data)) + data + (b"\x00" if len(data) % 2 else b"")
    return b"RIFF" + struct.pack("<I", 4 + len(chunk)) + b"WEBP" + chunk


def webp_lossy_header(width: int, height: int) -> bytes:
    """The first bytes of a lossy (VP8) WebP: enough for the header, not a picture."""
    frame = b"\x00\x00\x00" + b"\x9d\x01\x2a" + struct.pack("<HH", width, height) + bytes(8)
    return b"RIFF" + struct.pack("<I", 4 + 8 + len(frame)) + b"WEBP" + b"VP8 " + struct.pack("<I", len(frame)) + frame


def webp_extended_header(width: int, height: int) -> bytes:
    """The first bytes of an extended (VP8X) WebP: enough for the header, not a picture."""
    canvas = (width - 1).to_bytes(3, "little") + (height - 1).to_bytes(3, "little")
    data = b"\x10\x00\x00\x00" + canvas
    return b"RIFF" + struct.pack("<I", 4 + 8 + len(data) + 4) + b"WEBP" + b"VP8X" + struct.pack("<I", len(data)) + data + bytes(4)
