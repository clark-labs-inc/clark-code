param(
  [string]$Release = $env:CLARK_RELEASE
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($Release)) { $Release = "latest" }
$baseUrl = if ($env:CLARK_INSTALL_BASE_URL) { $env:CLARK_INSTALL_BASE_URL.TrimEnd("/") } else { "https://downloads.clarkchat.com/desktop/cli" }
$userRoot = [Environment]::GetFolderPath("UserProfile")
$clarkData = if ($env:CLARK_HOME) { $env:CLARK_HOME } else { Join-Path $userRoot ".clark" }
$installBin = if ($env:CLARK_INSTALL_DIR) { $env:CLARK_INSTALL_DIR } else { Join-Path $userRoot ".local\bin" }
$packageRoot = Join-Path $clarkData "packages\cli"
$releasesDir = Join-Path $packageRoot "releases"

if ($env:PROCESSOR_ARCHITECTURE -notin @("AMD64", "ARM64")) {
  throw "Clark CLI does not yet publish a Windows build for $env:PROCESSOR_ARCHITECTURE."
}
$target = "x86_64-pc-windows-msvc"
$asset = "clark-$target.zip"

$temporary = Join-Path ([IO.Path]::GetTempPath()) ("clark-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
  if ($Release -eq "latest") {
    $versionPath = Join-Path $temporary "VERSION"
    Invoke-WebRequest -UseBasicParsing -Uri "$baseUrl/latest/VERSION" -OutFile $versionPath
    $version = (Get-Content -Raw $versionPath).Trim()
  } else {
    $version = $Release.TrimStart("v")
  }
  if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "Invalid Clark release version: $version" }

  $releaseUrl = "$baseUrl/releases/v$version"
  $checksumsPath = Join-Path $temporary "SHA256SUMS"
  $archivePath = Join-Path $temporary $asset
  Write-Host "==> Downloading Clark $version for $target"
  Invoke-WebRequest -UseBasicParsing -Uri "$releaseUrl/SHA256SUMS" -OutFile $checksumsPath
  $line = Get-Content $checksumsPath | Where-Object { $_ -match "^([0-9a-fA-F]{64})\s+$([Regex]::Escape($asset))$" } | Select-Object -First 1
  if (-not $line) { throw "Clark release v$version has no checksum for $asset." }
  $expected = ([Regex]::Match($line, '^[0-9a-fA-F]{64}')).Value.ToLowerInvariant()
  Invoke-WebRequest -UseBasicParsing -Uri "$releaseUrl/$asset" -OutFile $archivePath
  $actual = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
  if ($actual -ne $expected) { throw "Clark archive checksum mismatch: got $actual, expected $expected" }

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [IO.Compression.ZipFile]::OpenRead($archivePath)
  try {
    foreach ($entry in $zip.Entries) {
      $entryPath = $entry.FullName.Replace('\', '/')
      if ($entryPath -notin @('bin/', 'bin/clark.exe', 'bin/clark-code-headless.exe')) {
        throw "Clark archive contains an unexpected path: $entryPath"
      }
    }
  } finally {
    $zip.Dispose()
  }

  $unpacked = Join-Path $temporary "unpacked"
  Expand-Archive -Path $archivePath -DestinationPath $unpacked
  $sourceBin = Join-Path $unpacked "bin"
  foreach ($name in @("clark.exe", "clark-code-headless.exe")) {
    if (-not (Test-Path -LiteralPath (Join-Path $sourceBin $name) -PathType Leaf)) {
      throw "Clark archive is missing bin/$name"
    }
  }
  & (Join-Path $sourceBin "clark.exe") --version | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "Clark CLI self-check failed" }
  & (Join-Path $sourceBin "clark-code-headless.exe") --self-test | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "Clark specialist worker self-check failed" }

  $releaseDir = Join-Path $releasesDir $version
  New-Item -ItemType Directory -Force -Path $releasesDir, $installBin | Out-Null
  if (-not (Test-Path -LiteralPath $releaseDir)) {
    Move-Item -LiteralPath $unpacked -Destination $releaseDir
  } else {
    foreach ($name in @("clark.exe", "clark-code-headless.exe")) {
      $verifiedPath = Join-Path $sourceBin $name
      $existingPath = Join-Path "$releaseDir\bin" $name
      if (-not (Test-Path -LiteralPath $existingPath -PathType Leaf)) {
        throw "Existing Clark release directory is incomplete: $releaseDir"
      }
      $verifiedHash = (Get-FileHash -Algorithm SHA256 $verifiedPath).Hash
      $existingHash = (Get-FileHash -Algorithm SHA256 $existingPath).Hash
      if ($verifiedHash -ne $existingHash) {
        throw "Existing Clark release directory differs from verified v${version}: $releaseDir"
      }
    }
  }
  foreach ($name in @("clark.exe", "clark-code-headless.exe")) {
    $destination = Join-Path $installBin $name
    $deadline = (Get-Date).AddSeconds(30)
    do {
      try {
        Copy-Item -LiteralPath (Join-Path "$releaseDir\bin" $name) -Destination $destination -Force
        break
      } catch {
        if ((Get-Date) -ge $deadline) { throw }
        Start-Sleep -Milliseconds 500
      }
    } while ($true)
  }

  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $pathParts = @($userPath -split ';' | Where-Object { $_ })
  if ($pathParts -notcontains $installBin) {
    [Environment]::SetEnvironmentVariable("Path", (($pathParts + $installBin) -join ';'), "User")
  }
  Write-Host "==> Installed Clark $version"
  Write-Host "Open a new terminal and run 'clark'. Over SSH, run 'clark login --device-code'."
} finally {
  Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
