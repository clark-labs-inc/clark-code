[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$FilePath
)

$ErrorActionPreference = "Stop"
$resolvedFile = (Resolve-Path -LiteralPath $FilePath).Path
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$installation = & $vswhere -latest -property installationPath
$dumpbin = Get-ChildItem "$installation\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" |
  Sort-Object FullName -Descending |
  Select-Object -First 1
if (-not $dumpbin) {
  throw "dumpbin.exe was not found"
}

$dump = & $dumpbin.FullName /imports $resolvedFile
if ($LASTEXITCODE -ne 0) {
  throw "dumpbin could not inspect $resolvedFile"
}

$imports = [ordered]@{}
$currentDll = $null
$insideImports = $false
foreach ($line in $dump) {
  if ($line -match "Section contains the following imports:") {
    $insideImports = $true
    continue
  }
  if (-not $insideImports) {
    continue
  }
  if ($line -match "^\s+Summary\s*$") {
    break
  }
  if ($line -match "^\s{4}([A-Za-z0-9_.-]+\.dll)\s*$") {
    $currentDll = $Matches[1].ToLowerInvariant()
    $imports[$currentDll] = [System.Collections.Generic.List[string]]::new()
    continue
  }
  if (
    $null -ne $currentDll -and
    $line -match "^\s+[0-9A-Fa-f]+\s+([^\s=]+)\s*$"
  ) {
    $imports[$currentDll].Add($Matches[1])
  }
}
if ($imports.Count -eq 0) {
  throw "no PE imports were found in $resolvedFile"
}

$missing = [System.Collections.Generic.List[string]]::new()
foreach ($entry in $imports.GetEnumerator()) {
  $handle = [IntPtr]::Zero
  if (-not [System.Runtime.InteropServices.NativeLibrary]::TryLoad($entry.Key, [ref]$handle)) {
    $missing.Add("$($entry.Key) (module unavailable)")
    continue
  }
  try {
    foreach ($symbol in $entry.Value) {
      $address = [IntPtr]::Zero
      if (
        -not [System.Runtime.InteropServices.NativeLibrary]::TryGetExport(
          $handle,
          $symbol,
          [ref]$address
        )
      ) {
        $missing.Add("$($entry.Key)!$symbol")
      }
    }
  } finally {
    [System.Runtime.InteropServices.NativeLibrary]::Free($handle)
  }
}

$manifestCandidates = [System.Collections.Generic.List[string]]::new()
if ($missing -contains "comctl32.dll!TaskDialogIndirect") {
  $kitsRoot = (Get-ItemProperty `
      -Path "HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots" `
      -Name KitsRoot10).KitsRoot10
  $mt = Get-ChildItem "$kitsRoot\bin\*\x64\mt.exe" |
    Sort-Object FullName -Descending |
    Select-Object -First 1
  if ($mt) {
    $manifestPath = Join-Path `
      $env:RUNNER_TEMP `
      "clark-pe-manifest-$([Guid]::NewGuid().ToString('N')).xml"
    try {
      & $mt.FullName `
        -nologo `
        "-inputresource:$resolvedFile;#1" `
        "-out:$manifestPath" 2>$null
      if ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $manifestPath)) {
        $manifestCandidates.Add((Get-Content -LiteralPath $manifestPath -Raw))
      }
    } finally {
      if (Test-Path -LiteralPath $manifestPath) {
        Remove-Item -LiteralPath $manifestPath -Force
      }
    }
  }

  $externalManifestPath = "$resolvedFile.manifest"
  if (Test-Path -LiteralPath $externalManifestPath) {
    $manifestCandidates.Add((Get-Content -LiteralPath $externalManifestPath -Raw))
  }
}
$commonControlsV6 = $false
foreach ($manifest in $manifestCandidates) {
  if (
    $manifest -match 'name=["'']Microsoft\.Windows\.Common-Controls["'']' -and
    $manifest -match 'version=["'']6\.0\.0\.0["'']'
  ) {
    $commonControlsV6 = $true
    break
  }
}
if ($commonControlsV6) {
  [void]$missing.Remove("comctl32.dll!TaskDialogIndirect")
}

if ($missing.Count -gt 0) {
  throw "PE imports unavailable on this Windows runner:`n$($missing -join "`n")"
}
Write-Host "verified $($imports.Count) direct PE dependency modules: $resolvedFile"
