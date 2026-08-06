import { readFileSync, statSync } from "node:fs";

function unquote(value) {
  const trimmed = value.trim().replace(/\r$/, "");
  if (
    trimmed.length >= 2
    && ((trimmed.startsWith('"') && trimmed.endsWith('"'))
      || (trimmed.startsWith("'") && trimmed.endsWith("'")))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

function yamlString(value) {
  return JSON.stringify(String(value));
}

function powershellLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

export function readIgnoredEnv(envPath, allowedNames) {
  const values = {};
  const source = readFileSync(envPath, "utf8");
  for (const line of source.split(/\r?\n/)) {
    const match = line.match(/^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$/);
    if (!match || !allowedNames.includes(match[1])) continue;
    values[match[1]] = unquote(match[2]);
  }
  return values;
}

export function loadVmCredentials(envPath) {
  const names = ["CLARK_QA_VM_USERNAME", "CLARK_QA_VM_PASSWORD"];
  const fromFile = readIgnoredEnv(envPath, names);
  const username = process.env.CLARK_QA_VM_USERNAME || fromFile.CLARK_QA_VM_USERNAME;
  const password = process.env.CLARK_QA_VM_PASSWORD || fromFile.CLARK_QA_VM_PASSWORD;
  if (!username || !password) {
    throw new Error(`VM credentials are missing; define ${names.join(" and ")} in the ignored .env`);
  }
  if (!/^[a-z_][a-z0-9_-]{0,31}$/.test(username)) {
    throw new Error("CLARK_QA_VM_USERNAME is not a portable local-account name");
  }
  const metadata = statSync(envPath);
  if ((metadata.mode & 0o077) !== 0) {
    throw new Error("the ignored .env must not be readable or writable by group/other users");
  }
  return {
    username,
    password,
    source: ".env",
    source_mode: metadata.mode & 0o777,
  };
}

export function buildUbuntuAutoinstall({ username, passwordHash }) {
  if (!/^[a-z_][a-z0-9_-]{0,31}$/.test(username)) {
    throw new Error("Ubuntu username is invalid");
  }
  if (!/^\$(?:2[aby]|5|6|y)\$/.test(passwordHash)) {
    throw new Error("Ubuntu autoinstall requires a crypt-compatible password hash");
  }
  const gdm = `[daemon]\\nAutomaticLoginEnable=true\\nAutomaticLogin=${username}\\n`;
  const dconf = [
    "[org/gnome/desktop/session]",
    "idle-delay=uint32 0",
    "[org/gnome/desktop/screensaver]",
    "lock-enabled=false",
    "ubuntu-lock-on-suspend=false",
    "",
  ].join("\\n");
  return `#cloud-config
autoinstall:
  version: 1
  locale: en_US.UTF-8
  keyboard:
    layout: us
  source:
    id: ubuntu-desktop
    search_drivers: false
  identity:
    hostname: clark-qa-ubuntu
    username: ${yamlString(username)}
    password: ${yamlString(passwordHash)}
  storage:
    layout:
      name: direct
  ssh:
    install-server: false
  packages:
    - bubblewrap
    - curl
    - git
    - qemu-guest-agent
    - spice-vdagent
  updates: security
  timezone: America/Los_Angeles
  late-commands:
    - curtin in-target --target=/target -- systemctl enable qemu-guest-agent
    - curtin in-target --target=/target -- bash -c ${yamlString(`printf '${gdm}' > /etc/gdm3/custom.conf`)}
    - curtin in-target --target=/target -- bash -c ${yamlString(`install -d /etc/dconf/db/local.d && printf '${dconf}' > /etc/dconf/db/local.d/00-clark-qa && dconf update`)}
    - curtin in-target --target=/target -- bash -c ${yamlString(`install -d -o ${username} -g ${username} /home/${username}/.config && install -o ${username} -g ${username} /dev/null /home/${username}/.config/gnome-initial-setup-done`)}
    - curtin in-target --target=/target -- bash -c ${yamlString("printf 'schema_version=1\\nprovisioner=clark-code-utm-autonomy\\n' > /etc/clark-code-qa-provisioned")}
  shutdown: poweroff
`;
}

export function buildWindowsOneShotAutologon({ username, password }) {
  if (!/^[a-z_][a-z0-9_-]{0,31}$/.test(username)) {
    throw new Error("Windows username is invalid");
  }
  const user = powershellLiteral(username);
  const secret = powershellLiteral(password);
  const cleanup = [
    "$ErrorActionPreference = 'SilentlyContinue'",
    "Start-Sleep -Seconds 15",
    "$key = 'HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon'",
    "Remove-ItemProperty -Path $key -Name DefaultPassword -Force",
    "Set-ItemProperty -Path $key -Name AutoAdminLogon -Value '0'",
    "Remove-ItemProperty -Path $key -Name AutoLogonCount -Force",
    "Unregister-ScheduledTask -TaskName 'ClarkCodeQA-ClearAutologon' -Confirm:$false",
    "Remove-Item -LiteralPath $PSCommandPath -Force",
  ].join("\r\n");
  return [
    "$ErrorActionPreference = 'Stop'",
    "$key = 'HKLM:\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon'",
    `$username = ${user}`,
    `$password = ${secret}`,
    "Set-ItemProperty -Path $key -Name AutoAdminLogon -Value '1'",
    "Set-ItemProperty -Path $key -Name DefaultUserName -Value $username",
    "Set-ItemProperty -Path $key -Name DefaultPassword -Value $password",
    "Set-ItemProperty -Path $key -Name AutoLogonCount -Type DWord -Value 1",
    "$cleanupPath = 'C:\\ProgramData\\ClarkCodeQA\\clear-autologon.ps1'",
    "New-Item -ItemType Directory -Force -Path (Split-Path -Parent $cleanupPath) | Out-Null",
    `$cleanup = ${powershellLiteral(cleanup)}`,
    "Set-Content -LiteralPath $cleanupPath -Value $cleanup -Encoding UTF8",
    "$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"' + $cleanupPath + '\"')",
    "$trigger = New-ScheduledTaskTrigger -AtLogOn",
    "Register-ScheduledTask -TaskName 'ClarkCodeQA-ClearAutologon' -Action $action -Trigger $trigger -User 'SYSTEM' -RunLevel Highest -Force | Out-Null",
    "$password = $null",
    "",
  ].join("\r\n");
}
