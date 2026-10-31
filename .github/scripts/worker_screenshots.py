#!/usr/bin/env python3
"""The screenshots of a worker version: ``<worker>/screenshots/screenshots.yaml``.

A worker keeps its screenshots next to its source, in ``<worker>/screenshots/``:
the images and ``screenshots.yaml``, the list the registry CLI reads with
``registry publish --screenshots`` (iii-hq/registry ``docs/SCREENSHOTS.md``).
The list is the display order; each entry is ``file`` (a path inside the folder
of the list), ``platform`` and an optional ``caption``:

- no ``screenshots.yaml``: the next version keeps the previous version's;
- ``[]``: the next version has none;
- entries: those images, in that order.

The deployment compiler reads every image once (``collect``), checks it against
the registry's limits and records it in the descriptor's top-level
``screenshots``. The limits are the registry's (``api/src/lib/screenshots.ts``),
mirrored here the way the CLI (``cli/src/screenshots.rs``) mirrors them, so a
list the registry would refuse fails when the descriptor index is compiled,
never when Release Control publishes. The build carries the bytes to
``build-<source_sha>`` and the build manifest (``check_descriptor_entries``,
``verify_file``); Release Control registers them on the version's registry
release and lists them in ``POST /publish``.

Standard library only at import time: the deploy train imports this module on
release runners that have no PyYAML (``collect`` imports it when it runs).
"""

from __future__ import annotations

import fnmatch
import hashlib
import os
import re
import stat
from pathlib import Path, PurePosixPath
from typing import Any

# Device families the registry knows, in the order its worker page offers them.
PLATFORMS = ("iphone", "ipad", "mac", "android", "web")
MIB = 1024 * 1024
# Bytes per image.
MAX_BYTES = 8 * MIB
# Bytes of all the images of one version.
MAX_TOTAL_BYTES = 128 * MIB
# Images per platform in one version, and in one version (every platform full).
MAX_PER_PLATFORM = 10
MAX_PER_VERSION = MAX_PER_PLATFORM * len(PLATFORMS)
# Pixels, each side.
MIN_SIDE = 320
MAX_SIDE = 8192
# Characters (Unicode code points) of a caption.
MAX_CAPTION_CHARS = 200

# Where a worker keeps them, relative to its source folder.
FOLDER = "screenshots"
LIST_FILE = "screenshots.yaml"

# Release kinds Release Control stages on a registry-hosted release (`binary`,
# `bundle`). An `oci-image` version is published without one, and the registry
# refuses listed screenshots there (`422 screenshots_require_release`): an image
# worker may only clear them with `[]`.
KINDS = frozenset({"rust-binary", "go-binary", "javascript-bundle", "python-bundle"})

# Workers the registry hides from every page (iii-hq/registry
# `api/src/lib/hidden-workers.ts`, `HIDDEN_WORKER_SLUGS`): `GET /w/{worker}`
# answers 404, so their screenshots could never be shown nor read back, and
# Release Control refuses a list for them. They may only clear with `[]`.
HIDDEN_WORKERS = frozenset({"iii"})

ENTRY_FIELDS = frozenset({"file", "platform", "caption"})
DESCRIPTOR_FIELDS = frozenset({"name", "path", "platform", "caption", "sha256", "size", "width", "height"})
DESCRIPTOR_REQUIRED = DESCRIPTOR_FIELDS - {"caption"}
# `<platform>-<n>.<ext>`, the name the CLI gives an image on the registry.
NAME_RE = re.compile(r"^(iphone|ipad|mac|android|web)-([1-9][0-9]*)\.(png|jpg|webp)$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")

FORMAT_LABEL = {"png": "PNG", "jpeg": "JPEG", "webp": "WebP"}
# The format a file name promises by its extension.
EXTENSION_FORMAT = {"png": "png", "jpg": "jpeg", "jpeg": "jpeg", "webp": "webp"}
# The extension an image is published with (`jpg` for any JPEG).
PUBLISHED_EXTENSION = {"png": "png", "jpeg": "jpg", "webp": "webp"}

# What JavaScript's `String.prototype.trim` removes. The registry trims captions
# with it (and stores the result), so the descriptor carries the caption exactly
# as the registry will answer it.
_JS_WHITESPACE = (
    "\t\n\v\f\r \u00a0\u1680"
    + "".join(map(chr, range(0x2000, 0x200B)))  # U+2000-U+200A
    + "\u2028\u2029\u202f\u205f\u3000\ufeff"
)


class ScreenshotError(ValueError):
    """A list, an image or a descriptor entry the registry would refuse."""


# ---------------------------------------------------------------- image headers


def _be16(b: bytes, at: int) -> int:
    return (b[at] << 8) | b[at + 1]


def _le16(b: bytes, at: int) -> int:
    return b[at] | (b[at + 1] << 8)


def _le24(b: bytes, at: int) -> int:
    return _le16(b, at) | (b[at + 2] << 16)


def _png(b: bytes) -> tuple[str, int, int] | None:
    if len(b) < 24 or b[:8] != b"\x89PNG\r\n\x1a\n" or b[12:16] != b"IHDR":
        return None
    # The first chunk is always IHDR: width and height are its first two fields.
    return "png", int.from_bytes(b[16:20], "big"), int.from_bytes(b[20:24], "big")


def _jpeg(b: bytes) -> tuple[str, int, int] | None:
    if len(b) < 4 or b[0] != 0xFF or b[1] != 0xD8:
        return None
    i = 2
    while i + 3 < len(b):
        if b[i] != 0xFF:
            return None
        marker = b[i + 1]
        if marker == 0xFF:  # fill byte
            i += 1
            continue
        # Markers without a length: TEM and RST0-7.
        if marker == 0x01 or 0xD0 <= marker <= 0xD7:
            i += 2
            continue
        # End of image or start of scan before any frame header: no size to read.
        if marker in (0xD9, 0xDA):
            return None
        length = _be16(b, i + 2)
        if length < 2:
            return None
        # SOF0-SOF15 carry the frame size; C4 (DHT), C8 (JPG) and CC (DAC) share the range.
        if 0xC0 <= marker <= 0xCF and marker not in (0xC4, 0xC8, 0xCC):
            if i + 9 > len(b):
                return None
            return "jpeg", _be16(b, i + 7), _be16(b, i + 5)
        i += 2 + length
    return None


def _webp(b: bytes) -> tuple[str, int, int] | None:
    if len(b) < 30 or b[0:4] != b"RIFF" or b[8:12] != b"WEBP":
        return None
    chunk = b[12:16]
    # Lossy: a 3-byte frame tag, the start code 9d 01 2a, then 14-bit sizes.
    if chunk == b"VP8 " and b[23:26] == b"\x9d\x01\x2a":
        return "webp", _le16(b, 26) & 0x3FFF, _le16(b, 28) & 0x3FFF
    # Lossless: signature 0x2f, then width-1 and height-1 as two 14-bit fields.
    if chunk == b"VP8L" and b[20] == 0x2F:
        bits = int.from_bytes(b[21:25], "little")
        return "webp", (bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1
    # Extended: canvas width-1 and height-1 as 24-bit little-endian fields.
    if chunk == b"VP8X":
        return "webp", _le24(b, 24) + 1, _le24(b, 27) + 1
    return None


def image_info(data: bytes) -> tuple[str, int, int] | None:
    """Format (`png`, `jpeg`, `webp`), width and height from the file header, or None.

    The size is the one stored in the file (a JPEG EXIF orientation is not
    applied), which is also what the registry reads back from the upload.
    """
    return _png(data) or _jpeg(data) or _webp(data)


def declared_format(name: str) -> str | None:
    """The format a file name promises by its extension, case-insensitively."""
    stem, dot, extension = PurePosixPath(name).name.lower().rpartition(".")
    if not dot or not stem:
        return None
    return EXTENSION_FORMAT.get(extension)


def trim_caption(caption: str) -> str | None:
    """The caption as the registry stores it: trimmed like JavaScript, blank is none."""
    return caption.strip(_JS_WHITESPACE) or None


# ---------------------------------------------------------------- files


def read_regular(path: Path, *, limit: int = MAX_BYTES) -> bytes:
    """The bytes of a regular file of at most `limit` bytes, checked before it is read.

    A symbolic link (even to a regular file), a folder or a device is refused
    from its metadata, and so is a file above the limit, without reading it.
    """
    try:
        status = path.lstat()
    except FileNotFoundError:
        raise ScreenshotError("does not exist") from None
    except OSError as error:
        raise ScreenshotError(f"could not be read: {error.strerror or error}") from None
    except ValueError as error:  # e.g. an embedded NUL
        raise ScreenshotError(f"could not be read: {error}") from None
    if not stat.S_ISREG(status.st_mode):
        raise ScreenshotError("is not a regular file (a symbolic link, a folder or a device is refused)")
    if status.st_size > limit:
        raise ScreenshotError(f"{status.st_size} bytes; a screenshot is at most {limit} bytes (8 MiB)")
    with path.open("rb") as handle:
        data = handle.read(limit + 1)
    if len(data) > limit:
        raise ScreenshotError(f"{len(data)} bytes; a screenshot is at most {limit} bytes (8 MiB)")
    return data


def verify_file(path: Path, entry: dict[str, Any]) -> bytes:
    """The bytes at `path` when they are exactly the image a descriptor entry names.

    Size (checked before reading), SHA-256, the format the published name
    promises and the pixel size must all be the entry's.
    """
    try:
        status = path.lstat()
    except FileNotFoundError:
        raise ScreenshotError(f"{path} does not exist") from None
    except (OSError, ValueError) as error:
        raise ScreenshotError(f"{path} could not be read: {error}") from None
    if not stat.S_ISREG(status.st_mode):
        raise ScreenshotError(f"{path} is not a regular file")
    if status.st_size != entry["size"]:
        raise ScreenshotError(f"{path} is {status.st_size} bytes, the descriptor says {entry['size']}")
    data = read_regular(path)
    if len(data) != entry["size"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
        raise ScreenshotError(f"{path} differs from the descriptor (sha256 {hashlib.sha256(data).hexdigest()})")
    info = image_info(data)
    expected = (declared_format(str(entry["name"])), entry["width"], entry["height"])
    if info != expected:
        raise ScreenshotError(f"{path} is not the {expected[1]}x{expected[2]} {expected[0]} image the descriptor names")
    return data


def _inside(folder: Path, file: str) -> tuple[Path, PurePosixPath]:
    """`file` as a path inside the folder of the list: no absolute path, no `..`."""
    if "\0" in file:
        raise ScreenshotError("must not contain a NUL character")
    if "\\" in file:
        raise ScreenshotError("must use `/` to separate folders")
    pure = PurePosixPath(file)
    if pure.is_absolute() or ".." in pure.parts or not pure.parts:
        raise ScreenshotError("must be a path inside the folder of the list (no absolute path, no `..`)")
    return folder.joinpath(*pure.parts), PurePosixPath(*pure.parts)


# ---------------------------------------------------------------- the list


def list_path(worker_dir: Path) -> Path:
    return worker_dir / FOLDER / LIST_FILE


def _strict_loader(yaml: Any) -> type:
    """A SafeLoader that refuses a duplicate key and a merge key (`<<`) in a mapping.

    PyYAML keeps the last of two equal keys and applies `<<` merges silently;
    an entry is three plain fields, so either is a mistake to report. (The
    registry CLI also refuses `<<`, as an unknown field.)
    """

    class StrictLoader(yaml.SafeLoader):
        pass

    def mapping(loader: Any, node: Any, deep: bool = False) -> Any:
        seen: list[Any] = []
        for key_node, _ in node.value:
            if key_node.tag == "tag:yaml.org,2002:merge":
                raise yaml.constructor.ConstructorError(
                    "while constructing a mapping",
                    node.start_mark,
                    "merge keys (`<<`) are not accepted",
                    key_node.start_mark,
                )
            key = loader.construct_object(key_node, deep=True)
            if key in seen:
                raise yaml.constructor.ConstructorError(
                    "while constructing a mapping",
                    node.start_mark,
                    f"found duplicate key {key!r}",
                    key_node.start_mark,
                )
            seen.append(key)
        return loader.construct_mapping(node, deep=deep)

    StrictLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, mapping)
    return StrictLoader


def _shown(file: str) -> str:
    """`file` for a message: as written, or quoted when it holds control characters."""
    return file if file.isprintable() else repr(file)


def collect(worker_dir: Path, *, root: Path, kind: str, worker: str) -> list[dict[str, Any]] | None:
    """The descriptor's `screenshots` for one worker, or None when it has no list.

    Every problem is reported at once, each with the index of its entry, in
    one `ScreenshotError`. Entries are named `<platform>-<n>.<ext>` like the
    registry CLI names them; `path` is relative to the repository root.
    `screenshots/` without a list may hold other images (a README's, say);
    a list under another name (`screenshots.yml`) is refused, never ignored,
    because ignoring it would let the version silently inherit.
    """
    root = root.resolve()
    folder = worker_dir.resolve() / FOLDER
    listing = folder / LIST_FILE
    label = listing.relative_to(root).as_posix()
    shown_folder = folder.relative_to(root).as_posix()
    if folder.is_symlink():
        raise ScreenshotError(f"{shown_folder} must be a folder, not a symbolic link")
    if not os.path.lexists(folder):
        return None
    if not folder.is_dir():
        raise ScreenshotError(f"{shown_folder} must be a folder holding {LIST_FILE} and its images")
    misnamed = sorted(
        path.name
        for path in folder.iterdir()
        if path.name != LIST_FILE and fnmatch.fnmatchcase(path.name.lower(), "screenshots.y*ml")
    )
    if misnamed:
        raise ScreenshotError(f"{shown_folder}/{misnamed[0]} is not read: the list must be named exactly {LIST_FILE}")
    if not os.path.lexists(listing):
        return None
    if listing.is_symlink() or not listing.is_file():
        raise ScreenshotError(f"{label} must be a regular file")
    try:
        import yaml
    except ImportError as error:  # pragma: no cover - the compiler requires PyYAML
        raise ScreenshotError("PyYAML is required to read screenshots.yaml") from error
    try:
        document = yaml.load(listing.read_text(encoding="utf-8"), Loader=_strict_loader(yaml))
    except (yaml.YAMLError, UnicodeDecodeError) as error:
        raise ScreenshotError(f"{label} is not valid YAML: {error}") from None
    if document is None:
        raise ScreenshotError(
            f"{label} lists no screenshots: write `[]` to publish the next version without screenshots, "
            "or delete the file to keep the previous version's"
        )
    if not isinstance(document, list):
        raise ScreenshotError(
            f"{label} must be a list of `- file: <image>` entries with a `platform` "
            f"({', '.join(PLATFORMS)}) and an optional `caption`"
        )
    if document and kind not in KINDS:
        raise ScreenshotError(
            f"{label}: {kind} workers cannot carry screenshots. The registry hosts a version's screenshots "
            "on its release, and an image version is published without one; write `[]` to clear the ones "
            "a version would inherit, or remove the list"
        )
    if document and worker in HIDDEN_WORKERS:
        raise ScreenshotError(
            f"{label}: the registry hides the {worker} worker (GET /w/{worker} answers 404), so its "
            "screenshots could never be shown nor read back; write `[]`, or remove the list"
        )
    if len(document) > MAX_PER_VERSION:
        raise ScreenshotError(f"{label} lists {len(document)} screenshots; at most {MAX_PER_VERSION} per version")

    base = PurePosixPath(folder.relative_to(root).as_posix())
    problems: list[str] = []
    shots: list[dict[str, Any]] = []
    per_platform: dict[str, int] = {}
    # sha256 -> the first entry with those bytes, so the same image under two names is caught.
    seen: dict[str, str] = {}
    for index, entry in enumerate(document):
        at = f"[{index}]"
        if not isinstance(entry, dict):
            problems.append(f"{at}: an entry is a mapping of `file`, `platform` and an optional `caption`")
            continue
        unknown = sorted(str(key) for key in entry if key not in ENTRY_FIELDS)
        if unknown:
            problems.append(
                f"{at}: unknown field(s) {', '.join(unknown)} "
                "(an entry is `file`, `platform` and an optional `caption`)"
            )
            continue
        file = entry.get("file")
        if not isinstance(file, str) or not file:
            problems.append(f"{at}: `file` must be the path of an image")
            continue
        at = f"[{index}] {_shown(file)}"
        try:
            shot = _check(folder, base, entry, file, per_platform)
        except ScreenshotError as error:
            problems.append(f"{at}: {error}")
            continue
        first = seen.get(shot["sha256"])
        if first is not None:
            problems.append(f"{at}: the same image as {first}; list an image once")
            continue
        seen[shot["sha256"]] = at
        shots.append(shot)
    total = sum(shot["size"] for shot in shots)
    if total > MAX_TOTAL_BYTES:
        problems.append(f"the screenshots total {total} bytes; at most {MAX_TOTAL_BYTES} (128 MiB) in all")
    if problems:
        raise ScreenshotError(
            f"{label}: {len(problems)} problem(s), the registry would refuse it:\n  " + "\n  ".join(problems)
        )
    return shots


def _check(
    folder: Path, base: PurePosixPath, entry: dict[str, Any], file: str, per_platform: dict[str, int]
) -> dict[str, Any]:
    """One entry: the platform, the caption, then the file itself."""
    platform = entry.get("platform")
    if "platform" not in entry:
        raise ScreenshotError(f"`platform` is missing (one of {', '.join(PLATFORMS)})")
    if platform not in PLATFORMS:
        raise ScreenshotError(f"platform `{platform}` is not one of {', '.join(PLATFORMS)}")
    caption = entry.get("caption")
    if caption is not None:
        if not isinstance(caption, str):
            raise ScreenshotError("the caption must be text (quote it)")
        caption = trim_caption(caption)
        if caption is not None and len(caption) > MAX_CAPTION_CHARS:
            raise ScreenshotError(f"the caption has {len(caption)} characters; at most {MAX_CAPTION_CHARS}")
    declared = declared_format(file)
    if declared is None:
        raise ScreenshotError("not a .png, .jpg, .jpeg or .webp file")
    path, relative = _inside(folder, file)
    if not path.resolve().is_relative_to(folder.resolve()):
        raise ScreenshotError("must be a path inside the folder of the list (it goes through a symbolic link)")
    data = read_regular(path)
    info = image_info(data)
    if info is None:
        raise ScreenshotError(f"not a {FORMAT_LABEL[declared]} image (its header is not one)")
    actual, width, height = info
    if actual != declared:
        raise ScreenshotError(
            f"a {FORMAT_LABEL[actual]} image with a {FORMAT_LABEL[declared]} name; rename it or convert it"
        )
    if min(width, height) < MIN_SIDE or max(width, height) > MAX_SIDE:
        raise ScreenshotError(f"{width}x{height} pixels; each side must be between {MIN_SIDE} and {MAX_SIDE}")
    count = per_platform.get(platform, 0) + 1
    if count > MAX_PER_PLATFORM:
        raise ScreenshotError(f"more than {MAX_PER_PLATFORM} screenshots for {platform}")
    per_platform[platform] = count
    shot: dict[str, Any] = {
        "name": f"{platform}-{count}.{PUBLISHED_EXTENSION[actual]}",
        "path": (base / relative).as_posix(),
        "platform": platform,
    }
    if caption is not None:
        shot["caption"] = caption
    shot.update(sha256=hashlib.sha256(data).hexdigest(), size=len(data), width=width, height=height)
    return shot


# ---------------------------------------------------------------- the descriptor field


def _whole(value: object, low: int, high: int) -> bool:
    return type(value) is int and low <= value <= high


def check_descriptor_entries(value: object, *, kind: str, worker: str, source_path: str) -> list[dict[str, Any]]:
    """The shape of a compiled descriptor's `screenshots`, without reading any file.

    The release train runs this on every descriptor it accepts, so a hand-made
    or stale descriptor cannot carry screenshots the registry would refuse, nor
    name a file outside the worker's own `screenshots/` folder.
    """
    folder = PurePosixPath(source_path) / FOLDER
    if not isinstance(value, list) or len(value) > MAX_PER_VERSION:
        raise ScreenshotError(f"must be an array of at most {MAX_PER_VERSION} entries")
    if value and kind not in KINDS:
        raise ScreenshotError(f"{kind} releases cannot carry screenshots (only `[]`, which clears them)")
    if value and worker in HIDDEN_WORKERS:
        raise ScreenshotError(f"the registry hides the {worker} worker: it cannot carry screenshots (only `[]`)")
    digests: set[str] = set()
    per_platform: dict[str, int] = {}
    total = 0
    for index, entry in enumerate(value):
        at = f"screenshots[{index}]"
        if not isinstance(entry, dict) or not DESCRIPTOR_REQUIRED <= set(entry) <= DESCRIPTOR_FIELDS:
            raise ScreenshotError(f"{at} fields differ from the compiler contract")
        name, platform = entry["name"], entry["platform"]
        if platform not in PLATFORMS:
            raise ScreenshotError(f"{at}.platform must be one of {', '.join(PLATFORMS)}")
        # `n` counts from 1 per platform in display order, as the CLI numbers
        # them and Release Control requires; it also makes every name unique.
        count = per_platform.get(platform, 0) + 1
        per_platform[platform] = count
        match = NAME_RE.fullmatch(name) if isinstance(name, str) else None
        if match is None or match.group(1) != platform or int(match.group(2)) != count:
            raise ScreenshotError(
                f"{at}.name must be {platform}-{count}.<png|jpg|webp>: numbered from 1 per platform in display order"
            )
        if count > MAX_PER_PLATFORM:
            raise ScreenshotError(f"{at}: more than {MAX_PER_PLATFORM} screenshots for {platform}")
        path = entry["path"]
        pure = PurePosixPath(path) if isinstance(path, str) and path and "\\" not in path and "\0" not in path else None
        if (
            pure is None
            or pure.is_absolute()
            or ".." in pure.parts
            or pure.as_posix() != path
            or not pure.is_relative_to(folder)
            or pure == folder
        ):
            raise ScreenshotError(f"{at}.path must be a normalized path inside {folder.as_posix()}/")
        if "caption" in entry:
            caption = entry["caption"]
            if not isinstance(caption, str) or trim_caption(caption) != caption or len(caption) > MAX_CAPTION_CHARS:
                raise ScreenshotError(f"{at}.caption must be trimmed text of 1 to {MAX_CAPTION_CHARS} characters")
        sha256 = entry["sha256"]
        if not isinstance(sha256, str) or not SHA256_RE.fullmatch(sha256) or sha256 in digests:
            raise ScreenshotError(f"{at}.sha256 must be a unique lowercase SHA-256")
        digests.add(sha256)
        if not _whole(entry["size"], 1, MAX_BYTES):
            raise ScreenshotError(f"{at}.size must be 1 to {MAX_BYTES} bytes")
        if not _whole(entry["width"], MIN_SIDE, MAX_SIDE) or not _whole(entry["height"], MIN_SIDE, MAX_SIDE):
            raise ScreenshotError(f"{at}: each side must be {MIN_SIDE} to {MAX_SIDE} pixels")
        total += entry["size"]
    if total > MAX_TOTAL_BYTES:
        raise ScreenshotError(f"the screenshots total {total} bytes; at most {MAX_TOTAL_BYTES} (128 MiB)")
    return value
