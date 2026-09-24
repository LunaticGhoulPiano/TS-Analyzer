#requires -Version 7.0
param(
  [Parameter(Mandatory=$true)][string]$OutputDirectory,
  [Parameter(Mandatory=$true)][string]$Compiler
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $root) { throw 'Use a fresh test output directory' }
New-Item -ItemType Directory -Path $root -Force | Out-Null
$id = 'TSAnalyzerTest' + [guid]::NewGuid().ToString('N')
$group = 'TS Analyzer Test ' + $id
$registry = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${id}_is1"
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) "$group/TS Analyzer.lnk"
$target = Join-Path $root 'installed app'
$builder = Join-Path $PSScriptRoot '../../packaging/windows/build-installer.ps1'
$installed = $false
. (Join-Path $PSScriptRoot 'update-worker-test.ps1')
$restartFixture = New-RestartFixture $root
function Run-Installer([string]$Executable, [string[]]$Arguments) {
  $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
  if ($process.ExitCode -ne 0) { throw "Installer returned $($process.ExitCode): $Executable" }
}
try {
  foreach ($version in @('0.2.1','0.2.2','0.2.3')) {
    $package = Join-Path $root "package-$version"
    New-Item -ItemType Directory -Path (Join-Path $package 'app'),(Join-Path $package 'scripts/windows') -Force | Out-Null
    foreach ($name in @('TS-Analyzer.exe','app/tsan-gui.exe')) {
      "Fixture $version; never executed" | Set-Content -LiteralPath (Join-Path $package $name)
    }
    Copy-Item -LiteralPath $restartFixture -Destination (Join-Path $package 'TS-Analyzer.exe')
    "release check $version" | Set-Content -LiteralPath (Join-Path $package 'scripts/windows/fetch-release.ps1')
    ('version = "' + $version + '"'),'target = "windows-x86_64"' | Set-Content -LiteralPath (Join-Path $package 'package.toml')
    'schema = 1','app_id = "TSAnalyzer"','mode = "portable"' | Set-Content -LiteralPath (Join-Path $package 'deployment.toml')
    $out = Join-Path $root "installer-$version"
    & $builder -Package $package -OutputDirectory $out -DeployDirectory $out -Compiler $Compiler
    # Only the ignored test copy receives a unique registry identity. The product template stays fixed.
    $script = Join-Path $out 'installer.iss'
    $text = Get-Content -Raw -LiteralPath $script
    if ($text -notmatch '(?m)^AppId=TSAnalyzer$' -or $text -notmatch '(?m)^UsePreviousAppDir=yes$') { throw 'Product installer identity or upgrade policy changed' }
    $text.Replace('AppId=TSAnalyzer', "AppId=$id").Replace('DefaultGroupName=TS Analyzer', "DefaultGroupName=$group") |
      Set-Content -LiteralPath $script -Encoding utf8
    & $Compiler $script *> (Join-Path $out 'test-compiler.log')
    if ($LASTEXITCODE -ne 0) { throw 'Could not compile isolated installer' }
    $exe = Join-Path $out "TS-Analyzer-windows-v$version-x86_64-setup.exe"
    $arguments = @('/SP-','/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/LOG="' + (Join-Path $out 'install.log') + '"'))
    if ($version -eq '0.2.1') { $arguments += '/DIR="' + $target + '"' }
    # The second install intentionally has no /DIR: it must discover and reuse the first location.
    if ($version -eq '0.2.3') {
      $stage = Join-Path $root 'update-stage'
      New-Item -ItemType Directory -Path $stage | Out-Null
      Copy-Item -LiteralPath $exe -Destination (Join-Path $stage 'installer.exe')
      Invoke-UpdateWorkerTest $target $stage 'installed' $version
    } else {
      Run-Installer $exe $arguments
    }
    $installed = $true
    $registration = Get-ItemProperty -LiteralPath $registry
    if ($registration.DisplayVersion -ne $version -or $registration.InstallLocation.TrimEnd('\') -ne $target) { throw 'Upgrade created a different installation' }
    if (-not (Test-Path -LiteralPath $shortcut)) { throw 'Start menu shortcut missing' }
    if ((Get-Content -Raw -LiteralPath (Join-Path $target 'deployment.toml')) -notmatch 'mode = "installed"') { throw 'Installed marker missing' }
    if ((Get-Content -Raw -LiteralPath (Join-Path $target 'app/tsan-gui.exe')).Trim() -ne "Fixture $version; never executed") { throw 'Application payload not replaced' }
    if ((Get-Content -Raw -LiteralPath (Join-Path $target 'scripts/windows/fetch-release.ps1')).Trim() -ne "release check $version") { throw 'Shared update script was not installed or upgraded' }
    if ($version -eq '0.2.1') {
      New-Item -ItemType Directory -Path (Join-Path $target 'data') | Out-Null
      'preserve settings' | Set-Content -LiteralPath (Join-Path $target 'data/tsan-config.toml')
    }
    if ((Get-Content -Raw -LiteralPath (Join-Path $target 'data/tsan-config.toml')).Trim() -ne 'preserve settings') { throw 'User data changed during upgrade' }
    Write-Output "PASS installer $version same identity, directory, shortcut and user data"
  }
} finally {
  $uninstaller = Join-Path $target 'unins000.exe'
  if (Test-Path -LiteralPath $uninstaller) {
    Run-Installer $uninstaller @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/LOG="' + (Join-Path $root 'uninstall.log') + '"'))
  }
}
if (-not $installed) { throw 'No installation was tested' }
foreach ($relative in @('TS-Analyzer.exe','app/tsan-gui.exe','scripts/windows/fetch-release.ps1','package.toml','deployment.toml','unins000.exe')) {
  if (Test-Path -LiteralPath (Join-Path $target $relative)) { throw "Uninstall left a managed file: $relative" }
}
if ((Test-Path -LiteralPath $registry) -or (Test-Path -LiteralPath $shortcut)) { throw 'Uninstall left registration or shortcut' }
if ((Get-Content -Raw -LiteralPath (Join-Path $target 'data/tsan-config.toml')).Trim() -ne 'preserve settings') { throw 'Uninstall deleted user data' }
Write-Output 'PASS uninstall removes managed files, registration and shortcut; preserves user data'
