import { Buffer } from "node:buffer";

import { assertClarkOwnedQaEmail } from "./clark-qa-auth.mjs";
import { nativeCredentialEnvelope } from "./native-credential-envelope.mjs";
import { executeGuestJson } from "./utm-guest-channel.mjs";

const DEFAULT_VM_NAME = "Clark QA - Ubuntu 24.04 Desktop";
const DEFAULT_FIXTURE = "/home/home/ClarkCodeQA";
const DEFAULT_MODEL = "clark-code:deepseek_v4_flash_latest";

function pythonString(value) {
  return JSON.stringify(String(value));
}

function validatedRetainedAuth(retainedAuth) {
  const user = retainedAuth?.descriptor?.user;
  const id = user?.id?.trim();
  const name = user?.name?.trim();
  const email = user?.email?.trim();
  const token = retainedAuth?.clarkToken?.trim();
  if (retainedAuth?.version !== 2 || !id || !name || !email || !token) {
    throw new Error("authenticated Ubuntu QA requires a complete v2 native auth record");
  }
  assertClarkOwnedQaEmail(email);
  if (token.split(".").length !== 3) {
    throw new Error("Clark token must be a JWT");
  }
  return retainedAuth;
}

export function buildUbuntuAuthenticatedWorkspaceProbe({
  retainedAuth,
  cwd = DEFAULT_FIXTURE,
  model = DEFAULT_MODEL,
}) {
  if (!cwd.startsWith("/home/home/")) {
    throw new Error("Ubuntu QA fixture must remain inside the home user profile");
  }
  if (!/^clark-code:[a-z0-9_]+$/.test(model)) {
    throw new Error("Ubuntu QA model must use a Clark Code model route");
  }
  const auth = validatedRetainedAuth(retainedAuth);
  const files = nativeCredentialEnvelope(auth);
  const expectedOwner = `id:${auth.descriptor.user.id}`;
  return `import base64, json, os, pathlib, pwd, signal, sqlite3, subprocess, time, urllib.parse
credential_key = base64.b64decode(${pythonString(files.key)})
credential_envelope = base64.b64decode(${pythonString(files.envelope)})
expected_owner = ${pythonString(expectedOwner)}
expected_email_domain = ${pythonString(auth.descriptor.user.email.split("@").at(-1).toLowerCase())}
encoded_owner = urllib.parse.quote(expected_owner, safe="")
settings_key = "clark-desktop:local-agent:" + encoded_owner
project_key = "clark-desktop:project-context:" + encoded_owner
fixture = pathlib.Path(${pythonString(cwd)})
model = ${pythonString(model)}
installed = pathlib.Path("/opt/Clark Code/clark-code")
home = pwd.getpwnam("home")
if not installed.is_file():
    raise RuntimeError("installed Ubuntu Clark Code product is unavailable")
if not fixture.is_dir():
    raise RuntimeError("autonomous Ubuntu QA fixture is unavailable")

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
time.sleep(2)

credential_root = pathlib.Path(
    "/home/home/.local/share/com.clark.desktop.dev/credentials"
)
credential_root.mkdir(parents=True, exist_ok=True, mode=0o700)
os.chmod(credential_root, 0o700)
for name, payload in [
    ("credentials.key", credential_key),
    ("credentials.enc", credential_envelope),
]:
    target = credential_root / name
    target.write_bytes(payload)
    os.chmod(target, 0o600)
    os.chown(target, home.pw_uid, home.pw_gid)
os.chown(credential_root, home.pw_uid, home.pw_gid)

storage_root = pathlib.Path(
    "/home/home/.local/share/com.clark.desktop.dev/localstorage"
)
storage_root.mkdir(parents=True, exist_ok=True)
database = storage_root / "tauri_localhost_0.localstorage"

def decode_value(value):
    return value.decode("utf-16-le") if isinstance(value, bytes) else str(value)

connection = sqlite3.connect(database)
connection.execute(
    "create table if not exists ItemTable "
    "(key TEXT unique on conflict replace, "
    "value BLOB not null on conflict fail)"
)
settings = {
    "cwd": "",
    "model": model,
    "reasoningEffort": "",
    "computerUseEnabled": False,
}
connection.execute("delete from ItemTable")
for key, value in [
    (settings_key, json.dumps(settings, separators=(",", ":"))),
    (project_key, json.dumps({"cwd": str(fixture)}, separators=(",", ":"))),
]:
    connection.execute(
        "insert or replace into ItemTable(key, value) values (?, ?)",
        (key, value.encode("utf-16-le")),
    )
connection.commit()
connection.execute("pragma wal_checkpoint(full)")
connection.close()
for item in [storage_root, *storage_root.iterdir()]:
    os.chown(item, home.pw_uid, home.pw_gid)

session_environment = None
for proc in pathlib.Path("/proc").iterdir():
    if not proc.name.isdigit():
        continue
    try:
        if proc.stat().st_uid != home.pw_uid:
            continue
        candidate = dict(
            item.split(b"=", 1)
            for item in (proc / "environ").read_bytes().split(b"\\0")
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
    raise RuntimeError("active Ubuntu graphical session environment is unavailable")
session_environment.update({
    "HOME": "/home/home",
    "USER": "home",
    "LOGNAME": "home",
    "PATH": "/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin",
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
for schema, key, value in [
    ("org.gnome.desktop.screensaver", "lock-enabled", "false"),
    ("org.gnome.desktop.screensaver", "ubuntu-lock-on-suspend", "false"),
    ("org.gnome.desktop.session", "idle-delay", "uint32 0"),
]:
    subprocess.run(
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

state_root = pathlib.Path("/home/home/.local/state/clark-code-qa")
state_root.mkdir(parents=True, exist_ok=True)
for item in [state_root, *state_root.iterdir()]:
    os.chown(item, home.pw_uid, home.pw_gid)
log_path = state_root / "ubuntu-auth-launch.log"
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

observed = {}
for attempt in range(120):
    time.sleep(1)
    if process.poll() is not None:
        break
    try:
        check = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
        rows = dict(check.execute(
            "select key, value from ItemTable where key in (?, ?)",
            (settings_key, project_key),
        ))
        check.close()
        observed_settings = json.loads(
            decode_value(rows[settings_key])
        )
        observed_project = json.loads(decode_value(rows[project_key]))
        observed = {
            "email_domain": expected_email_domain,
            "account_bound": all([
                (credential_root / "credentials.key").is_file(),
                (credential_root / "credentials.enc").is_file(),
            ]),
            "project_configured": (
                observed_project.get("cwd") == str(fixture)
            ),
            "model_configured": observed_settings.get("model") == model,
            "credential_fields_absent": not any(
                field in json.dumps(observed_settings)
                for field in ["apiKey", "apiKeyOwner", "clarkToken", "refreshToken", "accessToken"]
            ),
            "attempts": attempt + 1,
        }
        if all([
            observed["email_domain"] == "clarkslabs.com",
            observed["account_bound"],
            observed["project_configured"],
            observed["model_configured"],
            observed["credential_fields_absent"],
        ]):
            break
    except Exception:
        pass

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
workspace_ready = all([
    observed.get("email_domain") == "clarkslabs.com",
    observed.get("account_bound") is True,
    observed.get("project_configured") is True,
    observed.get("model_configured") is True,
    observed.get("credential_fields_absent") is True,
])
payload = {
    "status": "passed" if (
        process.poll() is None
        and "Clark Code Dev" in window.stdout
        and workspace_ready
        and unlock.returncode == 0
    ) else "failed",
    "process_running": process.poll() is None,
    "window_visible": "Clark Code Dev" in window.stdout,
    "session_unlock_exit_code": unlock.returncode,
    "workspace": observed,
    "credential_recorded": False,
    "credential_storage": "app_owned_encrypted_disk",
    "required_user_vm_actions": 0,
}`;
}

export function seedAndLaunchUbuntuAuthenticatedWorkspace({
  retainedAuth,
  run,
  vmName = DEFAULT_VM_NAME,
  cwd = DEFAULT_FIXTURE,
  model = DEFAULT_MODEL,
}) {
  const execution = executeGuestJson({
    platform: "ubuntu",
    vmName,
    state: "started",
    probeSource: buildUbuntuAuthenticatedWorkspaceProbe({
      retainedAuth,
      cwd,
      model,
    }),
    run,
    timeoutMs: 180_000,
    pollAttempts: 720,
    pollDelayMs: 250,
    executionAttempts: 1,
  });
  if (!execution.ok) {
    return {
      status: "failed",
      error: execution.error,
      attempts: execution.attempts,
      sensitive_transfer_erased: execution.cleanup_succeeded === true,
    };
  }
  const {
    probe_marker: _probeMarker,
    ...guest
  } = execution.data;
  const sensitiveTransferErased = execution.cleanup_succeeded === true;
  return {
    ...guest,
    status: (
      guest.status === "passed" && sensitiveTransferErased
    ) ? "passed" : "failed",
    sensitive_transfer_erased: sensitiveTransferErased,
  };
}
