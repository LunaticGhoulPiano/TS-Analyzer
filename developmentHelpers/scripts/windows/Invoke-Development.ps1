#requires -Version 7.0
[CmdletBinding()]
param(
  [ValidateSet('CheckEnvironment','Build','Run','Test','CI','GenerateFixtures','Package','Installer','VerifyDevelopment','VerifyUpdate','VerifyRuntime','VerifyReports','VerifyInstallation')]
  [string]$Action = 'CheckEnvironment',
  [ValidateSet('Debug','Release')][string]$Profile = 'Debug',
  [string]$Configuration,
  [string]$Package,
  [string]$Recordings,
  [string]$TexRecorder,
  [string]$OutputDirectory,
  [string]$DeployDirectory
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This entry point requires Windows. Platform-independent Cargo tests can run directly on other systems.' }
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
. (Join-Path $PSScriptRoot 'environment.ps1')
$settings = Get-DevelopmentConfiguration $repository $Configuration
foreach ($name in @('Package','Recordings','TexRecorder','OutputDirectory','DeployDirectory')) {
  $value = Get-Variable -Name $name -ValueOnly
  if ($value) { $settings[$name] = Resolve-DevelopmentPath $value $repository }
}
$testScripts = Join-Path $repository 'developmentHelpers/tests/windows'
$packageScripts = Join-Path $repository 'developmentHelpers/packaging/windows'

function Require-Package {
  $selected = $settings.Package
  if (-not $selected -or -not (Test-Path -LiteralPath (Join-Path $selected 'package.toml') -PathType Leaf)) {
    throw 'Specify -Package <unpacked package directory> or Package in developmentHelpers/config/windows.local.psd1.'
  }
  return $selected
}
function Require-Recordings {
  if (-not (Test-Path -LiteralPath $settings.Recordings -PathType Container) -or
      -not (Get-ChildItem -LiteralPath $settings.Recordings -Filter '*.ts' -File | Select-Object -First 1)) {
    throw 'No .ts recordings found. Set -Recordings, TSAN_TEST_RECORDINGS, or Recordings in windows.local.psd1.'
  }
}
function Require-RustToolchain {
  $toolchainText = Get-Content -Raw -LiteralPath (Join-Path $repository 'rust-toolchain.toml')
  $channel = [regex]::Match($toolchainText, '(?m)^channel\s*=\s*"([^"]+)"').Groups[1].Value
  if (-not (Find-DevelopmentCommand 'rustup.exe')) { throw 'Rustup is required; no tools will be installed automatically.' }
  $installed = & rustup toolchain list
  if ($LASTEXITCODE -ne 0 -or -not ($installed | Where-Object { $_.StartsWith("$channel-") -or $_.StartsWith("$channel ") })) {
    throw "Rust $channel is not installed. Install the pinned toolchain explicitly before building."
  }
}
# Restore the calling PowerShell session too, not just the parent VS Code process.
$environmentNames = @('PATH','PKG_CONFIG_PATH','TSDUCK_HOME','TSAN_CONFIG_PATH','TSAN_DIAGNOSTICS_DIR','TSAN_HEADLESS_TEST','TSAN_ERROR_FILE','TSAN_PACKAGE_VERSION','TSAN_UPDATE_STAGE','TSAN_UPDATE_VERSION','TSAN_UPDATE_SHA256','TSAN_UPDATE_BYTES','TSAN_UPDATE_MODE')
$previousEnvironment = @{}
foreach ($name in $environmentNames) { $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name) }
Push-Location -LiteralPath $repository
try {
  if ($Action -in @('Build','Run','Test','CI','Package','GenerateFixtures','VerifyUpdate','VerifyInstallation')) { Require-RustToolchain }
  if ($Action -in @('Build','Run','Test','CI','Package')) { Enable-DevelopmentEnvironment $settings }
  switch ($Action) {
    'CheckEnvironment' {
      foreach ($name in @('GStreamerRoot','TSDuckRoot','TexRoot','InnoCompiler')) {
        $value = $settings[$name]
        [pscustomobject]@{ Setting = $name; Path = $value; Exists = [bool]($value -and (Test-Path -LiteralPath $value)) }
      }
      foreach ($tool in @('cargo.exe','rustup.exe','pkg-config.exe')) {
        $value = Find-DevelopmentCommand $tool
        [pscustomobject]@{ Setting = $tool; Path = $value; Exists = [bool]$value }
      }
    }
    'Build' {
      $arguments = @('build','--locked','--offline','--workspace')
      if ($Profile -eq 'Release') { $arguments += '--release' }
      Invoke-DevelopmentTool 'cargo' $arguments
    }
    'Run' {
      $env:TSAN_CONFIG_PATH = Join-Path $settings.OutputDirectory 'state/tsan-config.toml'
      $env:TSAN_DIAGNOSTICS_DIR = Join-Path $settings.OutputDirectory 'diagnostics'
      $arguments = @('run','--locked','--offline','-p','tsan-gui')
      if ($Profile -eq 'Release') { $arguments += '--release' }
      Invoke-DevelopmentTool 'cargo' $arguments
    }
    'Test' { Invoke-DevelopmentTool 'cargo' @('test','--locked','--offline','--workspace') }
    'CI' {
      Invoke-DevelopmentTool 'cargo' @('fmt','--all','--','--check')
      & (Join-Path $testScripts 'test-development.ps1')
      $licenseOut = New-DevelopmentOutput $settings.OutputDirectory 'license-tests'
      & (Join-Path $testScripts 'test-rust-licenses.ps1') -OutputDirectory $licenseOut
      Invoke-DevelopmentTool 'cargo' @('test','--locked','--offline','--workspace')
      $out = New-DevelopmentOutput $settings.OutputDirectory 'update-tests'
      & (Join-Path $testScripts 'test-update-archive.ps1') -OutputDirectory $out
      $portableOut = New-DevelopmentOutput $settings.OutputDirectory 'portable-update-tests'
      $windowsPowerShell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
      Invoke-DevelopmentTool $windowsPowerShell @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',(Join-Path $testScripts 'test-fetch-release.ps1'))
      Invoke-DevelopmentTool $windowsPowerShell @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $testScripts 'test-portable-update.ps1'),'-OutputDirectory',$portableOut)
    }
    'GenerateFixtures' {
      Invoke-DevelopmentTool 'cargo' @('run','--locked','--offline','-p','tsan-core','--example','generate_test_data')
    }
    'VerifyDevelopment' { & (Join-Path $testScripts 'test-development.ps1') }
    'VerifyUpdate' {
      $out = New-DevelopmentOutput $settings.OutputDirectory 'update-tests'
      & (Join-Path $testScripts 'test-update-archive.ps1') -OutputDirectory $out
      $portableOut = New-DevelopmentOutput $settings.OutputDirectory 'portable-update-tests'
      $windowsPowerShell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
      Invoke-DevelopmentTool $windowsPowerShell @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',(Join-Path $testScripts 'test-fetch-release.ps1'))
      Invoke-DevelopmentTool $windowsPowerShell @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $testScripts 'test-portable-update.ps1'),'-OutputDirectory',$portableOut)
    }
    'Package' {
      if (-not $settings.TexRecorder -or -not (Test-Path -LiteralPath $settings.TexRecorder -PathType Leaf)) {
        throw 'Package requires -TexRecorder <report.fls> from a representative complete XeLaTeX report.'
      }
      if (-not $settings.MsvcRuntime) { $settings.MsvcRuntime = Find-MsvcRuntime }
      if (-not $settings.MsvcLicenses) { $settings.MsvcLicenses = Find-MsvcLicenses $settings.MsvcRuntime }
      if (-not $settings.MsvcLicenses) { throw 'MSVC licenses not found. Set MsvcLicenses or TSAN_DEV_MSVC_LICENSES for the selected runtime.' }
      if (-not $settings.MsvcRuntime -or -not $settings.TexRoot) { throw 'Configure MsvcRuntime and TexRoot in windows.local.psd1.' }
      Invoke-DevelopmentTool 'cargo' @('build','--locked','--offline','--release','-p','tsan-gui')
      $metadata = & cargo metadata --offline --locked --no-deps --format-version 1
      if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
      $target = ($metadata | ConvertFrom-Json).target_directory
      $out = New-DevelopmentOutput $settings.OutputDirectory 'packages'
      & (Join-Path $packageScripts 'build-package.ps1') -OutputDirectory $out -DeployDirectory $settings.DeployDirectory -MsvcLicenses $settings.MsvcLicenses -GStreamerRoot $settings.GStreamerRoot -TSDuckRoot $settings.TSDuckRoot -MsvcRuntime $settings.MsvcRuntime -TexRoot $settings.TexRoot -TexRecorder $settings.TexRecorder -AppExe (Join-Path $target 'release/tsan-gui.exe')
    }
    'Installer' {
      if (-not $settings.InnoCompiler) { throw 'Inno Setup compiler not found. Set InnoCompiler or ISCC_EXE.' }
      $selected = Require-Package
      $out = New-DevelopmentOutput $settings.OutputDirectory 'installers'
      & (Join-Path $packageScripts 'build-installer.ps1') -Package $selected -OutputDirectory $out -DeployDirectory $settings.DeployDirectory -Compiler $settings.InnoCompiler
    }
    'VerifyInstallation' {
      if (-not $settings.InnoCompiler) { throw 'Inno Setup compiler not found. Set InnoCompiler or ISCC_EXE.' }
      $out = New-DevelopmentOutput $settings.OutputDirectory 'installation-tests'
      & (Join-Path $testScripts 'test-installation.ps1') -Compiler $settings.InnoCompiler -OutputDirectory $out
    }
    'VerifyRuntime' {
      $selected = Require-Package
      Require-Recordings
      $out = New-DevelopmentOutput $settings.OutputDirectory 'runtime-tests'
      & (Join-Path $testScripts 'test-runtime.ps1') -Package $selected -Recordings $settings.Recordings -OutputDirectory $out -GStreamerTools (Join-Path $settings.GStreamerRoot 'bin')
    }
    'VerifyReports' {
      $selected = Require-Package
      Require-Recordings
      $out = New-DevelopmentOutput $settings.OutputDirectory 'report-tests'
      & (Join-Path $testScripts 'test-report-bundle.ps1') -Package $selected -Recordings $settings.Recordings -OutputDirectory $out
    }
  }
} finally {
  Pop-Location
  foreach ($name in $environmentNames) {
    if ($null -eq $previousEnvironment[$name]) { Remove-Item -LiteralPath ("Env:" + $name) -ErrorAction SilentlyContinue }
    else { [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process') }
  }
}
