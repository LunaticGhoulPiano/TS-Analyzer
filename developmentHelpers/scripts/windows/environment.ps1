# Shared by command-line and VS Code tasks. No global environment changes.
Set-StrictMode -Version Latest

function Resolve-DevelopmentPath([string]$Value, [string]$Repository) {
  if ([string]::IsNullOrWhiteSpace($Value)) { return '' }
  $expanded = [Environment]::ExpandEnvironmentVariables($Value)
  if ([IO.Path]::IsPathRooted($expanded)) { return [IO.Path]::GetFullPath($expanded) }
  return [IO.Path]::GetFullPath([IO.Path]::Combine($Repository, $expanded))
}

function Find-DevelopmentCommand([string]$Name) {
  $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($command) { return $command.Source }
  return ''
}

function Get-DevelopmentConfiguration([string]$Repository, [string]$Configuration) {
  $settings = @{
    GStreamerRoot = ''; TSDuckRoot = ''; TexRoot = ''; MsvcRuntime = ''; MsvcLicenses = ''
    InnoCompiler = ''; TexRecorder = ''; Package = ''
    Recordings = 'developmentHelpers/test-data/inputs/local'
    OutputDirectory = 'developmentHelpers/outputs/windows'
    DeployDirectory = 'developmentHelpers/outputs/deploy/windows'
  }
  if (-not $Configuration) { $Configuration = Join-Path $Repository 'developmentHelpers/config/windows.local.psd1' }
  else {
    $Configuration = Resolve-DevelopmentPath $Configuration $Repository
    if (-not (Test-Path -LiteralPath $Configuration -PathType Leaf)) { throw "Configuration does not exist: $Configuration" }
  }
  if (Test-Path -LiteralPath $Configuration) {
    $local = Import-PowerShellDataFile -LiteralPath $Configuration
    foreach ($key in $local.Keys) {
      if (-not $settings.ContainsKey($key)) { throw "Unknown development setting: $key" }
      $settings[$key] = [string]$local[$key]
    }
  }
  $environmentKeys = @{
    GStreamerRoot = 'GSTREAMER_1_0_ROOT_MSVC_X86_64'; TSDuckRoot = 'TSDUCK_HOME'
    TexRoot = 'TSAN_DEV_TEX_ROOT'; MsvcRuntime = 'TSAN_DEV_MSVC_RUNTIME'; MsvcLicenses = 'TSAN_DEV_MSVC_LICENSES'
    InnoCompiler = 'ISCC_EXE'; TexRecorder = 'TSAN_DEV_TEX_RECORDER'
    Recordings = 'TSAN_TEST_RECORDINGS'; OutputDirectory = 'TSAN_DEV_OUTPUT'; DeployDirectory = 'TSAN_DEV_DEPLOY'
  }
  foreach ($key in $environmentKeys.Keys) {
    $value = [Environment]::GetEnvironmentVariable($environmentKeys[$key])
    if ($value) { $settings[$key] = $value }
  }
  foreach ($key in @($settings.Keys)) {
    $settings[$key] = Resolve-DevelopmentPath $settings[$key] $Repository
  }
  $programFiles = [Environment]::GetFolderPath('ProgramFiles')
  if (-not $settings.GStreamerRoot) {
    $gst = Find-DevelopmentCommand 'gst-inspect-1.0.exe'
    if ($gst) { $settings.GStreamerRoot = Split-Path -Parent (Split-Path -Parent $gst) }
    elseif ($programFiles) { $settings.GStreamerRoot = Join-Path $programFiles 'gstreamer/1.0/msvc_x86_64' }
  }
  if (-not $settings.TSDuckRoot -and $programFiles) { $settings.TSDuckRoot = Join-Path $programFiles 'TSDuck' }
  if (-not $settings.TexRoot) {
    $tex = Find-DevelopmentCommand 'xelatex.exe'
    if ($tex) { $settings.TexRoot = [IO.Path]::GetFullPath((Join-Path (Split-Path -Parent $tex) '../..')) }
  }
  if (-not $settings.InnoCompiler) {
    $settings.InnoCompiler = Find-DevelopmentCommand 'ISCC.exe'
    if (-not $settings.InnoCompiler) {
      foreach ($base in @($programFiles, [Environment]::GetEnvironmentVariable('ProgramFiles(x86)'), (Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs'))) {
        if (-not $base) { continue }
        foreach ($version in @(7,6)) {
          $candidate = Join-Path $base "Inno Setup $version/ISCC.exe"
          if (Test-Path -LiteralPath $candidate -PathType Leaf) { $settings.InnoCompiler = $candidate; break }
        }
        if ($settings.InnoCompiler) { break }
      }
    }
  }
  return $settings
}

function Find-MsvcRuntime {
  $base = [Environment]::GetEnvironmentVariable('ProgramFiles(x86)')
  if (-not $base) { return '' }
  $vswhere = Join-Path $base 'Microsoft Visual Studio/Installer/vswhere.exe'
  if (-not (Test-Path -LiteralPath $vswhere)) { return '' }
  $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
  if ($LASTEXITCODE -ne 0 -or -not $installation) { return '' }
  $redist = Join-Path $installation 'VC/Redist/MSVC'
  if (-not (Test-Path -LiteralPath $redist)) { return '' }
  foreach ($version in (Get-ChildItem -LiteralPath $redist -Directory | Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } | Sort-Object { [version]$_.Name } -Descending)) {
    $x64 = Join-Path $version.FullName 'x64'
    if (Test-Path -LiteralPath $x64) {
      $crt = Get-ChildItem -LiteralPath $x64 -Directory -Filter 'Microsoft.VC*.CRT' | Select-Object -First 1
      if ($crt) { return $crt.FullName }
    }
  }
  return ''
}

function Enable-DevelopmentEnvironment($Settings) {
  foreach ($required in @(
    (Join-Path $Settings.GStreamerRoot 'lib/pkgconfig/gstreamer-1.0.pc'),
    (Join-Path $Settings.TSDuckRoot 'include/tsduck/tsTSPacket.h'),
    (Join-Path $Settings.TSDuckRoot 'bin/tsduck.dll')
  )) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
      throw "Missing SDK file: $required. Set developmentHelpers/config/windows.local.psd1 or the documented environment variable."
    }
  }
  $env:TSDUCK_HOME = $Settings.TSDuckRoot
  $env:PATH = (Join-Path $Settings.GStreamerRoot 'bin') + ';' + (Join-Path $Settings.TSDuckRoot 'bin') + ';' + $env:PATH
  $pc = Join-Path $Settings.GStreamerRoot 'lib/pkgconfig'
  $env:PKG_CONFIG_PATH = $pc + $(if ($env:PKG_CONFIG_PATH) { ';' + $env:PKG_CONFIG_PATH })
}

function Invoke-DevelopmentTool([string]$Executable, [string[]]$Arguments) {
  & $Executable @Arguments
  if ($LASTEXITCODE -ne 0) { throw "$Executable failed with exit code $LASTEXITCODE" }
}

function New-DevelopmentOutput([string]$Root, [string]$Category) {
  $directory = Join-Path (Join-Path $Root $Category) ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss-fff') + "-$PID")
  if (Test-Path -LiteralPath $directory) { throw "Output already exists: $directory" }
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $directory) -ErrorAction Stop | Out-Null
  return $directory
}

function Find-MsvcLicenses([string]$RuntimeDirectory) {
  if (-not $RuntimeDirectory -or -not (Test-Path -LiteralPath $RuntimeDirectory -PathType Container)) { return '' }
  # Discover the selected SDK's installation root by its VC and Licenses siblings.
  $directory = Get-Item -LiteralPath $RuntimeDirectory
  while ($directory) {
    $licenses = Join-Path $directory.FullName 'Licenses'
    if ((Test-Path -LiteralPath (Join-Path $directory.FullName 'VC') -PathType Container) -and
        (Test-Path -LiteralPath $licenses -PathType Container)) { return $licenses }
    $directory = $directory.Parent
  }
  return ''
}
