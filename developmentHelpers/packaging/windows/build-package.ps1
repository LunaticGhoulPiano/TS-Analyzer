param(
  [Parameter(Mandatory=$true)][string]$OutputDirectory,
  [Parameter(Mandatory=$true)][string]$DeployDirectory,
  [Parameter(Mandatory=$true)][string]$MsvcLicenses,
  [Parameter(Mandatory=$true)][string]$GStreamerRoot,
  [Parameter(Mandatory=$true)][string]$TSDuckRoot,
  [Parameter(Mandatory=$true)][string]$MsvcRuntime,
  [Parameter(Mandatory=$true)][string]$TexRoot,
  [Parameter(Mandatory=$true)][string]$TexRecorder,
  [string]$AppExe,
  [string]$Version,
  [switch]$SkipArchive
)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
. (Join-Path $PSScriptRoot 'pe-dependencies.ps1')
$platforms = Get-Content -Raw -LiteralPath (Join-Path $repository 'developmentHelpers/packaging/platforms.toml')
$platformSection = [regex]::Match($platforms, '(?ms)^\[windows\]\s*(.*?)(?=^\[|\z)').Groups[1].Value
$declaredVersion = [regex]::Match($platformSection, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $Version) { $Version = $declaredVersion }
if ($Version -ne $declaredVersion) { throw 'Package version must match developmentHelpers/packaging/platforms.toml [windows]' }
if (-not $AppExe) {
  Push-Location -LiteralPath $repository
  try {
    $targetMetadata = & cargo metadata --offline --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Could not resolve Cargo output directory' }
  } finally { Pop-Location }
  $AppExe = Join-Path ($targetMetadata | ConvertFrom-Json).target_directory 'release/tsan-gui.exe'
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Version must be major.minor.patch' }
$DeployDirectory = [IO.Path]::GetFullPath($DeployDirectory)
$package = [IO.Path]::GetFullPath((Join-Path $OutputDirectory "TS-Analyzer-windows-v$Version-x86_64"))
if ($DeployDirectory.Equals($package, [StringComparison]::OrdinalIgnoreCase) -or
    $DeployDirectory.StartsWith($package + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Archive delivery must be outside the package input directory.' }
if (Test-Path -LiteralPath $package) { throw "Package output already exists; use a new output directory: $package" }
function Copy-One([string]$Source, [string]$Relative) {
  if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) { throw "Missing runtime file: $Source" }
  $destination = Join-Path $package $Relative
  New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($destination)) | Out-Null
  Copy-Item -LiteralPath $Source -Destination $destination
}
function Copy-Tree([string]$Source, [string]$Relative) {
  if (-not (Test-Path -LiteralPath $Source -PathType Container)) { throw "Missing runtime directory: $Source" }
  $destination = Join-Path $package $Relative
  New-Item -ItemType Directory -Force -Path $destination | Out-Null
  Get-ChildItem -LiteralPath $Source | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $destination -Recurse }
}
New-Item -ItemType Directory -Force -Path $package | Out-Null
Copy-One $AppExe 'app/tsan-gui.exe'
Copy-One (Join-Path $repository 'scripts/windows/fetch-release.ps1') 'scripts/windows/fetch-release.ps1'
# Private, tested plugin set: no development headers, import libraries, PDBs or global registration.
$plugins = @('app','coreelements','mpegtsdemux','videoparsersbad','audioparsers',
  'd3d11','d3d12','playback','typefindfunctions','audioconvert','audioresample','wasapi2','libav')
foreach ($plugin in $plugins) {
  Copy-One (Join-Path $GStreamerRoot "lib/gstreamer-1.0/gst$plugin.dll") "runtime/gstreamer/lib/gstreamer-1.0/gst$plugin.dll"
}
Copy-One (Join-Path $GStreamerRoot 'libexec/gstreamer-1.0/gst-plugin-scanner.exe') 'runtime/gstreamer/libexec/gstreamer-1.0/gst-plugin-scanner.exe'
$gstSeeds = @($plugins | ForEach-Object { Join-Path $GStreamerRoot "lib/gstreamer-1.0/gst$_.dll" })
$gstSeeds += Join-Path $GStreamerRoot 'libexec/gstreamer-1.0/gst-plugin-scanner.exe'
foreach ($name in @(Get-PeImports $AppExe)) {
  $dependency = Join-Path $GStreamerRoot "bin/$name"
  if (Test-Path -LiteralPath $dependency) { Copy-One $dependency "runtime/gstreamer/bin/$name"; $gstSeeds += $dependency }
}
Copy-PeDependencies $gstSeeds @((Join-Path $GStreamerRoot 'bin'), $MsvcRuntime) 'runtime/gstreamer/bin'
Copy-Tree (Join-Path $GStreamerRoot 'share/licenses') 'licenses/gstreamer'
foreach ($name in @('tscore.dll','tsduck.dll')) { Copy-One (Join-Path $TSDuckRoot "bin/$name") "runtime/tsduck/$name" }
Get-ChildItem -LiteralPath (Join-Path $TSDuckRoot 'bin') -File |
  Where-Object { $_.Extension -in '.names','.xml','.model' } |
  ForEach-Object { Copy-One $_.FullName ('runtime/tsduck/' + $_.Name) }
Copy-One (Join-Path $TSDuckRoot 'LICENSE.txt') 'licenses/tsduck/LICENSE.txt'
Copy-One (Join-Path $TSDuckRoot 'OTHERS.txt') 'licenses/tsduck/OTHERS.txt'
Get-ChildItem -LiteralPath $MsvcRuntime -Filter '*.dll' |
  ForEach-Object { Copy-One $_.FullName ('runtime/msvc/' + $_.Name) }
if (-not (Test-Path -LiteralPath $MsvcLicenses -PathType Container)) { throw 'MSVC license directory is missing; configure MsvcLicenses for this SDK.' }
Copy-Tree $MsvcLicenses 'licenses/msvc'

# Copy the exact XeLaTeX input closure recorded by a representative complete report.
$texAbsolute = [IO.Path]::GetFullPath($TexRoot).TrimEnd('\','/')
$texPrefix = $texAbsolute + [IO.Path]::DirectorySeparatorChar
$inputs = @(Get-Content -LiteralPath $TexRecorder | Where-Object { $_.StartsWith('INPUT ') } |
  ForEach-Object { $_.Substring(6).Trim('"') } | Sort-Object -Unique)
$count = 0
foreach ($inputFile in $inputs) {
  if (-not [IO.Path]::IsPathRooted($inputFile)) { continue }
  $absolute = [IO.Path]::GetFullPath($inputFile)
  if ($absolute.StartsWith($texPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    Copy-One $absolute ('runtime/tex/' + $absolute.Substring($texPrefix.Length))
    $count++
  }
}
if ($count -lt 20) { throw 'The TeX recorder must come from a complete report built with the configured TexRoot (not a different bundled TeX runtime).' }
foreach ($name in @('xelatex.exe','xetex.exe','xdvipdfmx.exe','kpsewhich.exe')) {
  Copy-One (Join-Path $TexRoot "bin/windows/$name") "runtime/tex/bin/windows/$name"
}
$texSeeds = @('xelatex.exe','xetex.exe','xdvipdfmx.exe','kpsewhich.exe') | ForEach-Object { Join-Path $TexRoot "bin/windows/$_" }
Copy-PeDependencies $texSeeds @((Join-Path $TexRoot 'bin/windows'), $MsvcRuntime) 'runtime/tex/bin/windows'
Copy-One (Join-Path $TexRoot 'texmf-dist/web2c/texmf.cnf') 'runtime/tex/texmf-dist/web2c/texmf.cnf'
Copy-One (Join-Path $TexRoot 'texmf-var/web2c/xetex/xelatex.fmt') 'runtime/tex/texmf-var/web2c/xetex/xelatex.fmt'
foreach ($relative in @('fonts/opentype/public/lm-math',
  'fonts/tfm/public/lm','fonts/type1/public/lm','fonts/enc/dvips/lm','fonts/map/dvips/lm',
  'fonts/map/pdftex/updmap','fonts/map/dvipdfmx','dvipdfmx','tex/latex/l3backend')) {
  $source = Join-Path $TexRoot "texmf-dist/$relative"
  if (Test-Path -LiteralPath $source) { Copy-Tree $source "runtime/tex/texmf-dist/$relative" }
}
foreach ($name in @('lmroman10-regular.otf','lmroman10-bold.otf','lmroman10-italic.otf','lmroman10-bolditalic.otf',
  'lmmono10-regular.otf','lmsans10-regular.otf','lmsans10-bold.otf')) {
  Copy-One (Join-Path $TexRoot "texmf-dist/fonts/opentype/public/lm/$name") "runtime/tex/texmf-dist/fonts/opentype/public/lm/$name"
}
foreach ($name in @('FandolSong-Regular.otf','FandolSong-Bold.otf')) {
  Copy-One (Join-Path $TexRoot "texmf-dist/fonts/opentype/public/fandol/$name") "runtime/tex/texmf-dist/fonts/opentype/public/fandol/$name"
}
Copy-One (Join-Path $TexRoot 'LICENSE.TL') 'licenses/texlive/LICENSE.TL'
Copy-One (Join-Path $TexRoot 'LICENSE.CTAN') 'licenses/texlive/LICENSE.CTAN'
Copy-One (Join-Path $TexRoot 'release-texlive.txt') 'licenses/texlive/release-texlive.txt'
# Preserve package-specific notices for redistributed TeX macros/fonts.
$texDocs = @('base','fontspec','geometry','tools','booktabs','hyperref','graphics','xcolor',
  'l3kernel','l3backend','etoolbox','kvoptions','kvsetkeys','ltxcmds','auxhook','refcount',
  'gettitlestring','url','rerunfilecheck','atbegshi','atveryend','iftex','infwarerr','bitset',
  'intcalc','bigintcalc','pdfescape','hycolor','hopatch','letltxmacro','uniquecounter','lm','fandol')
foreach ($category in @('latex','generic','fonts')) {
  foreach ($name in $texDocs) {
    $source = Join-Path $TexRoot "texmf-dist/doc/$category/$name"
    if (Test-Path -LiteralPath $source) {
      Get-ChildItem -LiteralPath $source -Recurse -File |
        Where-Object { $_.Name -match '^(LICEN[SC]E|COPYING|NOTICE|README|AUTHORS|lppl)' -or $_.Extension -in '.txt','.md','.rst','.dtx','.ins' } |
        ForEach-Object { Copy-One $_.FullName ("licenses/texlive/packages/$category/$name/" + $_.FullName.Substring($source.Length + 1)) }
    }
  }
}
Copy-One (Join-Path $repository 'LICENSE') 'licenses/TS-Analyzer-LICENSE'
Copy-One (Join-Path $repository 'licenses/liquid-glass-notices.txt') 'licenses/liquid-glass-notices.txt'
Copy-One (Join-Path $repository 'Cargo.lock') 'licenses/Cargo.lock'
Copy-Tree (Join-Path $repository 'docs/usr') 'docs/usr'
$metadataText = & cargo metadata --offline --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $repository 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'Cargo dependency inventory failed' }
$metadata = $metadataText | ConvertFrom-Json
. (Join-Path $PSScriptRoot 'rust-licenses.ps1')
$licenseInventory = Copy-RustLicenses -Metadata $metadata -Repository $repository -Destination (Join-Path $package 'licenses')
Write-Output ("Rust license inventory: {0} locked dependencies" -f $licenseInventory.Count)
Push-Location -LiteralPath $repository
try {
  & cargo rustc --locked --offline --release -p tsan-launcher --target x86_64-pc-windows-msvc -- -C target-feature=+crt-static -C debuginfo=1 -C strip=none
  if ($LASTEXITCODE -ne 0) { throw 'Launcher build failed' }
  $launcherMetadata = & cargo metadata --offline --locked --no-deps --format-version 1
  if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
  $launcherTarget = ($launcherMetadata | ConvertFrom-Json).target_directory
  Copy-One (Join-Path $launcherTarget 'x86_64-pc-windows-msvc/release/tsan-launcher.exe') 'TS-Analyzer.exe'
} finally { Pop-Location }
# Keep matching PDBs alongside an identity manifest, outside every user package.
$symbols = Join-Path $OutputDirectory 'symbols'
New-Item -ItemType Directory -Force -Path $symbols | Out-Null
$symbolInventory = @()
foreach ($pair in @(
  @{ Executable = $AppExe; Pdb = Join-Path (Split-Path -Parent $AppExe) 'tsan_gui.pdb' },
  @{ Executable = Join-Path $launcherTarget 'x86_64-pc-windows-msvc/release/tsan-launcher.exe'; Pdb = Join-Path $launcherTarget 'x86_64-pc-windows-msvc/release/tsan_launcher.pdb' }
)) {
  if (-not (Test-Path -LiteralPath $pair.Pdb -PathType Leaf)) { throw "Matching release PDB is missing; rebuild with debug info: $($pair.Pdb)" }
  Copy-Item -LiteralPath $pair.Pdb -Destination (Join-Path $symbols ([IO.Path]::GetFileName($pair.Pdb)))
  $symbolInventory += [pscustomobject]@{
    Executable = [IO.Path]::GetFileName($pair.Executable)
    ExecutableSHA256 = (Get-FileHash -LiteralPath $pair.Executable -Algorithm SHA256).Hash.ToLowerInvariant()
    Pdb = [IO.Path]::GetFileName($pair.Pdb)
    PdbSHA256 = (Get-FileHash -LiteralPath $pair.Pdb -Algorithm SHA256).Hash.ToLowerInvariant()
    PlatformVersion = $Version
  }
}
$symbolInventory | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $symbols 'manifest.json') -Encoding utf8
@'
schema = 1
app_id = "TSAnalyzer"
mode = "portable"
'@ | Set-Content -Encoding utf8 -LiteralPath (Join-Path $package 'deployment.toml')
$appHash = (Get-FileHash -LiteralPath $AppExe -Algorithm SHA256).Hash.ToLowerInvariant()
$gstPc = Get-Content -Raw -LiteralPath (Join-Path $GStreamerRoot 'lib/pkgconfig/gstreamer-1.0.pc')
$gstVersion = [regex]::Match($gstPc, '(?m)^Version:\s*(\S+)').Groups[1].Value
$duckVersion = (Get-Item -LiteralPath (Join-Path $TSDuckRoot 'bin/tsduck.dll')).VersionInfo.FileVersion
$texVersion = [regex]::Match((Get-Content -Raw -LiteralPath (Join-Path $TexRoot 'release-texlive.txt')), 'version\s+(\d{4})').Groups[1].Value
if (-not $gstVersion -or -not $duckVersion -or -not $texVersion) { throw 'Runtime version metadata is missing' }
@"
schema = 1
version = "$Version"
target = "windows-x86_64"
minimum_os = "Windows 10 22H2 / Windows 11 (x64)"
app_sha256 = "$appHash"
gstreamer = "$gstVersion"
tsduck = "$duckVersion"
texlive = "$texVersion"
"@ | Set-Content -Encoding utf8 -LiteralPath (Join-Path $package 'package.toml')
$buildInfo = Join-Path $package 'build-info.toml'
$env:TSAN_HEADLESS_TEST = '1'
$env:TSAN_ERROR_FILE = Join-Path $package 'build-error.txt'
$check = Start-Process -FilePath (Join-Path $package 'TS-Analyzer.exe') -ArgumentList @('--wait','--build-info',('"' + $buildInfo + '"')) -WindowStyle Hidden -Wait -PassThru
if ($check.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $buildInfo)) { throw 'Packaged application could not report its build identity' }
$identity = Get-Content -Raw -LiteralPath $buildInfo
if ($identity -notmatch ('(?m)^version = "' + [regex]::Escape($Version) + '"\r?$') -or
    $identity -notmatch '(?m)^target = "windows-x86_64"\r?$') { throw 'Application version/target differs from the package; rebuild the application' }
@'
TS Analyzer for Windows x64
Portable: extract the entire ZIP and run TS-Analyzer.exe.
Portable configuration: data\tsan-config.toml beside TS-Analyzer.exe.
Portable cache and updates: data\cache and data\updates.
Windows installation: download the separate *-setup.exe installer from the same release.
Installed location defaults to %LOCALAPPDATA%\Programs\TS-Analyzer; custom locations are supported.
Installed configuration: %APPDATA%\TS-Analyzer\tsan-config.toml.
Diagnostics: portable data\diagnostics; installed %LOCALAPPDATA%\TS-Analyzer\diagnostics.
Logs and crash dumps remain local. Review diagnostic files before sharing them.
Installed updates use the installer in the original location; portable updates keep the portable folder.
User configuration and reports are preserved. Uninstall through Windows Settings.
The package includes private GStreamer, TSDuck, MSVC runtime and XeLaTeX.
Target systems: Windows 10 22H2 and Windows 11 x64, with a compatible graphics driver.
Authenticode signing is not configured in this build. SHA256SUMS records package file integrity.
User guide: docs/usr/outlines.md.
Dependency license texts are under licenses/. Cargo.lock identifies Rust crate versions.
The TeX subset supports reports generated by this application, not arbitrary TeX documents.
'@ | Set-Content -Encoding utf8 -LiteralPath (Join-Path $package 'README.txt')
$files = Get-ChildItem -LiteralPath $package -Recurse -File
$hashes = $files | ForEach-Object {
  $relative = $_.FullName.Substring($package.Length + 1).Replace('\','/')
  ((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()) + '  ' + $relative
}
$hashes | Set-Content -Encoding utf8 -LiteralPath (Join-Path $package 'SHA256SUMS')
if (-not $SkipArchive) {
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = Join-Path $DeployDirectory "TS-Analyzer-windows-v$Version-x86_64-portable.zip"
  if ((Test-Path -LiteralPath $zip) -or (Test-Path -LiteralPath "$zip.sha256")) { throw 'Deployment artifact already exists; do not replace a published version.' }
  New-Item -ItemType Directory -Force -Path $DeployDirectory | Out-Null
  [IO.Compression.ZipFile]::CreateFromDirectory($package, $zip, [IO.Compression.CompressionLevel]::Optimal, $false)
  $digest = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
  "$digest  $([IO.Path]::GetFileName($zip))" | Set-Content -Encoding utf8 -LiteralPath "$zip.sha256"
  Write-Output "Archive: $zip"
}
Write-Output "Package: $package"
