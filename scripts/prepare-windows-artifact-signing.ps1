[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$Endpoint,

  [Parameter(Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$AccountName,

  [Parameter(Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$ResourceGroup,

  [Parameter(Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$CertificateProfileName,

  [Parameter(Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$ExpectedSignerSubject,

  [Parameter(Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$CorrelationId
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$ProgressPreference = "SilentlyContinue"

if ($Endpoint -notmatch '^https://[a-z0-9]+\.codesigning\.azure\.net/?$') {
  throw "Artifact Signing endpoint is not a region endpoint: $Endpoint"
}

$account = az account show --output json | ConvertFrom-Json
if ([string]::IsNullOrWhiteSpace($account.id)) {
  throw "Azure OIDC login did not select a subscription"
}

$profileResourceId = (
  "/subscriptions/{0}/resourceGroups/{1}/providers/Microsoft.CodeSigning/" +
  "codeSigningAccounts/{2}/certificateProfiles/{3}"
) -f $account.id, $ResourceGroup, $AccountName, $CertificateProfileName
az resource show --ids $profileResourceId --only-show-errors --output none
if ($LASTEXITCODE -ne 0) {
  throw "Artifact Signing certificate profile is unavailable: $profileResourceId"
}

$installer = Join-Path $env:RUNNER_TEMP "ArtifactSigningClientTools.msi"
try {
  Invoke-WebRequest `
    -Uri "https://download.microsoft.com/download/70ad2c3b-761f-4aa9-a9de-e7405aa2b4c1/ArtifactSigningClientTools.msi" `
    -OutFile $installer
  $installerSignature = Get-AuthenticodeSignature -LiteralPath $installer
  if (
    $installerSignature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
    $installerSignature.SignerCertificate.Subject -notmatch 'Microsoft'
  ) {
    throw "Artifact Signing Client Tools installer lacks a valid Microsoft signature"
  }
  $install = Start-Process `
    -FilePath msiexec.exe `
    -ArgumentList @("/i", $installer, "/quiet", "/norestart") `
    -Wait `
    -PassThru
  if ($install.ExitCode -notin @(0, 3010)) {
    throw "Artifact Signing Client Tools installation failed with exit code $($install.ExitCode)"
  }
} finally {
  Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
}

$signTools = Get-ChildItem `
  "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" `
  -ErrorAction SilentlyContinue |
  Where-Object {
    try {
      [version]$_.VersionInfo.ProductVersion -ge [version]"10.0.22621.755"
    } catch {
      $false
    }
  } |
  Sort-Object { [version]$_.VersionInfo.ProductVersion } -Descending
$signTool = $signTools |
  Select-Object -First 1
if (-not $signTool) {
  throw "x64 SignTool 10.0.22621.755 or newer was not found"
}

$dlib = Get-ChildItem `
  "${env:ProgramFiles(x86)}\Microsoft\ArtifactSigningClientTools\bin\Azure.CodeSigning.Dlib.dll", `
  "${env:ProgramFiles(x86)}\Microsoft\ArtifactSigningClientTools\bin\x64\Azure.CodeSigning.Dlib.dll", `
  "${env:ProgramFiles(x86)}\Microsoft\Artifact Signing Client Tools\*\x64\Azure.CodeSigning.Dlib.dll", `
  "${env:ProgramFiles(x86)}\Microsoft\Artifact Signing Client Tools\x64\Azure.CodeSigning.Dlib.dll", `
  "${env:ProgramFiles(x86)}\Windows Kits\AzureCodeSigning\bin\x64\Azure.CodeSigning.Dlib.dll", `
  "${env:ProgramFiles}\Microsoft\ArtifactSigningClientTools\bin\Azure.CodeSigning.Dlib.dll", `
  "${env:ProgramFiles}\Microsoft\ArtifactSigningClientTools\bin\x64\Azure.CodeSigning.Dlib.dll", `
  "${env:ProgramFiles}\Microsoft\Artifact Signing Client Tools\*\x64\Azure.CodeSigning.Dlib.dll", `
  "${env:ProgramFiles}\Microsoft\Artifact Signing Client Tools\x64\Azure.CodeSigning.Dlib.dll" `
  -ErrorAction SilentlyContinue |
  Sort-Object FullName -Descending |
  Select-Object -First 1
if (-not $dlib) {
  throw "Azure.CodeSigning.Dlib.dll was not installed"
}

$metadataDirectory = Join-Path $env:GITHUB_WORKSPACE "target\artifact-signing"
New-Item -ItemType Directory -Force -Path $metadataDirectory | Out-Null
$metadataPath = Join-Path $metadataDirectory "metadata.json"
@{
  Endpoint = $Endpoint.TrimEnd("/")
  CodeSigningAccountName = $AccountName
  CertificateProfileName = $CertificateProfileName
  CorrelationId = $CorrelationId
  ExcludeCredentials = @(
    "EnvironmentCredential"
    "WorkloadIdentityCredential"
    "ManagedIdentityCredential"
    "SharedTokenCacheCredential"
    "VisualStudioCredential"
    "VisualStudioCodeCredential"
    "AzurePowerShellCredential"
    "AzureDeveloperCliCredential"
    "InteractiveBrowserCredential"
  )
} |
  ConvertTo-Json -Depth 5 |
  Set-Content -LiteralPath $metadataPath -Encoding utf8

$settings = @{
  CLARK_ARTIFACT_SIGNTOOL = $signTool.FullName
  CLARK_ARTIFACT_SIGNING_DLIB = $dlib.FullName
  CLARK_ARTIFACT_SIGNING_METADATA = $metadataPath
  CLARK_WINDOWS_SIGNER_SUBJECT = $ExpectedSignerSubject
}
foreach ($setting in $settings.GetEnumerator()) {
  [Environment]::SetEnvironmentVariable($setting.Key, $setting.Value, "Process")
  if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) {
    "$($setting.Key)=$($setting.Value)" >> $env:GITHUB_ENV
  }
}
