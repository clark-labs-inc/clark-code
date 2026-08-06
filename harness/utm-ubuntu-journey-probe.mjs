export function ubuntuBuildInstallLaunchProbe() {
  return String.raw`
import hashlib, json, os, pathlib, pwd, shutil, signal, subprocess, time

source_root = pathlib.Path(
    pathlib.Path("/opt/clark-qa/source-current.txt").read_text().strip()
)
source_sha256 = (source_root / ".source-sha256").read_text().strip()
target_root = pathlib.Path("/opt/clark-qa/cargo-target") / source_sha256[:12]
run_root = pathlib.Path("/opt/clark-qa/runs") / (
    "ubuntu-product-" + source_sha256[:12]
)
home = pwd.getpwnam("home")
for directory in [target_root, run_root]:
    directory.mkdir(parents=True, exist_ok=True)
    os.chown(directory, home.pw_uid, home.pw_gid)

# The staged archive is extracted by root and remains the pinned source of
# truth. The frontend build legitimately creates node_modules, dist, and
# TypeScript metadata, so only that subtree is delegated to the unprivileged
# build user. Rust output stays isolated under target_root.
frontend_root = source_root / "app"
for directory, child_directories, files in os.walk(frontend_root):
    os.chown(directory, home.pw_uid, home.pw_gid)
    for name in [*child_directories, *files]:
        os.chown(pathlib.Path(directory) / name, home.pw_uid, home.pw_gid)

# tauri-build writes generated permission schemas below this ignored path.
# Precreate only the generated-output root; the Rust host source stays
# root-owned and read-only to the unprivileged build.
tauri_generated_root = source_root / "src-tauri/gen"
tauri_generated_root.mkdir(parents=True, exist_ok=True)
os.chown(tauri_generated_root, home.pw_uid, home.pw_gid)

guest_env = {
    "HOME": "/home/home",
    "USER": "home",
    "LOGNAME": "home",
    "CI": "1",
    "RUSTUP_HOME": "/home/home/.rustup",
    "CARGO_HOME": "/home/home/.cargo",
    "CARGO_TARGET_DIR": str(target_root),
    "XDG_CACHE_HOME": "/home/home/.cache",
    "PATH": (
        "/home/home/.cargo/bin:"
        "/opt/clark-qa/node-v24.14.0-linux-arm64/bin:"
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    ),
}

def run_step(step_id, args, timeout_seconds):
    log_path = run_root / (step_id + ".log")
    started = time.monotonic()
    timed_out = False
    with log_path.open("w", encoding="utf-8", errors="replace") as log:
        process = subprocess.Popen(
            ["runuser", "-u", "home", "--", "env"]
            + [name + "=" + value for name, value in guest_env.items()]
            + args,
            cwd=source_root,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            exit_code = process.wait(timeout=timeout_seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGKILL)
            exit_code = process.wait()
    output = log_path.read_text(encoding="utf-8", errors="replace")
    return {
        "id": step_id,
        "status": "passed" if exit_code == 0 and not timed_out else "failed",
        "exit_code": exit_code,
        "timed_out": timed_out,
        "duration_ms": round((time.monotonic() - started) * 1000),
        "output_tail": output[-2000:],
    }

steps = [
    run_step(
        "frontend_install",
        [
            "corepack", "pnpm@10", "--dir", "app", "install",
            "--frozen-lockfile",
        ],
        1800,
    )
]
if steps[-1]["status"] == "passed":
    steps.append(run_step(
        "frontend_build",
        ["corepack", "pnpm@10", "--dir", "app", "build"],
        1800,
    ))
if steps[-1]["status"] == "passed":
    steps.append(run_step(
        "native_arm_build",
        [
            "cargo", "build", "--locked", "-p", "clark-desktop",
            "--features", "tauri/custom-protocol",
        ],
        3600,
    ))

source_binary = target_root / "debug/clark-desktop"
if (
    len(steps) != 3
    or any(step["status"] != "passed" for step in steps)
    or not source_binary.is_file()
):
    payload = {
        "status": "failed",
        "phase": "build",
        "source_sha256": source_sha256,
        "steps": steps,
        "required_user_vm_actions": 0,
    }
else:
    dependency_steps = []
    if shutil.which("rg") is None:
        update = subprocess.run(
            ["apt-get", "update", "-qq"],
            text=True,
            capture_output=True,
            timeout=900,
        )
        dependency_steps.append({
            "id": "apt_update",
            "exit_code": update.returncode,
            "output_tail": (update.stdout + update.stderr)[-1000:],
        })
        if update.returncode == 0:
            install = subprocess.run(
                ["apt-get", "install", "-y", "-qq", "ripgrep"],
                text=True,
                capture_output=True,
                timeout=900,
            )
            dependency_steps.append({
                "id": "ripgrep_install",
                "exit_code": install.returncode,
                "output_tail": (install.stdout + install.stderr)[-1000:],
            })

    install_root = pathlib.Path("/opt/Clark Code")
    install_root.mkdir(parents=True, exist_ok=True)
    installed = install_root / "clark-code"
    temporary = install_root / ".clark-code-installing"
    shutil.copy2(source_binary, temporary)
    temporary.chmod(0o755)
    os.replace(temporary, installed)

    link = pathlib.Path("/usr/local/bin/clark-code")
    temporary_link = pathlib.Path("/usr/local/bin/.clark-code-link")
    temporary_link.unlink(missing_ok=True)
    temporary_link.symlink_to(installed)
    os.replace(temporary_link, link)

    icon_source = source_root / "src-tauri/icons/128x128.png"
    icon_target = pathlib.Path(
        "/usr/local/share/icons/hicolor/128x128/apps/clark-code.png"
    )
    icon_target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(icon_source, icon_target)
    desktop_file = pathlib.Path(
        "/usr/local/share/applications/com.clark.desktop.dev.desktop"
    )
    desktop_file.parent.mkdir(parents=True, exist_ok=True)
    desktop_file.write_text(
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=Clark Code Dev\n"
        "Exec=/usr/local/bin/clark-code\n"
        "Icon=clark-code\n"
        "Terminal=false\n"
        "Categories=Development;Utility;\n",
        encoding="utf-8",
    )
    desktop_file.chmod(0o644)
    if shutil.which("update-desktop-database"):
        subprocess.run(
            ["update-desktop-database", "/usr/local/share/applications"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=30,
        )

    fixture = pathlib.Path("/home/home/ClarkCodeQA")
    fixture.mkdir(parents=True, exist_ok=True)
    (fixture / "README.md").write_text(
        "# Clark Code autonomous Ubuntu fixture\n\n"
        "This repository belongs only to the QA harness.\n",
        encoding="utf-8",
    )
    (fixture / "numbers.txt").write_text(
        "2\n3\n5\n7\n",
        encoding="utf-8",
    )
    for item in [fixture, *fixture.iterdir()]:
        os.chown(item, home.pw_uid, home.pw_gid)

    for proc in pathlib.Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            if proc.stat().st_uid != home.pw_uid:
                continue
            executable = os.readlink(proc / "exe").removesuffix(" (deleted)")
            command = (proc / "comm").read_text().strip()
            if executable == str(installed) or command == "clark-code":
                os.kill(int(proc.name), signal.SIGTERM)
        except Exception:
            pass
    time.sleep(1)

    session_environment = None
    for proc in pathlib.Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            if proc.stat().st_uid != home.pw_uid:
                continue
            raw = (proc / "environ").read_bytes()
            candidate = dict(
                item.split(b"=", 1)
                for item in raw.split(b"\0")
                if b"=" in item
            )
            if b"DISPLAY" in candidate and b"XAUTHORITY" in candidate:
                session_environment = {
                    key.decode(): value.decode(errors="replace")
                    for key, value in candidate.items()
                }
                break
        except Exception:
            pass
    if session_environment is None:
        raise RuntimeError(
            "active Ubuntu graphical session environment is unavailable"
        )
    session_environment.update({
        "HOME": "/home/home",
        "USER": "home",
        "LOGNAME": "home",
        "PATH": (
            "/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin"
        ),
        "GDK_BACKEND": "x11",
        "WEBKIT_DISABLE_COMPOSITING_MODE": "1",
        "WEBKIT_DISABLE_DMABUF_RENDERER": "1",
    })

    session_id = None
    sessions = subprocess.run(
        ["loginctl", "list-sessions", "--no-legend"],
        text=True,
        capture_output=True,
        timeout=10,
    ).stdout.splitlines()
    for row in sessions:
        fields = row.split()
        if len(fields) >= 3 and fields[2] == "home":
            session_id = fields[0]
            break
    if session_id is None:
        raise RuntimeError("active Ubuntu home login session is unavailable")
    unlock = subprocess.run(
        ["loginctl", "unlock-session", session_id],
        text=True,
        capture_output=True,
        timeout=10,
    )
    settings = []
    for schema, key, value in [
        ("org.gnome.desktop.screensaver", "lock-enabled", "false"),
        (
            "org.gnome.desktop.screensaver",
            "ubuntu-lock-on-suspend",
            "false",
        ),
        ("org.gnome.desktop.session", "idle-delay", "uint32 0"),
    ]:
        setting = subprocess.run(
            [
                "runuser", "-u", "home", "--", "env",
                "DBUS_SESSION_BUS_ADDRESS="
                + session_environment["DBUS_SESSION_BUS_ADDRESS"],
                "gsettings", "set", schema, key, value,
            ],
            text=True,
            capture_output=True,
            timeout=10,
        )
        settings.append({
            "schema": schema,
            "key": key,
            "exit_code": setting.returncode,
        })

    state_root = pathlib.Path("/home/home/.local/state/clark-code-qa")
    state_root.mkdir(parents=True, exist_ok=True)
    os.chown(state_root, home.pw_uid, home.pw_gid)
    log_path = state_root / "ubuntu-launch.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen(
            [str(installed)],
            cwd=fixture,
            env=session_environment,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            user=home.pw_uid,
            group=home.pw_gid,
            extra_groups=[],
        )
    os.chown(log_path, home.pw_uid, home.pw_gid)

    process_running = False
    window_visible = False
    for _ in range(60):
        process_running = process.poll() is None
        window = subprocess.run(
            [
                "runuser", "-u", "home", "--", "env",
                "DISPLAY=" + session_environment["DISPLAY"],
                "XAUTHORITY=" + session_environment["XAUTHORITY"],
                "xwininfo", "-root", "-tree",
            ],
            text=True,
            capture_output=True,
            timeout=10,
        )
        window_visible = "Clark Code Dev" in window.stdout
        if process_running and window_visible:
            break
        time.sleep(1)

    binary_sha256 = hashlib.sha256(installed.read_bytes()).hexdigest()
    install_receipt = {
        "source_sha256": source_sha256,
        "binary_sha256": binary_sha256,
        "architecture": subprocess.run(
            ["uname", "-m"],
            text=True,
            capture_output=True,
        ).stdout.strip(),
    }
    (install_root / "install-receipt.json").write_text(
        json.dumps(install_receipt, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    log_tail = log_path.read_text(errors="replace")[-2000:]
    dependencies_ready = (
        shutil.which("rg") is not None
        and shutil.which("bwrap") is not None
    )
    passed = (
        process_running
        and window_visible
        and dependencies_ready
        and link.resolve() == installed
        and unlock.returncode == 0
        and all(item["exit_code"] == 0 for item in settings)
    )
    payload = {
        "status": "passed" if passed else "failed",
        "phase": "product_launch",
        "source_sha256": source_sha256,
        "binary_sha256": binary_sha256,
        "installed_path": str(installed),
        "launcher_path": str(link),
        "desktop_file": str(desktop_file),
        "installation_kind": "atomic_native_arm_debug",
        "process_id": process.pid,
        "process_running": process_running,
        "window_visible": window_visible,
        "window_title": "Clark Code Dev" if window_visible else None,
        "session_id": session_id,
        "session_unlock_exit_code": unlock.returncode,
        "settings": settings,
        "architecture": install_receipt["architecture"],
        "ripgrep_path": shutil.which("rg"),
        "bubblewrap_path": shutil.which("bwrap"),
        "dependency_steps": dependency_steps,
        "steps": steps,
        "launch_log_tail": log_tail,
        "required_user_vm_actions": 0,
    }
`;
}
