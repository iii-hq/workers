"""The screenshots list a worker keeps in <worker>/screenshots/, read the way the Registry checks it."""
from __future__ import annotations

import hashlib
import os
from pathlib import Path

import pytest

import worker_screenshots as screenshots
from _screenshot_images import jpeg, png, webp, webp_extended_header, webp_lossy_header


def worker(root: Path, listing: str | None, files: dict[str, bytes] | None = None) -> Path:
    """`<root>/smoke` with `files` under `screenshots/` and the list, when given."""
    folder = root / "smoke" / "screenshots"
    folder.mkdir(parents=True)
    for name, data in (files or {}).items():
        (folder / name).parent.mkdir(parents=True, exist_ok=True)
        (folder / name).write_bytes(data)
    if listing is not None:
        (folder / "screenshots.yaml").write_text(listing, encoding="utf-8")
    return root / "smoke"


def collect(worker_dir: Path, kind: str = "rust-binary"):
    return screenshots.collect(worker_dir, root=worker_dir.parent, kind=kind)


def problems(worker_dir: Path, kind: str = "rust-binary") -> str:
    with pytest.raises(screenshots.ScreenshotError) as error:
        collect(worker_dir, kind)
    return str(error.value)


def test_headers_give_the_format_and_the_stored_pixel_size() -> None:
    assert screenshots.image_info(png(1320, 2868, 1)) == ("png", 1320, 2868)
    assert screenshots.image_info(jpeg(2880, 1800, 1)) == ("jpeg", 2880, 1800)
    assert screenshots.image_info(jpeg(1080, 2400, progressive_marker=True)) == ("jpeg", 1080, 2400)
    assert screenshots.image_info(webp(1080, 2424, 1)) == ("webp", 1080, 2424)
    assert screenshots.image_info(webp_lossy_header(1600, 900)) == ("webp", 1600, 900)
    assert screenshots.image_info(webp_extended_header(8192, 320)) == ("webp", 8192, 320)
    # A fill byte and a DHT (C4, inside the SOF range) before the frame header are skipped.
    filled = jpeg(640, 480)[:2] + b"\xff" + jpeg(640, 480)[2:]
    assert screenshots.image_info(filled) == ("jpeg", 640, 480)


def test_other_and_truncated_files_have_no_header() -> None:
    assert screenshots.image_info(b"GIF89a, not a screenshot at all.........") is None
    assert screenshots.image_info(png(400, 400)[:20]) is None
    assert screenshots.image_info(jpeg(400, 400)[:12]) is None
    assert screenshots.image_info(webp(400, 400)[:28]) is None
    assert screenshots.image_info(b"") is None
    # A JPEG whose scan starts before any frame header has no size to read.
    assert screenshots.image_info(b"\xff\xd8\xff\xda\x00\x08" + bytes(16)) is None


def test_a_list_becomes_descriptor_entries_named_per_platform_in_display_order(tmp_path: Path) -> None:
    images = {
        "android-board.webp": webp(1080, 2424, 1),
        "iphone-board.png": png(1320, 2868, 2),
        "shots/Mac Board.JPEG": jpeg(2880, 1800, 3),
        "iphone-ticket.png": png(1320, 2868, 4),
    }
    worker_dir = worker(tmp_path, (
        "# display order\n"
        "- file: android-board.webp\n  platform: android\n  caption: '  The board, one lane at a time  '\n"
        "- file: ./iphone-board.png\n  platform: iphone\n  caption: The board\n"
        "- file: shots/Mac Board.JPEG\n  platform: mac\n"
        "- file: iphone-ticket.png\n  platform: iphone\n  caption: '   '\n"
    ), images)

    shots = collect(worker_dir)

    assert [shot["name"] for shot in shots] == ["android-1.webp", "iphone-1.png", "mac-1.jpg", "iphone-2.png"]
    assert shots[0] == {
        "name": "android-1.webp",
        "path": "smoke/screenshots/android-board.webp",
        "platform": "android",
        "caption": "The board, one lane at a time",
        "sha256": hashlib.sha256(images["android-board.webp"]).hexdigest(),
        "size": len(images["android-board.webp"]),
        "width": 1080,
        "height": 2424,
    }
    assert shots[1]["path"] == "smoke/screenshots/iphone-board.png"
    assert shots[2]["path"] == "smoke/screenshots/shots/Mac Board.JPEG"
    assert (shots[2]["width"], shots[2]["height"]) == (2880, 1800)
    assert "caption" not in shots[2]
    assert "caption" not in shots[3], "a blank caption is none"
    assert screenshots.check_descriptor_entries(shots, kind="rust-binary", source_path="smoke") == shots


def test_no_list_inherits_and_an_empty_list_clears(tmp_path: Path) -> None:
    assert collect(worker(tmp_path / "absent", None)) is None
    # Images without a list are not screenshots of the version (a README may use them).
    assert collect(worker(tmp_path / "readme", None, {"board.png": png(400, 400)})) is None
    assert collect(worker(tmp_path / "empty", "[]\n")) == []
    no_folder = tmp_path / "bare" / "smoke"
    no_folder.mkdir(parents=True)
    assert collect(no_folder) is None


def test_an_empty_file_and_a_mapping_are_refused(tmp_path: Path) -> None:
    assert "lists no screenshots: write `[]`" in problems(worker(tmp_path / "a", "# nothing yet\n"))
    assert "must be a list of `- file: <image>` entries" in problems(worker(tmp_path / "b", "file: a.png\n"))
    assert "is not valid YAML" in problems(worker(tmp_path / "c", "- file: [unclosed\n"))


def test_every_problem_is_reported_at_once_with_the_index_of_its_entry(tmp_path: Path) -> None:
    listing = (
        "- file: watch.png\n  platform: watch\n"           # 0
        "- file: anim.gif\n  platform: web\n"              # 1
        "- file: missing.png\n  platform: web\n"           # 2
        "- file: fake.png\n  platform: web\n"              # 3 a JPEG
        "- file: small.png\n  platform: web\n"             # 4
        "- file: ok.png\n  platform: android\n"            # 5
        "- file: ok.png\n  platform: ipad\n"               # 6 the same bytes again
        f"- file: long.png\n  platform: web\n  caption: {'x' * 201}\n"  # 7
        "- file: typo.png\n  platfrom: web\n"              # 8
        "- file: number.png\n  platform: web\n  caption: 2024\n"       # 9
        "- file: noplatform.png\n"                         # 10
        "- file: huge-side.webp\n  platform: web\n"        # 11
        "- just a string\n"                                # 12
        "- file: ''\n  platform: web\n"                    # 13
    )
    worker_dir = worker(tmp_path, listing, {
        "watch.png": png(400, 400, 1),
        "anim.gif": b"GIF89a",
        "fake.png": jpeg(400, 400),
        "small.png": png(319, 400, 2),
        "ok.png": png(400, 400, 3),
        "long.png": png(400, 400, 4),
        "typo.png": png(400, 400, 5),
        "number.png": png(400, 400, 6),
        "noplatform.png": png(400, 400, 7),
        "huge-side.webp": webp_extended_header(8193, 400),
    })

    message = problems(worker_dir)

    for expected in (
        "smoke/screenshots/screenshots.yaml: 13 problem(s), the registry would refuse it",
        "[0] watch.png: platform `watch` is not one of iphone, ipad, mac, android, web",
        "[1] anim.gif: not a .png, .jpg, .jpeg or .webp file",
        "[2] missing.png: does not exist",
        "[3] fake.png: a JPEG image with a PNG name; rename it or convert it",
        "[4] small.png: 319x400 pixels; each side must be between 320 and 8192",
        "[6] ok.png: the same image as [5] ok.png; list an image once",
        "[7] long.png: the caption has 201 characters; at most 200",
        "[8]: unknown field(s) platfrom",
        "[9] number.png: the caption must be text (quote it)",
        "[10] noplatform.png: `platform` is missing",
        "[11] huge-side.webp: 8193x400 pixels",
        "[12]: an entry is a mapping",
        "[13]: `file` must be the path of an image",
    ):
        assert expected in message, f"missing {expected!r} in:\n{message}"
    assert "\n  [5]" not in message, "entry 5 is fine"


def test_files_stay_inside_the_folder_and_are_read_only_when_small_and_regular(tmp_path: Path) -> None:
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "secret.png").write_bytes(png(400, 400, 9))
    worker_dir = worker(tmp_path / "repo", (
        "- file: ../outside.png\n  platform: web\n"
        "- file: /etc/passwd.png\n  platform: web\n"
        "- file: folder.png\n  platform: web\n"
        "- file: huge.png\n  platform: web\n"
        "- file: zero.png\n  platform: web\n"
        "- file: link.png\n  platform: web\n"
        "- file: escape/secret.png\n  platform: web\n"
        "- file: 'back\\slash.png'\n  platform: web\n"
        "- file: ok.png\n  platform: web\n"
    ), {"ok.png": png(400, 400, 1)})
    folder = worker_dir / "screenshots"
    (folder / "folder.png").mkdir()
    # Sparse: 9 MiB on paper, refused from its metadata without being read.
    with (folder / "huge.png").open("wb") as handle:
        handle.truncate(screenshots.MAX_BYTES + 1024 * 1024)
    os.symlink("/dev/zero", folder / "zero.png")
    os.symlink("ok.png", folder / "link.png")
    os.symlink(outside, folder / "escape")

    message = problems(worker_dir)

    for expected in (
        "[0] ../outside.png: must be a path inside the folder of the list (no absolute path, no `..`)",
        "[1] /etc/passwd.png: must be a path inside the folder of the list",
        "[2] folder.png: is not a regular file",
        "[3] huge.png: 9437184 bytes; a screenshot is at most 8388608 bytes (8 MiB)",
        "[4] zero.png: must be a path inside the folder of the list",
        "[5] link.png: is not a regular file (a symbolic link",
        "[6] escape/secret.png: must be a path inside the folder of the list (it goes through a symbolic link)",
        "[7] back\\slash.png: must use `/` to separate folders",
    ):
        assert expected in message, f"missing {expected!r} in:\n{message}"
    assert message.startswith("smoke/screenshots/screenshots.yaml: 8 problem(s)"), message


def test_the_list_itself_must_be_a_regular_file_in_a_real_folder(tmp_path: Path) -> None:
    worker_dir = worker(tmp_path / "a", None, {"real.yaml": b"[]\n"})
    os.symlink("real.yaml", worker_dir / "screenshots" / "screenshots.yaml")
    assert "smoke/screenshots/screenshots.yaml must be a regular file" in problems(worker_dir)

    elsewhere = worker(tmp_path / "b", "[]\n")
    linked = tmp_path / "c" / "smoke"
    linked.mkdir(parents=True)
    os.symlink(elsewhere / "screenshots", linked / "screenshots")
    assert "smoke/screenshots must be a folder, not a symbolic link" in problems(linked)


def test_at_most_ten_per_platform_and_fifty_per_version(tmp_path: Path) -> None:
    files = {f"{i}.png": png(400, 800, i) for i in range(11)}
    listing = "".join(f"- file: {i}.png\n  platform: android\n" for i in range(11))
    message = problems(worker(tmp_path / "platform", listing, files))
    assert "[10] 10.png: more than 10 screenshots for android" in message
    assert message.startswith("smoke/screenshots/screenshots.yaml: 1 problem(s)"), message

    full = "".join(f"- file: {i}.png\n  platform: {platform}\n" for platform in screenshots.PLATFORMS for i in range(10))
    many = {f"{i}.png": png(320, 320, i) for i in range(10)}
    assert "lists 51 screenshots; at most 50 per version" in problems(
        worker(tmp_path / "version", full + "- file: 0.png\n  platform: web\n", many)
    )


def test_all_the_images_of_a_version_are_capped_together(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    files = {"a.png": png(400, 400, 1), "b.png": png(400, 400, 2)}
    worker_dir = worker(tmp_path, "- file: a.png\n  platform: web\n- file: b.png\n  platform: web\n", files)
    total = sum(len(data) for data in files.values())
    monkeypatch.setattr(screenshots, "MAX_TOTAL_BYTES", total)
    assert len(collect(worker_dir)) == 2
    monkeypatch.setattr(screenshots, "MAX_TOTAL_BYTES", total - 1)
    assert f"the screenshots total {total} bytes" in problems(worker_dir)


def test_captions_count_code_points_and_are_trimmed_like_the_registry_trims_them(tmp_path: Path) -> None:
    emoji = "\N{CAMERA}" * 200
    worker_dir = worker(tmp_path / "ok", (
        f"- file: a.png\n  platform: web\n  caption: \"{emoji}\"\n"
        # JavaScript's trim removes U+FEFF and keeps U+0085 (NEL); the registry trims with it.
        "- file: b.png\n  platform: web\n  caption: \"\\ufeff\\u3000Board\\u0085\"\n"
    ), {"a.png": png(400, 400, 1), "b.png": png(400, 400, 2)})
    first, second = collect(worker_dir)
    assert first["caption"] == emoji
    assert second["caption"] == "Board\u0085"
    too_long = worker(tmp_path / "long", f"- file: a.png\n  platform: web\n  caption: \"{emoji}x\"\n", {"a.png": png(400, 400)})
    assert "the caption has 201 characters; at most 200" in problems(too_long)


def test_image_workers_cannot_carry_screenshots_but_may_clear_them(tmp_path: Path) -> None:
    listed = worker(tmp_path / "listed", "- file: a.png\n  platform: web\n", {"a.png": png(400, 400)})
    message = problems(listed, kind="oci-image")
    assert "oci-image workers cannot carry screenshots" in message
    assert "write `[]` to clear" in message
    # The registry accepts `screenshots: []` on a publish without a release.
    assert collect(worker(tmp_path / "cleared", "[]\n"), "oci-image") == []
    for kind in sorted(screenshots.KINDS):
        assert collect(worker(tmp_path / kind, "[]\n"), kind) == []


def compiled(tmp_path: Path) -> list[dict]:
    worker_dir = worker(tmp_path / "compiled", (
        "- file: a.png\n  platform: iphone\n  caption: Board\n- file: b.webp\n  platform: android\n"
    ), {"a.png": png(400, 800, 1), "b.webp": webp(1080, 2424, 2)})
    return collect(worker_dir)


@pytest.mark.parametrize(("field", "value", "message"), [
    ("name", "iphone-1.gif", "name must be a unique"),
    ("name", "android-1.png", "name must be a unique"),
    ("path", "other/screenshots/a.png", "path must be a normalized path inside smoke/screenshots/"),
    ("path", "smoke/screenshots/../../etc/a.png", "path must be a normalized path"),
    ("path", "smoke/screenshots", "path must be a normalized path"),
    ("caption", " Board", "caption must be trimmed text"),
    ("caption", "", "caption must be trimmed text"),
    ("sha256", "A" * 64, "sha256 must be a unique lowercase SHA-256"),
    ("size", True, "size must be 1 to"),
    ("size", screenshots.MAX_BYTES + 1, "size must be 1 to"),
    ("width", 319, "each side must be"),
    ("height", 8192.0, "each side must be"),
    ("extra", 1, "fields differ from the compiler contract"),
])
def test_a_descriptor_cannot_carry_entries_the_registry_would_refuse(tmp_path: Path, field: str, value, message: str) -> None:
    entries = compiled(tmp_path)
    entries[0][field] = value
    with pytest.raises(screenshots.ScreenshotError, match=message):
        screenshots.check_descriptor_entries(entries, kind="rust-binary", source_path="smoke")


def test_a_descriptor_entry_list_is_checked_as_a_whole(tmp_path: Path) -> None:
    entries = compiled(tmp_path)
    with pytest.raises(screenshots.ScreenshotError, match="oci-image releases cannot carry screenshots"):
        screenshots.check_descriptor_entries(entries, kind="oci-image", source_path="smoke")
    assert screenshots.check_descriptor_entries([], kind="oci-image", source_path="smoke") == []
    with pytest.raises(screenshots.ScreenshotError, match="sha256 must be a unique"):
        screenshots.check_descriptor_entries([entries[0], {**entries[1], "sha256": entries[0]["sha256"]}], kind="go-binary", source_path="smoke")
    with pytest.raises(screenshots.ScreenshotError, match="name must be a unique"):
        screenshots.check_descriptor_entries([entries[0], entries[0]], kind="go-binary", source_path="smoke")
    with pytest.raises(screenshots.ScreenshotError, match="at most 50 entries"):
        screenshots.check_descriptor_entries({"not": "a list"}, kind="go-binary", source_path="smoke")
    assert screenshots.check_descriptor_entries([], kind="python-bundle", source_path="smoke") == []


def test_verify_file_accepts_only_the_exact_bytes_a_descriptor_entry_names(tmp_path: Path) -> None:
    entry = compiled(tmp_path)[1]
    copy = tmp_path / "android-1.webp"
    copy.write_bytes(webp(1080, 2424, 2))
    assert screenshots.verify_file(copy, entry) == copy.read_bytes()
    copy.write_bytes(webp(1080, 2424, 3))
    with pytest.raises(screenshots.ScreenshotError, match="differs from the descriptor"):
        screenshots.verify_file(copy, entry)
    copy.write_bytes(webp(1080, 2424, 2) + b"\x00")
    with pytest.raises(screenshots.ScreenshotError, match="the descriptor says"):
        screenshots.verify_file(copy, entry)
    with pytest.raises(screenshots.ScreenshotError, match="does not exist"):
        screenshots.verify_file(tmp_path / "missing.webp", entry)
    # The right bytes under a descriptor that declares another pixel size are not the image it names.
    copy.write_bytes(webp(1080, 2424, 2))
    with pytest.raises(screenshots.ScreenshotError, match="not the 1080x2400 webp image"):
        screenshots.verify_file(copy, {**entry, "height": 2400})
