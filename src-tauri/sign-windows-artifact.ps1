[CmdletBinding()]
param(
  [Parameter(Mandatory = $true, Position = 0)]
  [ValidateNotNullOrEmpty()]
  [string]$FilePath
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Require-FileEnvironmentVariable {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Name
  )

  $value = [Environment]::GetEnvironmentVariable($Name)
  if ([string]::IsNullOrWhiteSpace($value)) {
    throw "$Name is required for Windows release signing"
  }
  if (-not (Test-Path -LiteralPath $value -PathType Leaf)) {
    throw "$Name does not name a file: $value"
  }
  return (Resolve-Path -LiteralPath $value).Path
}

$target = (Resolve-Path -LiteralPath $FilePath).Path
if ([IO.Path]::GetExtension($target) -notin @(".exe", ".dll")) {
  throw "Artifact Signing is restricted to Windows PE files: $target"
}

$signTool = Require-FileEnvironmentVariable "CLARK_ARTIFACT_SIGNTOOL"
$dlib = Require-FileEnvironmentVariable "CLARK_ARTIFACT_SIGNING_DLIB"
$metadata = Require-FileEnvironmentVariable "CLARK_ARTIFACT_SIGNING_METADATA"

$metadataDocument = Get-Content -LiteralPath $metadata -Raw | ConvertFrom-Json
foreach ($field in @("Endpoint", "CodeSigningAccountName", "CertificateProfileName")) {
  if ([string]::IsNullOrWhiteSpace($metadataDocument.$field)) {
    throw "Artifact Signing metadata is missing $field"
  }
}

& $signTool sign `
  /v `
  /debug `
  /fd SHA256 `
  /tr "http://timestamp.acs.microsoft.com" `
  /td SHA256 `
  /d "Clark Code" `
  /dlib $dlib `
  /dmdf $metadata `
  $target
if ($LASTEXITCODE -ne 0) {
  throw "Artifact Signing failed for $target with exit code $LASTEXITCODE"
}

& $signTool verify /v /pa /all $target
if ($LASTEXITCODE -ne 0) {
  throw "Authenticode verification failed for $target with exit code $LASTEXITCODE"
}
