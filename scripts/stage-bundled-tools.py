#!/usr/bin/env python3
"""Fetch, verify, and stage target-specific Tauri sidecars."""

import argparse
import hashlib
import json
import os
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
CACHE_DIR = Path(tempfile.gettempdir()) / "clark-code-bundled-tools"


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
    if artifact["format"] == "tar.gz":
        with tarfile.open(archive, "r:gz") as source:
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


def stage_tool(tool: dict, target: str, destination: Path) -> None:
    artifact = tool["targets"].get(target)
    if artifact is None:
        raise RuntimeError(f"{tool['command']} has no artifact for {target}")
    if artifact.get("strategy") == "macho-universal":
        with tempfile.TemporaryDirectory(prefix="clark-tool-lipo-") as temp:
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
    else:
        extract_member(artifact, destination)


def main() -> int:
    args = parse_args()
    manifest = load_manifest()
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    for tool in manifest["tools"].values():
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
        archive_root = Path(notice_artifact["member"]).parent
        for notice in tool.get("notices", []):
            notice_destination = OUTPUT_DIR / f"{tool['command']}-{notice}"
            notice_destination.unlink(missing_ok=True)
            extract_member(
                notice_artifact,
                notice_destination,
                str(archive_root / notice),
            )
        print(
            f"staged {tool['command']} {tool['version']} for {args.target}: {destination}",
            flush=True,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
