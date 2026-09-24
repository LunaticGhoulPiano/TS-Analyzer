#requires -Version 7.0
[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][string]$Package,
  [Parameter(Mandatory=$true)][string]$OutputDirectory,
  [Parameter(Mandatory=$true)][string]$DeployDirectory,
  [Parameter(Mandatory=$true)][string]$Compiler
)
$ErrorActionPreference = 'Stop'
$packageRoot = (Resolve-Path -LiteralPath $Package).ProviderPath.TrimEnd('\','/')
$out = [IO.Path]::GetFullPath($OutputDirectory)
$deploy = [IO.Path]::GetFullPath($DeployDirectory)
if ($deploy.Equals($packageRoot, [StringComparison]::OrdinalIgnoreCase) -or
    $deploy.StartsWith($packageRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Installer deployment must be outside the package input directory.' }
if ($out.Equals($packageRoot, [StringComparison]::OrdinalIgnoreCase) -or
    $out.StartsWith($packageRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Installer output must be outside the package input directory.'
}
if (-not (Test-Path -LiteralPath $Compiler -PathType Leaf)) { throw 'Inno Setup compiler missing. Set InnoCompiler or ISCC_EXE; no automatic installation is performed.' }
foreach ($relative in @('package.toml','deployment.toml','TS-Analyzer.exe','app/tsan-gui.exe')) {
  if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $relative) -PathType Leaf)) { throw "Package is missing $relative" }
}
$manifest = Get-Content -Raw -LiteralPath (Join-Path $packageRoot 'package.toml')
$version = [regex]::Match($manifest, '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$').Groups[1].Value
if (-not $version -or $manifest -notmatch '(?m)^target\s*=\s*"windows-x86_64"\s*$') { throw 'Expected a versioned Windows x86_64 package.' }
$deployment = Get-Content -Raw -LiteralPath (Join-Path $packageRoot 'deployment.toml')
if ($deployment -notmatch '(?m)^mode = "portable"\s*$' -or
    $deployment -notmatch '(?m)^app_id = "TSAnalyzer"\s*$') { throw 'Build the installer from a current portable package.' }
$artifact = Join-Path $deploy "TS-Analyzer-windows-v$version-x86_64-setup.exe"
$script = Join-Path $out 'installer.iss'
if ((Test-Path -LiteralPath $script) -or (Test-Path -LiteralPath $artifact) -or (Test-Path -LiteralPath "$artifact.sha256")) { throw 'Use a fresh installer output directory.' }
New-Item -ItemType Directory -Force -Path $out, $deploy | Out-Null
@'
schema = 1
app_id = "TSAnalyzer"
mode = "installed"
'@ | Set-Content -LiteralPath (Join-Path $out 'installed.toml') -Encoding utf8
$template = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'ts-analyzer.iss')
# Keep generated scripts relocatable; cross-drive inputs are supplied only for this process.
$relativePackage = [IO.Path]::GetRelativePath($out, $packageRoot)
$packageDefine = if ([IO.Path]::IsPathRooted($relativePackage)) {
  '#define PackageDir GetEnv("TSAN_INSTALLER_PACKAGE")'
} else {
  '#define PackageDir SourcePath + "\' + $relativePackage + '"'
}
$relativeDeploy = [IO.Path]::GetRelativePath($out, $deploy)
$deployDefine = if ([IO.Path]::IsPathRooted($relativeDeploy)) {
  '#define InstallerOutput GetEnv("TSAN_INSTALLER_OUTPUT")'
} else {
  '#define InstallerOutput SourcePath + "\' + $relativeDeploy + '"'
}
$defines = @(
  $packageDefine
  '#define PackageVersion "' + $version + '"'
  $deployDefine
)
($defines -join [Environment]::NewLine) + [Environment]::NewLine + $template | Set-Content -LiteralPath $script -Encoding utf8
$log = Join-Path $out 'compiler.log'
$previousPackage = [Environment]::GetEnvironmentVariable('TSAN_INSTALLER_PACKAGE')
$previousOutput = [Environment]::GetEnvironmentVariable('TSAN_INSTALLER_OUTPUT')
try {
  $env:TSAN_INSTALLER_PACKAGE = $packageRoot
  $env:TSAN_INSTALLER_OUTPUT = $deploy
  & $Compiler $script *> $log
} finally {
  foreach ($entry in @{ TSAN_INSTALLER_PACKAGE = $previousPackage; TSAN_INSTALLER_OUTPUT = $previousOutput }.GetEnumerator()) {
    if ($null -eq $entry.Value) { Remove-Item -LiteralPath ("Env:" + $entry.Key) -ErrorAction SilentlyContinue }
    else { [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process') }
  }
}
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
  Get-Content -LiteralPath $log -Tail 25 | Write-Output
  throw "Inno Setup compilation failed; see $log"
}
$digest = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
"$digest  $([IO.Path]::GetFileName($artifact))" | Set-Content -LiteralPath "$artifact.sha256" -Encoding utf8
Write-Output "Compiler log: $log"
Write-Output "Installer: $artifact"
Write-Output "Generated script: $script"
Write-Output 'The installer upgrades the same application identity and preserves user configuration. This command does not install or publish.'
