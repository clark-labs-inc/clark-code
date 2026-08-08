#!/usr/bin/env python3
"""Fetch, verify, and stage target-specific Tauri sidecars."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import stat
import subprocess
import tarfile
import tempfile
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path
from typing import Optional


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "scripts" / "bundled-tools.json"
OUTPUT_DIR = ROOT / "src-tauri" / "binaries"
CACHE_DIR = Path(tempfile.gettempdir()) / "agent-desktop-bundled-tools"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, help="Tauri/Rust target triple")
    parser.add_argument("--force", action="store_true", help="replace staged files")
    return parser.parse_args()


def load_manifest() -> dict:
    with MANIFEST.open(encoding="utf-8") as source:
        manifest = json.load(source)
    if manifest.get("schema_version") != 1:
        raise RuntimeError(f"unsupported manifest schema: {manifest.get('schema_version')}")
    return manifest


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def verified_archive(artifact: dict) -> Path:
    filename = Path(urllib.parse.urlparse(artifact["url"]).path).name
    archive = CACHE_DIR / artifact["sha256"] / filename
    valid = (
        archive.is_file()
        and archive.stat().st_size == artifact["size"]
        and digest(archive) == artifact["sha256"]
    )
    if valid:
        return archive

    archive.parent.mkdir(parents=True, exist_ok=True)
    temporary = archive.with_suffix(archive.suffix + ".tmp")
    temporary.unlink(missing_ok=True)
    print(f"fetching {artifact['url']}", flush=True)
    try:
        with urllib.request.urlopen(artifact["url"], timeout=60) as response:
            with temporary.open("wb") as output:
                shutil.copyfileobj(response, output)
        if temporary.stat().st_size != artifact["size"]:
            raise RuntimeError(
                f"archive size mismatch: got {temporary.stat().st_size}, expected {artifact['size']}"
            )
        actual = digest(temporary)
        if actual != artifact["sha256"]:
            raise RuntimeError(
                f"archive digest mismatch: got {actual}, expected {artifact['sha256']}"
            )
        temporary.replace(archive)
    finally:
        temporary.unlink(missing_ok=True)
    return archive


def extract_member(
    artifact: dict, destination: Path, member_name: Optional[str] = None
) -> None:
    archive = verified_archive(artifact)
    member_name = member_name or artifact["member"]
    destination.parent.mkdir(parents=True, exist_ok=True)
    if artifact["format"] in {"tar.gz", "tar.xz"}:
        mode = "r:gz" if artifact["format"] == "tar.gz" else "r:xz"
        with tarfile.open(archive, mode) as source:
            member = source.extractfile(member_name)
            if member is None:
                raise RuntimeError(f"missing archive member {member_name}")
            with destination.open("wb") as output:
                shutil.copyfileobj(member, output)
    elif artifact["format"] == "zip":
        with zipfile.ZipFile(archive) as source:
            with source.open(member_name) as member:
                with destination.open("wb") as output:
                    shutil.copyfileobj(member, output)
    else:
        raise RuntimeError(f"unsupported archive format {artifact['format']}")
    if os.name != "nt":
        destination.chmod(destination.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def extract_source_tree(artifact: dict, destination: Path) -> Path:
    archive = verified_archive(artifact)
    source_dir = artifact["source_dir"]
    with tarfile.open(archive, "r:xz") as source:
        for member in source.getmembers():
            member_path = Path(member.name)
            if member_path.is_absolute() or ".." in member_path.parts:
                raise RuntimeError(f"unsafe source archive member {member.name}")
            if member.issym() or member.islnk():
                link_path = Path(member.linkname)
                if link_path.is_absolute() or ".." in link_path.parts:
                    raise RuntimeError(
                        f"unsafe source archive link {member.name} -> {member.linkname}"
                    )
        source.extractall(destination)
    extracted = destination / source_dir
    if not extracted.is_dir():
        raise RuntimeError(f"missing source directory {source_dir}")
    return extracted


def build_meson_artifact(artifact: dict, target: str, destination: Path) -> None:
    required_commands = ("meson", "ninja", "pkg-config")
    missing_commands = [
        command for command in required_commands if shutil.which(command) is None
    ]
    if missing_commands:
        raise RuntimeError(
            "source build prerequisites are missing: " + ", ".join(missing_commands)
        )
    expected_arch = target.split("-", 1)[0]
    host_arch = platform.machine().lower()
    aliases = {"amd64": "x86_64", "arm64": "aarch64"}
    host_arch = aliases.get(host_arch, host_arch)
    if host_arch != expected_arch:
        raise RuntimeError(
            f"native source build for {target} requires {expected_arch}, host is {host_arch}"
        )
    with tempfile.TemporaryDirectory(prefix="agent-tool-build-") as temp:
        temp_dir = Path(temp)
        source = extract_source_tree(artifact, temp_dir)
        build = temp_dir / "build"
        subprocess.run(
            ["meson", "setup", str(build), str(source), *artifact.get("meson_args", [])],
            check=True,
        )
        subprocess.run(
            ["meson", "compile", "-C", str(build), artifact["build_target"]],
            check=True,
        )
        built = build / artifact["output"]
        if not built.is_file():
            raise RuntimeError(f"Meson build did not produce {built}")
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(built, destination)
        destination.chmod(destination.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def stage_tool(tool: dict, target: str, destination: Path) -> None:
    artifact = tool["targets"].get(target)
    if artifact is None:
        raise RuntimeError(f"{tool['command']} has no artifact for {target}")
    strategy = artifact.get("strategy")
    if strategy == "macho-universal":
        with tempfile.TemporaryDirectory(prefix="agent-tool-lipo-") as temp:
            inputs = []
            for source_target in artifact["merge"]:
                source = Path(temp) / source_target
                extract_member(tool["targets"][source_target], source)
                inputs.append(str(source))
            subprocess.run(
                ["lipo", "-create", *inputs, "-output", str(destination)],
                check=True,
            )
            destination.chmod(destination.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    elif strategy == "meson-source":
        build_meson_artifact(artifact, target, destination)
    else:
        extract_member(artifact, destination)


def target_platform(target: str) -> str:
    if "windows" in target:
        return "windows"
    if "linux" in target:
        return "linux"
    if "darwin" in target:
        return "macos"
    raise RuntimeError(f"unsupported target platform: {target}")


def main() -> int:
    args = parse_args()
    manifest = load_manifest()
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    for tool in manifest["tools"].values():
        platforms = tool.get("platforms")
        if platforms is not None and target_platform(args.target) not in platforms:
            continue
        suffix = ".exe" if "windows" in args.target else ""
        destination = OUTPUT_DIR / f"{tool['command']}-{args.target}{suffix}"
        if destination.exists():
            if not args.force:
                raise RuntimeError(f"staged tool already exists: {destination}; pass --force")
            destination.unlink()
        stage_tool(tool, args.target, destination)
        notice_artifact = tool["targets"][args.target]
        if "merge" in notice_artifact:
            notice_artifact = tool["targets"][notice_artifact["merge"][0]]
        # Archive member names always use POSIX separators, even when this
        # staging script runs on Windows.
        archive_root = notice_artifact.get("source_dir")
        if archive_root is None:
            archive_root = notice_artifact["member"].rsplit("/", 1)[0]
        for notice in tool.get("notices", []):
            notice_destination = OUTPUT_DIR / f"{tool['command']}-{notice}"
            notice_destination.unlink(missing_ok=True)
            extract_member(
                notice_artifact,
                notice_destination,
                f"{archive_root}/{notice}",
            )
        if tool.get("bundle_source_archive"):
            if "merge" in tool["targets"][args.target]:
                raise RuntimeError(
                    f"{tool['command']} cannot bundle a merged artifact as source"
                )
            source_archive = verified_archive(tool["targets"][args.target])
            source_format = tool["targets"][args.target]["format"]
            source_suffix = {
                "tar.gz": ".tar.gz",
                "tar.xz": ".tar.xz",
                "zip": ".zip",
            }.get(source_format)
            if source_suffix is None:
                raise RuntimeError(f"unsupported bundled source format {source_format}")
            source_destination = OUTPUT_DIR / (
                f"{tool['command']}-{tool['version']}-source{source_suffix}"
            )
            source_destination.unlink(missing_ok=True)
            shutil.copy2(source_archive, source_destination)
        print(
            f"staged {tool['command']} {tool['version']} for {args.target}: {destination}",
            flush=True,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
