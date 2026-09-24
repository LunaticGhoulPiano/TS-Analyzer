#requires -Version 7.0
param([Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
. (Join-Path $repository 'developmentHelpers/packaging/windows/deployment.ps1')
$out = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $out) { throw 'Use a fresh deployment test directory.' }
$artifacts = Join-Path $out 'artifacts with spaces'
$deploy = Join-Path $out 'delivery with spaces'
$version = '0.0.0'
$name = Get-WindowsPackageName $version
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
function Write-Artifact([string]$Filename) {
  $file = Join-Path $artifacts $Filename
  'Build fixture; not an executable.' | Set-Content -LiteralPath $file -Encoding ascii
  $digest = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
  "$digest  $Filename" | Set-Content -LiteralPath "$file.sha256" -Encoding utf8
}
function Assert-Rejected([string]$Destination, [string]$Reason) {
  $rejected = $false
  try { $null = Complete-WindowsDeployment $artifacts $Destination $version } catch { $rejected = $true }
  if (-not $rejected) { throw "Accepted invalid deployment: $Reason" }
}
Write-Artifact "$name-portable.zip"
Assert-Rejected $deploy 'installer failed or missing'
if (Test-Path -LiteralPath $deploy) { throw 'A partial build created a delivery directory.' }
Write-Artifact "$name-setup.exe"
$finished = Complete-WindowsDeployment $artifacts $deploy $version
if ($finished -ne (Join-Path $deploy $name)) { throw 'Delivery folder does not use the platform/version/architecture name.' }
if (@(Get-ChildItem -LiteralPath $finished -Force).Count -ne 4) { throw 'Delivery must contain exactly four files.' }
foreach ($source in Get-ChildItem -LiteralPath $artifacts -File) {
  if ((Get-FileHash -LiteralPath $source.FullName).Hash -ne (Get-FileHash -LiteralPath (Join-Path $finished $source.Name)).Hash) { throw 'Delivered bytes changed.' }
}
$before = (Get-FileHash -LiteralPath (Join-Path $finished "$name-setup.exe")).Hash
Assert-Rejected $deploy 'duplicate version'
if ((Get-FileHash -LiteralPath (Join-Path $finished "$name-setup.exe")).Hash -ne $before) { throw 'Existing delivery was modified.' }
'tampered' | Add-Content -LiteralPath (Join-Path $artifacts "$name-setup.exe")
$invalid = Join-Path $out 'invalid delivery'
Assert-Rejected $invalid 'checksum mismatch'
if (Test-Path -LiteralPath $invalid) { throw 'Invalid checksums created a delivery directory.' }
Write-Artifact "$name-setup.exe"
'development log' | Set-Content -LiteralPath (Join-Path $artifacts 'compiler.log')
Assert-Rejected $invalid 'intermediate file'
$rejected = $false
try { $null = Get-WindowsPackageName '../escape' } catch { $rejected = $true }
if (-not $rejected) { throw 'Release name accepted a path traversal.' }
if (@(Get-ChildItem -LiteralPath $deploy -Force).Count -ne 1) { throw 'Temporary staging directory was left in the delivery root.' }
Write-Output 'PASS complete versioned deployment: both formats, checksums, spaces, partial-build rejection, collision preservation, and no intermediate files'
