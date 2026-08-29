import argparse
import json
import tarfile
import zipfile
from pathlib import Path

import pytest

import release_train
from release_train import (
    build,
    build_frontends,
    build_oci_layout,
    frontend_metadata,
    normalized_tar,
    normalized_zip,
    select_descriptor,
)


def descriptor(worker: str, source_sha: str, version: str, digest: str) -> dict:
    return {
        "contract": "release-descriptor",
        "worker": worker,
        "version": version,
        "source_sha": source_sha,
        "descriptor_sha256": digest,
        "package": {},
        "build_units": [{"id": "bundle", "kind": "javascript-bundle"}],
    }


def test_select_descriptor_preserves_compiler_bytes(tmp_path: Path) -> None:
    source_sha = "a" * 40
    compiler_sha = "b" * 40
    digest = "c" * 64
    descriptor_dir = tmp_path / "descriptors"
    descriptor_dir.mkdir()
    selected = descriptor("smoke", source_sha, "1.0.0-rc.1", digest)
    descriptor_bytes = (json.dumps(selected, indent=4) + "\n").encode()
    (descriptor_dir / "smoke.json").write_bytes(descriptor_bytes)
    (tmp_path / "release-descriptor-index.json").write_text(json.dumps({
        "contract": "release-descriptor-index",
        "source_sha": source_sha,
        "compiler_sha": compiler_sha,
        "workers": {"smoke": {
            "path": "descriptors/smoke.json",
            "digest": digest,
            "version": "1.0.0-rc.1",
            "publish": True,
        }},
    }), encoding="utf-8")
    output = tmp_path / "selected.json"

    select_descriptor(argparse.Namespace(
        index_dir=tmp_path,
        worker="smoke",
        source_sha=source_sha,
        compiler_sha=compiler_sha,
        candidate_version="1.0.0-rc.1",
        descriptor_sha256=digest,
        out=output,
    ))

    assert output.read_bytes() == descriptor_bytes


def test_normalized_tar_preflights_every_include_before_creating_archive(tmp_path: Path) -> None:
    source = tmp_path / "source.txt"
    source.write_text("ok", encoding="utf-8")
    directory = tmp_path / "directory"
    directory.mkdir()
    destination = tmp_path / "artifact.tar.gz"

    with pytest.raises(SystemExit, match="not a regular file"):
        normalized_tar([(source, "source.txt"), (directory, "directory")], destination)

    assert not destination.exists()


def test_normalized_tar_rejects_symlink_includes(tmp_path: Path) -> None:
    source = tmp_path / "source.txt"
    source.write_text("ok", encoding="utf-8")
    link = tmp_path / "link.txt"
    link.symlink_to(source)

    with pytest.raises(SystemExit, match="not a regular file"):
        normalized_tar([(link, "link.txt")], tmp_path / "artifact.tar.gz")


def test_normalized_zip_rejects_symlink_includes(tmp_path: Path) -> None:
    source = tmp_path / "source.exe"
    source.write_bytes(b"binary")
    link = tmp_path / "link.exe"
    link.symlink_to(source)

    with pytest.raises(SystemExit, match="not a regular file"):
        normalized_zip([(link, "worker.exe")], tmp_path / "artifact.zip")


@pytest.mark.parametrize(
    ("target", "archive_name", "member_name"),
    [
        ("x86_64-unknown-linux-gnu", "smoke-x86_64-unknown-linux-gnu.tar.gz", "smoke"),
        ("x86_64-pc-windows-msvc", "smoke-x86_64-pc-windows-msvc.zip", "smoke.exe"),
    ],
)
def test_rust_build_uses_deterministic_target_native_archive(
    tmp_path: Path,
    monkeypatch,
    target: str,
    archive_name: str,
    member_name: str,
) -> None:
    source_sha = "a" * 40
    digest = "c" * 64
    worker_dir = tmp_path / "smoke"
    worker_dir.mkdir()
    (worker_dir / "Cargo.toml").write_text("[package]\nname='smoke'\nversion='0.1.0'\n", encoding="utf-8")
    binary = worker_dir / "target" / target / "release" / member_name
    binary.parent.mkdir(parents=True)
    binary.write_bytes(b"immutable executable\n")
    binary.chmod(0o755)
    selected = descriptor("smoke", source_sha, "1.0.0-rc.1", digest)
    selected["package"] = {
        "source": {"path": "smoke", "package_manifest": "Cargo.toml"},
        "artifact": {
            "kind": "rust-binary",
            "binary": "smoke",
            "targets": [target],
            "toolchain": {"name": "rust", "version": "1.97.1"},
        },
    }
    selected["build_units"] = [{"id": f"rust-{target}", "kind": "rust-binary", "target": target}]
    descriptor_path = tmp_path / "release-descriptor.json"
    descriptor_path.write_text(json.dumps(selected), encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(release_train.subprocess, "check_output", lambda *args, **kwargs: source_sha + "\n")
    commands = []
    monkeypatch.setattr(release_train, "run", lambda *command, cwd=None: commands.append((command, cwd)))

    outputs = []
    for output_name in ("first", "second"):
        output = tmp_path / output_name
        build(argparse.Namespace(descriptor=descriptor_path, unit=f"rust-{target}", out=output))
        outputs.append(output)

    first_archive = outputs[0] / archive_name
    second_archive = outputs[1] / archive_name
    assert first_archive.read_bytes() == second_archive.read_bytes()
    checksum_name = archive_name.removesuffix(".tar.gz").removesuffix(".zip") + ".sha256"
    assert (outputs[0] / checksum_name).read_text(encoding="utf-8") == (
        f"{release_train.sha256(first_archive)}  {archive_name}\n"
    )
    assert commands == [
        (("cargo", "build", "--release", "--target", target, "--manifest-path", "smoke/Cargo.toml"), None),
        (("cargo", "build", "--release", "--target", target, "--manifest-path", "smoke/Cargo.toml"), None),
    ]

    if archive_name.endswith(".zip"):
        with zipfile.ZipFile(first_archive) as archive:
            assert archive.namelist() == [member_name]
            info = archive.getinfo(member_name)
            assert info.date_time == (1980, 1, 1, 0, 0, 0)
            assert (info.external_attr >> 16) & 0o777 == 0o755
            assert archive.read(member_name) == binary.read_bytes()
    else:
        with tarfile.open(first_archive, mode="r:gz") as archive:
            member = archive.getmember(member_name)
            assert member.mtime == 0
            assert member.uid == member.gid == 0
            assert member.mode == 0o755
            extracted = archive.extractfile(member)
            assert extracted is not None and extracted.read() == binary.read_bytes()


def test_javascript_bundle_preserves_compiler_owned_include_path(tmp_path: Path, monkeypatch) -> None:
    source_sha = "a" * 40
    digest = "c" * 64
    source = tmp_path / "worker"
    bundle = source / "dist" / "bundle" / "index.mjs"
    bundle.parent.mkdir(parents=True)
    bundle.write_text("export default true;\n", encoding="utf-8")
    (tmp_path / "pnpm-lock.yaml").write_text("lockfileVersion: '9.0'\n", encoding="utf-8")
    selected = descriptor("worker", source_sha, "1.0.0-rc.1", digest)
    selected["package"] = {
        "source": {"path": "worker", "package_manifest": "package.json"},
        "artifact": {
            "kind": "javascript-bundle",
            "workspace_root": ".",
            "runtime": {"name": "node", "version": "22.20.0"},
            "package_manager": {"name": "pnpm", "version": "11.13.1"},
            "lockfile": "pnpm-lock.yaml",
            "install_command": ["true"],
            "build_command": ["true"],
            "include": ["dist/bundle/index.mjs"],
        },
        "runtime": {"exec": ["node", "./dist/bundle/index.mjs"]},
    }
    descriptor_path = tmp_path / "release-descriptor.json"
    descriptor_path.write_text(json.dumps(selected), encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(release_train.subprocess, "check_output", lambda *args, **kwargs: source_sha + "\n")
    monkeypatch.setattr(release_train, "run", lambda *args, **kwargs: None)

    output = tmp_path / "output"
    build(argparse.Namespace(descriptor=descriptor_path, unit="bundle", out=output))

    with tarfile.open(output / "worker.tar.gz", mode="r:gz") as archive:
        assert archive.getnames() == ["dist/bundle/index.mjs"]
        extracted = archive.extractfile("dist/bundle/index.mjs")
        assert extracted is not None and extracted.read() == bundle.read_bytes()


def test_oci_build_uses_reproducible_exporter_and_reports_index_digest(tmp_path: Path, monkeypatch) -> None:
    context = tmp_path / "context"
    context.mkdir()
    dockerfile = context / "Dockerfile"
    dockerfile.write_text("FROM scratch\n", encoding="utf-8")
    destination = tmp_path / "layout"
    calls = []

    def fake_run(*command: str, cwd: Path | None = None) -> None:
        calls.append((command, cwd))
        destination.mkdir()
        (destination / "index.json").write_bytes(b'{"manifests":[]}')

    monkeypatch.setattr(release_train, "run", fake_run)
    digest = build_oci_layout(
        context=context,
        dockerfile=dockerfile,
        platforms=["linux/amd64", "linux/arm64"],
        destination=destination,
    )

    assert digest == "sha256:" + release_train.sha256(destination / "index.json")
    assert calls == [(('docker', 'buildx', 'build',
                       '--provenance=false', '--build-arg', 'SOURCE_DATE_EPOCH=0',
                       '--platform', 'linux/amd64,linux/arm64', '--file', str(dockerfile),
                       '--output', f'type=oci,tar=false,dest={destination},rewrite-timestamp=true',
                       str(context)), None)]


def test_descriptor_frontends_install_once_and_build_each_explicit_output(tmp_path: Path, monkeypatch) -> None:
    (tmp_path / "pnpm-lock.yaml").write_text("lockfileVersion: '9.0'\n", encoding="utf-8")
    for source in ("one/ui", "two/ui"):
        (tmp_path / source).mkdir(parents=True)
    selected = descriptor("smoke", "a" * 40, "1.0.0-rc.1", "c" * 64)
    selected["package"] = {
        "artifact": {
            "kind": "rust-binary",
            "frontends": [
                {
                    "workspace_root": ".",
                    "source_path": source,
                    "runtime": {"name": "node", "version": "22"},
                    "package_manager": {"name": "pnpm", "version": "11.13.1"},
                    "lockfile": "pnpm-lock.yaml",
                    "install_command": ["pnpm", "install", "--frozen-lockfile"],
                    "build_command": ["pnpm", "run", "build"],
                    "outputs": ["dist"],
                }
                for source in ("one/ui", "two/ui")
            ],
        },
    }
    descriptor_path = tmp_path / "descriptor.json"
    descriptor_path.write_text(json.dumps(selected), encoding="utf-8")
    github_output = tmp_path / "github-output"
    monkeypatch.chdir(tmp_path)

    frontend_metadata(argparse.Namespace(descriptor=descriptor_path, github_output=github_output))
    outputs = dict(line.split("=", 1) for line in github_output.read_text().splitlines())
    assert outputs["count"] == "2"
    assert outputs["runtime_version"] == "22"
    assert outputs["package_manager_version"] == "11.13.1"
    assert len(outputs["lock_sha256"]) == 64

    calls = []

    def fake_run(*command: str, cwd: Path | None = None) -> None:
        calls.append((command, cwd))
        if command == ("pnpm", "run", "build"):
            assert cwd is not None
            (cwd / "dist").mkdir()
            (cwd / "dist" / "index.js").write_text(cwd.as_posix(), encoding="utf-8")

    monkeypatch.setattr(release_train, "run", fake_run)
    output_dir = tmp_path / "frontend-output"
    build_frontends(argparse.Namespace(descriptor=descriptor_path, out=output_dir))

    assert [command for command, _cwd in calls].count(("pnpm", "install", "--frozen-lockfile")) == 1
    assert [command for command, _cwd in calls].count(("pnpm", "run", "build")) == 2
    assert (output_dir / "one/ui/dist/index.js").is_file()
    assert (output_dir / "two/ui/dist/index.js").is_file()
