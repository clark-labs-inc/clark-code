#!/usr/bin/env python3
"""Verify Clark's public-tool/private-resource package boundary."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tarfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "scripts" / "bundled-tools.json"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("macos", "linux", "windows"), required=True)
    parser.add_argument("--resource-root", type=Path, required=True)
    parser.add_argument("--executable-dir", type=Path)
    parser.add_argument(
        "--staged-bwrap",
        type=Path,
        help="source-built bwrap whose digest must match the packaged Linux helper",
    )
    return parser.parse_args()


def require_file(path: Path, *, executable: bool = False) -> None:
    if not path.is_file():
        raise RuntimeError(f"required package file is missing: {path}")
    if executable and os.name != "nt" and not os.access(path, os.X_OK):
        raise RuntimeError(f"packaged executable has no execute bit: {path}")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def assert_private_names_absent(directory: Path | None, names: list[str]) -> None:
    if directory is None or not directory.exists():
        return
    for name in names:
        candidate = directory / name
        if candidate.exists():
            raise RuntimeError(f"private helper leaked into a public command directory: {candidate}")


def assert_windows_posix_toolchain_absent(*roots: Path | None) -> None:
    forbidden_files = {
        "bash.exe",
        "sh.exe",
        "mintty.exe",
        "msys-2.0.dll",
        "gcc.exe",
        "g++.exe",
    }
    forbidden_components = {"mingw", "mingw64", "msys", "msys2", "git-bash"}
    for root in roots:
        if root is None or not root.exists():
            continue
        for candidate in root.rglob("*"):
            components = {part.lower() for part in candidate.relative_to(root).parts}
            if (
                candidate.name.lower() in forbidden_files
                or components.intersection(forbidden_components)
            ):
                raise RuntimeError(
                    "Windows package unexpectedly bundles a POSIX shell/toolchain: "
                    f"{candidate}"
                )


def verify(args: argparse.Namespace) -> dict:
    resource_root = args.resource_root.resolve()
    executable_dir = args.executable_dir.resolve() if args.executable_dir else None
    public_dir = resource_root / "clark-path"
    private_root = resource_root / "clark-resources"
    private_names = [
        "bwrap",
        "clark-computer-use-helper",
        "clark-computer-use-helper.exe",
        "clark-command-runner.exe",
        "clark-windows-sandbox-setup.exe",
    ]

    if args.platform == "macos":
        if executable_dir is None:
            raise RuntimeError("macOS verification requires --executable-dir")
        # Tauri keeps rg PATH-visible in Contents/MacOS. Computer Use is a
        # separately identified nested app so its macOS privacy grants remain
        # bound to a durable service identity across Clark host updates.
        require_file(executable_dir / "rg", executable=True)
        require_file(executable_dir / "clark-code-headless", executable=True)
        service = resource_root / "Clark Computer Use.app"
        require_file(
            service / "Contents" / "MacOS" / "clark-computer-use-helper",
            executable=True,
        )
        require_file(
            service / "Contents" / "Frameworks" / "libswift_Concurrency.dylib"
        )
        require_file(service / "Contents" / "Info.plist")
        require_file(private_root / "licenses/ripgrep/LICENSE-MIT")
        require_file(private_root / "licenses/ripgrep/UNLICENSE")
        assert_private_names_absent(executable_dir, private_names)
    elif args.platform == "linux":
        if executable_dir is None:
            raise RuntimeError("Linux verification requires --executable-dir")
        require_file(executable_dir / "clark-code-headless", executable=True)
        require_file(public_dir / "rg", executable=True)
        computer_use_service = (
            private_root / "computer-use/clark-computer-use-helper"
        )
        require_file(computer_use_service, executable=True)
        packaged_bwrap = private_root / "sandbox/linux/bwrap"
        require_file(packaged_bwrap, executable=True)
        if args.staged_bwrap is None:
            raise RuntimeError("Linux verification requires --staged-bwrap")
        require_file(args.staged_bwrap, executable=True)
        staged_bwrap_digest = sha256(args.staged_bwrap)
        packaged_bwrap_digest = sha256(packaged_bwrap)
        if packaged_bwrap_digest != staged_bwrap_digest:
            raise RuntimeError(
                "packaged bubblewrap digest mismatch: "
                f"got {packaged_bwrap_digest}, expected {staged_bwrap_digest}"
            )
        require_file(private_root / "licenses/ripgrep/LICENSE-MIT")
        require_file(private_root / "licenses/ripgrep/UNLICENSE")
        require_file(private_root / "licenses/bubblewrap/COPYING")
        require_file(private_root / "licenses/bubblewrap/LICENSE")
        source = private_root / "licenses/bubblewrap/bubblewrap-0.11.2.tar.xz"
        require_file(source)
        manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
        expected = manifest["tools"]["bubblewrap"]["targets"][
            "x86_64-unknown-linux-gnu"
        ]["sha256"]
        actual = sha256(source)
        if actual != expected:
            raise RuntimeError(
                f"bundled bubblewrap source digest mismatch: got {actual}, expected {expected}"
            )
        with tarfile.open(source, "r:xz") as archive:
            names = set(archive.getnames())
        for member in (
            "bubblewrap-0.11.2/COPYING",
            "bubblewrap-0.11.2/LICENSE",
        ):
            if member not in names:
                raise RuntimeError(f"bubblewrap source archive is missing {member}")
        assert_private_names_absent(public_dir, private_names)
        assert_private_names_absent(executable_dir, private_names)
    else:
        if executable_dir is None:
            raise RuntimeError("Windows verification requires --executable-dir")
        require_file(executable_dir / "clark-code-headless.exe")
        require_file(public_dir / "rg.exe")
        require_file(
            private_root / "computer-use/clark-computer-use-helper.exe"
        )
        require_file(private_root / "sandbox/windows/clark-command-runner.exe")
        require_file(private_root / "sandbox/windows/clark-windows-sandbox-setup.exe")
        require_file(private_root / "licenses/ripgrep/LICENSE-MIT")
        require_file(private_root / "licenses/ripgrep/UNLICENSE")
        assert_private_names_absent(public_dir, private_names)
        assert_private_names_absent(executable_dir, private_names)
        assert_windows_posix_toolchain_absent(resource_root, executable_dir)

    receipt = {
        "platform": args.platform,
        "resource_root": str(resource_root),
        "executable_dir": str(executable_dir) if executable_dir else None,
        "path_boundary": "verified",
        "private_boundary": "verified",
    }
    if args.platform == "linux":
        receipt["bwrap_sha256"] = sha256(
            resource_root / "clark-resources/sandbox/linux/bwrap"
        )
        receipt["computer_use_service_sha256"] = sha256(
            resource_root
            / "clark-resources/computer-use/clark-computer-use-helper"
        )
    elif args.platform == "windows":
        receipt["computer_use_service_sha256"] = sha256(
            resource_root
            / "clark-resources/computer-use/clark-computer-use-helper.exe"
        )
        receipt["shell_runtime"] = "native_windows_only"
    if executable_dir is not None:
        worker = executable_dir / (
            "clark-code-headless.exe"
            if args.platform == "windows"
            else "clark-code-headless"
        )
        receipt["scientist_worker_sha256"] = sha256(worker)
    return receipt


def main() -> int:
    print(json.dumps(verify(parse_args()), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
