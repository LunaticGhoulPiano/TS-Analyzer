param([switch]$VerifyLocalArchive, [string]$ExtractName = 'package')
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$mode = $env:TSAN_UPDATE_MODE
if ($mode -notin @('installed','portable')) { throw 'Unknown update mode' }
$name = if ($mode -eq 'installed') { 'installer.exe' } else { 'package.zip' }
$artifact = Join-Path $env:TSAN_UPDATE_STAGE $name
if (-not $VerifyLocalArchive) {
  Invoke-WebRequest -UseBasicParsing -Uri $env:TSAN_UPDATE_URL -OutFile $artifact -TimeoutSec 180
}
if ((Get-Item -LiteralPath $artifact).Length -ne [long]$env:TSAN_UPDATE_BYTES) { throw 'Package size mismatch' }
if ((Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash -ne $env:TSAN_UPDATE_SHA256) { throw 'Package SHA-256 mismatch' }
if ($mode -eq 'installed') {
  $stream = [IO.File]::OpenRead($artifact)
  try { if ($stream.ReadByte() -ne 0x4D -or $stream.ReadByte() -ne 0x5A) { throw 'Invalid Windows installer' } }
  finally { $stream.Dispose() }
  Write-Output $artifact
  return
}
if ($ExtractName -notmatch '^[a-z0-9-]+$') { throw 'Invalid extraction directory' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$destination = [IO.Path]::GetFullPath((Join-Path $env:TSAN_UPDATE_STAGE $ExtractName))
if (Test-Path -LiteralPath $destination) { throw 'Extraction directory already exists' }
$prefix = $destination + [IO.Path]::DirectorySeparatorChar
$archive = [IO.Compression.ZipFile]::OpenRead($artifact)
try {
  $names = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
  [long]$expanded = 0
  if ($archive.Entries.Count -gt 20000) { throw 'Too many archive entries' }
  foreach ($entry in $archive.Entries) {
    $name = $entry.FullName.Replace('/', '\')
    $target = [IO.Path]::GetFullPath((Join-Path $destination $name))
    if ([IO.Path]::IsPathRooted($name) -or $name -match '[:*?"<>|]' -or
        $name -match '(^|\\)(\.{1,2}|[^\\]*[. ])(\\|$)' -or
        $name -match '(?i)(^|\\)(con|prn|aux|nul|com[1-9]|lpt[1-9])(\.|\\|$)' -or
        -not $target.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -or
        -not $names.Add($name) -or (($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000 -or
        (($entry.ExternalAttributes -band 0x400) -ne 0) -or $name -match '(?i)^data(\\|$)') {
      throw 'Unsafe archive entry'
    }
    $expanded += $entry.Length
    if ($expanded -gt 4GB) { throw 'Package expands beyond 4 GiB' }
  }
} finally { $archive.Dispose() }
[IO.Compression.ZipFile]::ExtractToDirectory($artifact, $destination)
$manifest = Get-Content -Raw -LiteralPath (Join-Path $destination 'package.toml')
$expected = 'version = "' + $env:TSAN_UPDATE_VERSION + '"'
if (-not ($manifest.Split([char]10) | Where-Object { $_.Trim() -eq $expected }) -or
    $manifest -notmatch '(?m)^target\s*=\s*"windows-x86_64"\s*$') { throw 'Package version or target mismatch' }
$deployment = Get-Content -Raw -LiteralPath (Join-Path $destination 'deployment.toml')
if ($deployment -notmatch '(?m)^schema\s*=\s*1\s*$' -or
    $deployment -notmatch '(?m)^mode\s*=\s*"portable"\s*$' -or
    $deployment -notmatch '(?m)^app_id\s*=\s*"TSAnalyzer"\s*$') { throw 'Package deployment mismatch' }
foreach ($required in @('TS-Analyzer.exe','app/tsan-gui.exe','SHA256SUMS')) {
  if (-not (Test-Path -LiteralPath (Join-Path $destination $required) -PathType Leaf)) { throw 'Incomplete package' }
}
Write-Output $destination
