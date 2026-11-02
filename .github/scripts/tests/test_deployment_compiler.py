import json
from pathlib import Path

import pytest

import build_skills_payload
import deployment_compiler
import deployment_targets
import deployment_train
import worker_screenshots
from _screenshot_images import jpeg, png, webp


ROOT = Path(__file__).resolve().parents[3]


def test_canonical_numbers_match_json_stringify_representation():
    assert deployment_compiler.canonical_bytes(
        {
            "analysis": {"max_cost_usd": 2.0, "max_turns": 4, "ratio": 0.5},
            "negative_zero": -0.0,
        }
    ) == (
        b'{"analysis":{"max_cost_usd":2,"max_turns":4,"ratio":0.5},'
        b'"negative_zero":0}'
    )


def test_canonical_numbers_reject_non_json_values():
    with pytest.raises(ValueError):
        deployment_compiler.canonical_bytes({"budget": float("nan")})


@pytest.mark.parametrize(
    "config_file",
    [None, "", "# comments only\n", "{}\n", "null\n"],
    ids=["absent", "empty", "comments", "empty-mapping", "explicit-null"],
)
def test_normalize_config_emits_null_without_public_defaults(
    tmp_path: Path, config_file: str | None
) -> None:
    worker_dir = tmp_path / "smoke"
    worker_dir.mkdir()
    if config_file is not None:
        (worker_dir / "config.yaml").write_text(config_file, encoding="utf-8")

    assert deployment_compiler.normalize_config(worker_dir, {}) is None


def test_normalize_config_emits_null_for_explicit_inline_null(tmp_path: Path) -> None:
    worker_dir = tmp_path / "smoke"
    worker_dir.mkdir()

    assert deployment_compiler.normalize_config(worker_dir, {"config": None}) is None


def test_normalize_config_preserves_defaults_and_inline_file_precedence(tmp_path: Path) -> None:
    worker_dir = tmp_path / "smoke"
    worker_dir.mkdir()
    (worker_dir / "config.yaml").write_text(
        "source: file\nnested:\n  enabled: true\n", encoding="utf-8"
    )

    file_defaults = {"source": "file", "nested": {"enabled": True}}
    inline_defaults = {"source": "inline", "retries": 3}
    assert deployment_compiler.normalize_config(worker_dir, {}) == file_defaults
    assert deployment_compiler.normalize_config(
        worker_dir, {"config": inline_defaults}
    ) == inline_defaults
    # A non-null inline value wins even when it normalizes to public absence.
    assert deployment_compiler.normalize_config(worker_dir, {"config": {}}) is None
    # The historical explicit-null behavior falls back to config.yaml.
    assert deployment_compiler.normalize_config(
        worker_dir, {"config": None}
    ) == file_defaults


@pytest.mark.parametrize(
    ("manifest", "config_file"),
    [({"config": []}, None), ({}, "- invalid\n")],
    ids=["inline", "file"],
)
def test_normalize_config_rejects_invalid_shapes(
    tmp_path: Path, manifest: dict, config_file: str | None
) -> None:
    worker_dir = tmp_path / "smoke"
    worker_dir.mkdir()
    if config_file is not None:
        (worker_dir / "config.yaml").write_text(config_file, encoding="utf-8")

    with pytest.raises(ValueError, match="public config must be a mapping or null"):
        deployment_compiler.normalize_config(worker_dir, manifest)


@pytest.mark.parametrize(
    ("manifest", "config_file"),
    [
        ({"config": {"api_key": "released-secret"}}, None),
        ({}, "nested:\n  access_token: released-secret\n"),
    ],
    ids=["inline", "file"],
)
def test_normalize_config_rejects_secret_defaults_from_both_sources(
    tmp_path: Path, manifest: dict, config_file: str | None
) -> None:
    worker_dir = tmp_path / "smoke"
    worker_dir.mkdir()
    if config_file is not None:
        (worker_dir / "config.yaml").write_text(config_file, encoding="utf-8")

    with pytest.raises(ValueError, match="secret defaults cannot be released"):
        deployment_compiler.normalize_config(worker_dir, manifest)


def test_compiler_derives_registry_interface_capture_policy_for_every_worker():
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    descriptors = {
        worker: deployment_compiler.compile_worker(
            ROOT,
            worker,
            value,
            "a" * 40,
            "b" * 64,
        )
        for worker, value in catalog.items()
        if value.get("publish") is True
    }

    assert descriptors["acp"]["interface_capture"] == "skipped"
    assert descriptors["lsp"]["interface_capture"] == "skipped"
    assert descriptors["ios-simulator"]["interface_capture"] == "skipped"
    assert {
        worker
        for worker, descriptor in descriptors.items()
        if descriptor["interface_capture"] != "required"
    } == {"acp", "ios-simulator", "lsp"}
    assert descriptors["database"]["runtime"]["interface_config"] == {
        "path": "config.collect.yaml",
        "sha256": deployment_compiler.file_sha256(ROOT / "database" / "config.collect.yaml"),
    }
    assert descriptors["web"]["runtime"]["interface_config"] is None


def test_descriptor_schema_requires_explicit_interface_capture_policy():
    schema = json.loads(
        (ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(
            encoding="utf-8"
        )
    )

    assert "interface_capture" in schema["required"]
    assert schema["properties"]["interface_capture"] == {
        "enum": ["required", "skipped"]
    }


def test_kanban_rust_binary_builds_its_ui_with_the_workspace_pnpm_policy():
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]

    descriptor = deployment_compiler.compile_worker(
        ROOT,
        "kanban",
        catalog["kanban"],
        "a" * 40,
        "b" * 64,
    )

    artifact = descriptor["artifact"]
    assert artifact["kind"] == "rust-binary"
    assert artifact["binary"] == "kanban"
    assert {frontend["source_path"] for frontend in artifact["frontends"]} == {
        "kanban/ui", "kanban/native-ui",
    }
    workspace = deployment_compiler.read_yaml(ROOT / "pnpm-workspace.yaml")
    for frontend in artifact["frontends"]:
        assert frontend["workspace_root"] == "."
        assert frontend["install_command"] == ["pnpm", "install", "--frozen-lockfile"]
        assert frontend["build_command"] == ["pnpm", "run", "build"]
        assert frontend["outputs"] == ["dist"]
        assert frontend["source_path"] in workspace["packages"]
    assert workspace["allowBuilds"] == {"core-js": False, "esbuild": True, "protobufjs": False}


def test_registry_projection_carries_the_skills_payload():
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]

    def projection(worker: str) -> dict:
        return deployment_compiler.compile_worker(ROOT, worker, catalog[worker], "a" * 40, "b" * 64)[
            "registry_projection"
        ]

    kanban = projection("kanban")["skills"]
    assert kanban == build_skills_payload.collect_skills(ROOT / "kanban")
    assert "SKILL.md" in kanban
    assert "skills/tickets/index.md" in kanban
    assert "agents/tech-lead.md" in kanban
    # Worker-shipped profiles land in every project that installs the worker
    # (also as a kit dependency), so they stay out of the chat gallery (MOT-5437).
    for key, body in kanban.items():
        if key.startswith("agents/"):
            frontmatter = body.split("\n---", 1)[0] + "\n"
            assert "\nhidden: true\n" in frontmatter, key
    assert all(key.endswith(".md") and body.strip() for key, body in kanban.items())

    assert projection("acp")["skills"] == {}


def test_descriptor_schema_validates_the_skills_projection():
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads(
        (ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(
            encoding="utf-8"
        )
    )
    assert "skills" in schema["properties"]["registry_projection"]["required"]
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    validator = jsonschema.Draft202012Validator(schema)

    for worker in ("kanban", "acp"):
        validator.validate(deployment_compiler.compile_worker(ROOT, worker, catalog[worker], "a" * 40, "b" * 64))

    broken = deployment_compiler.compile_worker(ROOT, "kanban", catalog["kanban"], "a" * 40, "b" * 64)
    broken["registry_projection"]["skills"] = {"../escape.md": "x"}
    with pytest.raises(jsonschema.ValidationError):
        validator.validate(broken)


def test_descriptor_schema_accepts_null_and_real_public_config() -> None:
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads(
        (ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(
            encoding="utf-8"
        )
    )
    config_schema = schema["properties"]["registry_projection"]["properties"]["config"]
    assert config_schema == {"type": ["object", "null"]}
    validator = jsonschema.Draft202012Validator(schema)
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]

    without_defaults = deployment_compiler.compile_worker(
        ROOT, "acp", catalog["acp"], "a" * 40, "b" * 64
    )
    with_defaults = deployment_compiler.compile_worker(
        ROOT, "security-scan", catalog["security-scan"], "a" * 40, "b" * 64
    )

    assert "config" in without_defaults["registry_projection"]
    with_file_defaults = deployment_compiler.compile_worker(
        ROOT, "claude-code", catalog["claude-code"], "a" * 40, "b" * 64
    )
    assert without_defaults["registry_projection"]["config"] is None
    assert json.loads(json.dumps(without_defaults))["registry_projection"]["config"] is None
    assert with_defaults["registry_projection"]["config"] == {
        "repositories": [],
        "analysis": {
            "model": "",
            "max_turns": 4,
            "max_output_tokens": 8000,
            "max_total_tokens": 50000,
            "max_cost_usd": 2.0,
        },
    }
    assert with_file_defaults["registry_projection"]["config"] == deployment_compiler.read_yaml(
        ROOT / "claude-code" / "config.yaml"
    )
    validator.validate(without_defaults)
    validator.validate(with_defaults)
    validator.validate(with_file_defaults)


def test_binary_catalog_restores_intel_macos_except_the_microvm_sandbox():
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    checked = set()
    for worker, value in catalog.items():
        if value.get("publish") is not True or value["artifact"]["kind"] != "rust-binary":
            continue
        descriptor = deployment_compiler.compile_worker(ROOT, worker, value, "a" * 40, "b" * 64)
        targets = descriptor["artifact"]["targets"]
        assert "aarch64-apple-darwin" in targets, worker
        assert ("x86_64-apple-darwin" in targets) == (worker not in {"sandbox-code-runner", "sandbox-libkrun"}), worker
        assert set(deployment_targets.normalize_targets(targets)) == set(targets), worker
        assert {unit["target"] for unit in descriptor["build_units"]} == set(targets), worker
        checked.add(worker)
    assert {"state", "http", "queue", "pubsub", "cron", "ide", "code-runner", "sandbox-code-runner", "sandbox-libkrun"} <= checked


def test_rust_companions_are_validated_and_reach_the_schema():
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads(
        (ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(encoding="utf-8")
    )
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    compiled = deployment_compiler.compile_worker(ROOT, "judge-semif", catalog["judge-semif"], "a" * 40, "b" * 64)
    assert "libggml-vulkan.so" in compiled["artifact"]["companions"]
    jsonschema.Draft202012Validator(schema).validate(compiled)
    for broken in (["../escape.so"], ["lib/x.so"], [], [""], "libggml.so.0"):
        entry = json.loads(json.dumps(catalog["judge-semif"]))
        entry["artifact"]["companions"] = broken
        with pytest.raises(ValueError, match="companions"):
            deployment_compiler.compile_worker(ROOT, "judge-semif", entry, "a" * 40, "b" * 64)


def test_sandbox_libkrun_build_extras_are_validated_and_reach_the_schema():
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads((ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(encoding="utf-8"))
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    compiled = deployment_compiler.compile_worker(ROOT, "sandbox-libkrun", catalog["sandbox-libkrun"], "a" * 40, "b" * 64)
    art = compiled["artifact"]
    assert art["features"] == ["embed-init", "embed-libkrunfw"] and art["prebuild"] == ["scripts/build-init.sh"]
    assert art["macos_entitlements"] == "entitlements.plist"
    assert art["targets"] == ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
    jsonschema.Draft202012Validator(schema).validate(compiled)
    for field, broken in (("features", []), ("features", ["Bad Feature"]), ("prebuild", []),
                          ("prebuild", ["../escape.sh"]), ("macos_entitlements", "missing.plist")):
        entry = json.loads(json.dumps(catalog["sandbox-libkrun"])); entry["artifact"][field] = broken
        with pytest.raises(ValueError, match=field):
            deployment_compiler.compile_worker(ROOT, "sandbox-libkrun", entry, "a" * 40, "b" * 64)


def descriptor_schema() -> dict:
    return json.loads((ROOT / ".github" / "contracts" / "deployment-descriptor.schema.json").read_text(encoding="utf-8"))


def rust_worker(root: Path, name: str = "smoke") -> dict:
    worker = root / name
    worker.mkdir(parents=True)
    (worker / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "1.4.0"\nedition = "2021"\n')
    (worker / "README.md").write_text("# Smoke\n")
    (worker / "iii.worker.yaml").write_text(
        f"iii: v1\nname: {name}\nlanguage: rust\ndeploy: binary\nmanifest: Cargo.toml\n"
        "license: Apache-2.0\ndescription: Smoke worker\ntags: [smoke]\n"
    )
    return {
        "source": {"path": name, "package_manifest": "Cargo.toml"},
        "artifact": {
            "kind": "rust-binary", "binary": name,
            "toolchain": {"name": "rust", "version": "1.97.1"},
            "targets": ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"],
            "windows_exception": "This fixture only supports Unix.",
        },
        "publish": True,
    }


def write_screenshots(worker: Path, listing: str, files: dict[str, bytes]) -> None:
    folder = worker / "screenshots"
    folder.mkdir(exist_ok=True)
    for name, data in files.items():
        (folder / name).write_bytes(data)
    (folder / "screenshots.yaml").write_text(listing, encoding="utf-8")


SHOTS = {"android-board.webp": webp(1080, 2424, 1), "iphone-board.png": png(1320, 2868, 2), "mac-board.jpg": jpeg(2880, 1800, 3)}
LISTING = (
    "- file: android-board.webp\n  platform: android\n  caption: The board, one lane at a time\n"
    "- file: iphone-board.png\n  platform: iphone\n"
    "- file: mac-board.jpg\n  platform: mac\n  caption: The board on a Mac\n"
)


def test_screenshots_are_a_top_level_descriptor_field_only_when_listed(tmp_path: Path) -> None:
    value = rust_worker(tmp_path)
    absent = deployment_compiler.compile_worker(tmp_path, "smoke", value, "a" * 40, "b" * 64)
    assert "screenshots" not in absent

    write_screenshots(tmp_path / "smoke", "[]\n", {})
    cleared = deployment_compiler.compile_worker(tmp_path, "smoke", value, "a" * 40, "b" * 64)
    assert cleared["screenshots"] == []

    write_screenshots(tmp_path / "smoke", LISTING, SHOTS)
    listed = deployment_compiler.compile_worker(tmp_path, "smoke", value, "a" * 40, "b" * 64)
    assert [(shot["name"], shot["path"], shot["platform"], shot.get("caption")) for shot in listed["screenshots"]] == [
        ("android-1.webp", "smoke/screenshots/android-board.webp", "android", "The board, one lane at a time"),
        ("iphone-1.png", "smoke/screenshots/iphone-board.png", "iphone", None),
        ("mac-1.jpg", "smoke/screenshots/mac-board.jpg", "mac", "The board on a Mac"),
    ]
    assert listed["screenshots"] == worker_screenshots.collect(tmp_path / "smoke", root=tmp_path, kind="rust-binary", worker="smoke")

    # Never part of what Release Control spreads into POST /publish.
    for descriptor in (absent, cleared, listed):
        assert "screenshots" not in descriptor["registry_projection"]
        assert descriptor["registry_projection_sha256"] == absent["registry_projection_sha256"]
        assert deployment_train.verify_descriptor(json.loads(json.dumps(descriptor))) == descriptor
    assert len({absent["descriptor_sha256"], cleared["descriptor_sha256"], listed["descriptor_sha256"]}) == 3


def test_descriptor_schema_validates_the_screenshots(tmp_path: Path) -> None:
    jsonschema = pytest.importorskip("jsonschema")
    validator = jsonschema.Draft202012Validator(descriptor_schema())
    value = rust_worker(tmp_path)
    validator.validate(deployment_compiler.compile_worker(tmp_path, "smoke", value, "a" * 40, "b" * 64))
    write_screenshots(tmp_path / "smoke", "[]\n", {})
    validator.validate(deployment_compiler.compile_worker(tmp_path, "smoke", value, "a" * 40, "b" * 64))
    write_screenshots(tmp_path / "smoke", LISTING, SHOTS)
    listed = deployment_compiler.compile_worker(tmp_path, "smoke", value, "a" * 40, "b" * 64)
    validator.validate(listed)
    broken = json.loads(json.dumps(listed))
    broken["screenshots"][0]["caption"] = "x" * 201
    with pytest.raises(jsonschema.ValidationError):
        validator.validate(broken)


def test_the_descriptor_digest_follows_the_image_bytes(tmp_path: Path) -> None:
    value = rust_worker(tmp_path)
    write_screenshots(tmp_path / "smoke", LISTING, SHOTS)
    before = deployment_compiler.compile_worker(tmp_path, "smoke", value, "a" * 40, "b" * 64)
    (tmp_path / "smoke" / "screenshots" / "iphone-board.png").write_bytes(png(1320, 2868, 9))
    after = deployment_compiler.compile_worker(tmp_path, "smoke", value, "a" * 40, "b" * 64)

    assert after["descriptor_sha256"] != before["descriptor_sha256"]
    assert after["screenshots"][1]["sha256"] != before["screenshots"][1]["sha256"]
    assert after["screenshots"][0] == before["screenshots"][0]
    for unchanged in ("registry_projection_sha256", "public_manifest_sha256", "deployment_spec_sha256", "registry_projection"):
        assert after[unchanged] == before[unchanged], unchanged


def test_a_list_the_registry_would_refuse_fails_the_compile_naming_its_entry(tmp_path: Path) -> None:
    value = rust_worker(tmp_path)
    write_screenshots(tmp_path / "smoke", LISTING + "- file: tiny.png\n  platform: web\n", {**SHOTS, "tiny.png": png(100, 100)})
    with pytest.raises(ValueError, match=r"smoke/screenshots/screenshots.yaml: 1 problem\(s\)[^\n]*\n  \[3\] tiny.png: 100x100 pixels"):
        deployment_compiler.compile_worker(tmp_path, "smoke", value, "a" * 40, "b" * 64)


def test_image_workers_cannot_carry_screenshots(tmp_path: Path) -> None:
    worker = tmp_path / "img"
    worker.mkdir()
    (worker / "pyproject.toml").write_text('[project]\nname = "img"\nversion = "0.1.0"\n')
    (worker / "Dockerfile").write_text("FROM scratch\n")
    (worker / "iii.worker.yaml").write_text(
        "iii: v1\nname: img\nlanguage: python\ndeploy: image\nmanifest: pyproject.toml\n"
        "license: Apache-2.0\ndescription: Image worker\n"
    )
    value = {
        "source": {"path": "img", "package_manifest": "pyproject.toml"},
        "artifact": {"kind": "oci-image", "context": ".", "dockerfile": "Dockerfile", "platforms": ["linux/amd64", "linux/arm64"]},
        "publish": True,
    }
    assert "screenshots" not in deployment_compiler.compile_worker(tmp_path, "img", value, "a" * 40, "b" * 64)
    # `[]` clears what the version would inherit; the registry takes it without a release.
    write_screenshots(worker, "[]\n", {})
    assert deployment_compiler.compile_worker(tmp_path, "img", value, "a" * 40, "b" * 64)["screenshots"] == []
    write_screenshots(worker, "- file: board.png\n  platform: web\n", {"board.png": png(400, 400)})
    with pytest.raises(ValueError, match="oci-image workers cannot carry screenshots"):
        deployment_compiler.compile_worker(tmp_path, "img", value, "a" * 40, "b" * 64)


def test_the_worker_the_registry_hides_cannot_carry_screenshots(tmp_path: Path) -> None:
    # GET /w/iii answers 404, so Release Control could never read its screenshots back.
    value = rust_worker(tmp_path, "iii")
    write_screenshots(tmp_path / "iii", "[]\n", {})
    assert deployment_compiler.compile_worker(tmp_path, "iii", value, "a" * 40, "b" * 64)["screenshots"] == []
    write_screenshots(tmp_path / "iii", LISTING, SHOTS)
    with pytest.raises(ValueError, match=r"the registry hides the iii worker \(GET /w/iii answers 404\)"):
        deployment_compiler.compile_worker(tmp_path, "iii", value, "a" * 40, "b" * 64)


def test_compiler_digest_covers_the_screenshot_policy(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    policy = tmp_path / "worker_screenshots.py"
    policy.write_text("MAX_PER_PLATFORM = 10\n")
    monkeypatch.setattr(worker_screenshots, "__file__", str(policy))
    script = ROOT / ".github/scripts/deployment_compiler.py"
    schema = ROOT / ".github/contracts/deployment-descriptor.schema.json"
    before = deployment_compiler.compiler_digest(script, schema)
    policy.write_text("MAX_PER_PLATFORM = 11\n")
    assert deployment_compiler.compiler_digest(script, schema) != before


def test_every_catalog_worker_carries_screenshots_exactly_when_it_lists_them() -> None:
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]
    for worker, value in catalog.items():
        if value.get("publish") is not True:
            continue
        descriptor = deployment_compiler.compile_worker(ROOT, worker, value, "a" * 40, "b" * 64)
        listed = (ROOT / value["source"]["path"] / "screenshots" / "screenshots.yaml").is_file()
        assert ("screenshots" in descriptor) == listed, worker
        assert "screenshots" not in descriptor["registry_projection"], worker
