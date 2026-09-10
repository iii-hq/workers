import argparse
import json
from pathlib import Path

import pytest

import deployment_compiler


ROOT = Path(__file__).resolve().parents[3]


def write_worker(
    root: Path,
    selector: str,
    *,
    name: str,
    source_path: str | None = None,
    previous_names: list[str] | None = None,
    previous_source_paths: list[str] | None = None,
    dependencies: dict[str, str] | None = None,
) -> dict:
    source_path = source_path or selector
    worker_dir = root / source_path
    worker_dir.mkdir(parents=True)
    (worker_dir / "Cargo.toml").write_text(
        f'[package]\nname = "{selector}"\nversion = "1.0.0"\n', encoding="utf-8"
    )
    manifest = {
        "iii": "v1",
        "name": name,
        "language": "rust",
        "deploy": "binary",
        "manifest": "Cargo.toml",
        "bin": f"{selector}-bin",
        "description": selector,
        "license": "Apache-2.0",
    }
    if dependencies is not None:
        manifest["dependencies"] = dependencies
    (worker_dir / "iii.worker.yaml").write_text(json.dumps(manifest), encoding="utf-8")
    spec = {
        "source": {"path": source_path, "package_manifest": "Cargo.toml"},
        "artifact": {
            "kind": "rust-binary",
            "toolchain": {"name": "rust", "version": "1.97.1"},
            "binary": f"{selector}-bin",
            "targets": ["x86_64-pc-windows-msvc"],
        },
        "publish": True,
    }
    if previous_names is not None:
        spec["previous_names"] = previous_names
    if previous_source_paths is not None:
        spec["previous_source_paths"] = previous_source_paths
    return spec


def compile_temp_index(root: Path, workers: dict[str, dict]) -> dict:
    deploy = root / ".deploy"
    deploy.mkdir()
    (deploy / "workers.yaml").write_text(json.dumps({"workers": workers}), encoding="utf-8")
    output = root / "compiled"
    deployment_compiler.compile_index(argparse.Namespace(
        root=root,
        deployment_spec=Path(".deploy/workers.yaml"),
        source_sha="a" * 40,
        compiler_repository="iii-hq/workers",
        schema=ROOT / ".github/contracts/deployment-descriptor.schema.json",
        output_dir=output,
    ))
    return {
        path.stem: json.loads(path.read_text(encoding="utf-8"))
        for path in (output / "descriptors").glob("*.json")
    }


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
    assert {
        worker
        for worker, descriptor in descriptors.items()
        if descriptor["interface_capture"] != "required"
    } == {"acp", "lsp"}
    assert descriptors["database"]["runtime"]["interface_config"] == {
        "path": "config.collect.yaml",
        "sha256": deployment_compiler.file_sha256(ROOT / "database" / "config.collect.yaml"),
    }
    assert descriptors["web"]["runtime"]["interface_config"] is None
    assert descriptors["web"]["worker"] == "web"
    assert descriptors["web"]["registry_projection"]["worker_name"] == "web"
    assert "previous_names" not in descriptors["web"]
    assert "previous_source_paths" not in descriptors["web"]
    assert descriptors["shell"]["runtime"]["exec"] == ["ide"]
    assert descriptors["console"]["runtime"]["exec"] == ["ade"]


def test_compiler_separates_build_selector_from_public_identity_and_normalizes_aliases(tmp_path: Path):
    workers = {
        "consumer-selector": write_worker(
            tmp_path,
            "consumer-selector",
            name="consumer",
            dependencies={"oldest": "1.x"},
        ),
        "stable-selector": write_worker(
            tmp_path,
            "stable-selector",
            name="canonical",
            source_path="workers/current-location",
            previous_names=["oldest", "middle"],
            previous_source_paths=["workers/oldest", "workers/middle"],
        ),
    }

    descriptors = compile_temp_index(tmp_path, workers)
    target = descriptors["stable-selector"]

    assert target["worker"] == "stable-selector"
    assert target["registry_projection"]["worker_name"] == "canonical"
    assert target["previous_names"] == ["oldest", "middle"]
    assert target["previous_source_paths"] == ["workers/oldest", "workers/middle"]
    assert target["source"]["path"] == "workers/current-location"
    assert target["artifact"]["binary"] == "stable-selector-bin"
    assert target["runtime"]["exec"] == ["canonical"]
    assert target["registry_projection_sha256"] == deployment_compiler.json_sha256(
        target["registry_projection"]
    )
    assert descriptors["consumer-selector"]["registry_projection"]["dependencies"] == [
        {"name": "canonical", "version": "1.x"}
    ]


def test_empty_private_identity_history_is_omitted(tmp_path: Path):
    workers = {
        "selector": write_worker(
            tmp_path,
            "selector",
            name="public-name",
            previous_names=[],
            previous_source_paths=[],
        )
    }

    descriptor = compile_temp_index(tmp_path, workers)["selector"]

    assert "previous_names" not in descriptor
    assert "previous_source_paths" not in descriptor


@pytest.mark.parametrize(
    ("second_name", "second_aliases"),
    [("alpha", None), ("shared", None), ("beta", ["shared"])],
)
def test_compile_index_rejects_duplicate_or_ambiguous_public_names(
    tmp_path: Path, second_name: str, second_aliases: list[str] | None
):
    workers = {
        "one": write_worker(tmp_path, "one", name="alpha", previous_names=["shared"]),
        "two": write_worker(tmp_path, "two", name=second_name, previous_names=second_aliases),
    }

    with pytest.raises(ValueError, match="declared by both"):
        compile_temp_index(tmp_path, workers)


@pytest.mark.parametrize("aliases", [["current"], ["old", "old"]])
def test_previous_names_are_unique_and_exclude_current(tmp_path: Path, aliases: list[str]):
    workers = {
        "selector": write_worker(
            tmp_path, "selector", name="current", previous_names=aliases
        )
    }

    with pytest.raises(ValueError, match="unique and must not contain"):
        compile_temp_index(tmp_path, workers)


@pytest.mark.parametrize("aliases", [False, "", {}, [1]])
def test_previous_names_reject_non_array_values(tmp_path: Path, aliases):
    workers = {
        "selector": write_worker(
            tmp_path, "selector", name="current", previous_names=aliases
        )
    }

    with pytest.raises(ValueError, match="array of valid worker names"):
        compile_temp_index(tmp_path, workers)


@pytest.mark.parametrize("path", ["../old", "/old"])
def test_previous_source_paths_must_be_safe_but_need_not_exist(tmp_path: Path, path: str):
    workers = {
        "selector": write_worker(
            tmp_path, "selector", name="current", previous_source_paths=[path]
        )
    }

    with pytest.raises(ValueError, match="safe relative paths"):
        compile_temp_index(tmp_path, workers)


@pytest.mark.parametrize("paths", [False, "", {}, [1]])
def test_previous_source_paths_reject_non_array_values(tmp_path: Path, paths):
    workers = {
        "selector": write_worker(
            tmp_path, "selector", name="current", previous_source_paths=paths
        )
    }

    with pytest.raises(ValueError, match="array of safe relative paths"):
        compile_temp_index(tmp_path, workers)


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


def test_kanban_bundle_install_uses_its_pnpm_build_policy():
    catalog = deployment_compiler.read_yaml(ROOT / ".deploy" / "workers.yaml")["workers"]

    descriptor = deployment_compiler.compile_worker(
        ROOT,
        "kanban",
        catalog["kanban"],
        "a" * 40,
        "b" * 64,
    )

    assert descriptor["artifact"]["install_command"] == [
        "pnpm",
        "install",
        "--frozen-lockfile",
    ]
    assert descriptor["runtime"]["start"] == "node ./dist/bundle/index.mjs"
    policy = deployment_compiler.read_yaml(
        ROOT / descriptor["artifact"]["workspace_root"] / "pnpm-workspace.yaml"
    )["allowBuilds"]
    assert policy == {"esbuild": True, "protobufjs": False}
