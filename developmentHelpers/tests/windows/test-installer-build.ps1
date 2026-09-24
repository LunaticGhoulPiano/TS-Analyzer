#requires -Version 7.0
[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][string]$Compiler,
  [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$out = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $out) { throw 'Use a fresh test output directory.' }
# These are compiler fixtures only. No installer or product is executed.
$package = Join-Path $out 'package with spaces'
$work = Join-Path $out 'work with spaces'
$deploy = Join-Path $out 'delivery with spaces'
New-Item -ItemType Directory -Force -Path (Join-Path $package 'app') | Out-Null
@'
version = "0.0.0"
target = "windows-x86_64"
'@ | Set-Content -LiteralPath (Join-Path $package 'package.toml') -Encoding utf8
@'
schema = 1
app_id = "TSAnalyzer"
mode = "portable"
'@ | Set-Content -LiteralPath (Join-Path $package 'deployment.toml') -Encoding utf8
foreach ($relative in @('TS-Analyzer.exe','app/tsan-gui.exe')) {
  'Non-executable installer compiler fixture' | Set-Content -LiteralPath (Join-Path $package $relative) -Encoding ascii
}
$build = Join-Path $repository 'developmentHelpers/packaging/windows/build-installer.ps1'
$oldPackage = [Environment]::GetEnvironmentVariable('TSAN_INSTALLER_PACKAGE')
$oldOutput = [Environment]::GetEnvironmentVariable('TSAN_INSTALLER_OUTPUT')
& $build -Package $package -OutputDirectory $work -DeployDirectory $deploy -Compiler $Compiler
if ([Environment]::GetEnvironmentVariable('TSAN_INSTALLER_PACKAGE') -cne $oldPackage -or
    [Environment]::GetEnvironmentVariable('TSAN_INSTALLER_OUTPUT') -cne $oldOutput) { throw 'Build leaked installer environment variables.' }
$artifact = Join-Path $deploy 'TS-Analyzer-windows-v0.0.0-x86_64-setup.exe'
$expected = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
$checksum = Get-Content -Raw -LiteralPath "$artifact.sha256"
if ($checksum.Trim() -cne "$expected  $([IO.Path]::GetFileName($artifact))") { throw 'Installer checksum mismatch.' }
if (@(Get-ChildItem -LiteralPath $deploy -File).Count -ne 2) { throw 'Intermediate files leaked into delivery output.' }
foreach ($name in @('installer.iss','installed.toml','compiler.log')) {
  if (-not (Test-Path -LiteralPath (Join-Path $work $name) -PathType Leaf)) { throw "Missing work file: $name" }
}
$script = Get-Content -Raw -LiteralPath (Join-Path $work 'installer.iss')
if ($script.Contains($out)) { throw 'Generated installer script contains a fixed absolute path.' }
if ($script -notmatch '(?m)^AppId=TSAnalyzer$' -or $script -notmatch '(?m)^UsePreviousAppDir=yes$') { throw 'Installer upgrade identity changed.' }
$rejected = $false
try { & $build -Package $package -OutputDirectory (Join-Path $out 'duplicate work') -DeployDirectory $deploy -Compiler $Compiler } catch { $rejected = $true }
if (-not $rejected -or (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw 'Existing delivery artifact was overwritten.' }
$rejected = $false
try { & $build -Package $package -OutputDirectory (Join-Path $out 'nested work') -DeployDirectory (Join-Path $package 'nested delivery') -Compiler $Compiler } catch { $rejected = $true }
if (-not $rejected) { throw 'Delivery inside the input package was accepted.' }
Write-Output 'PASS compiler-only installer build: separate work/delivery paths, checksum, relative paths, stable identity, collision rejection, environment restoration'
