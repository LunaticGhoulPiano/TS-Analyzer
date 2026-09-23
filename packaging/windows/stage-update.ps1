param([switch]$VerifyLocalArchive)
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = Join-Path $env:TSAN_UPDATE_STAGE 'package.zip'
if (-not $VerifyLocalArchive) {
  Invoke-WebRequest -UseBasicParsing -Uri $env:TSAN_UPDATE_URL -OutFile $zip -TimeoutSec 180
}
if ((Get-Item -LiteralPath $zip).Length -ne [long]$env:TSAN_UPDATE_BYTES) { throw 'Package size mismatch' }
if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ne $env:TSAN_UPDATE_SHA256) { throw 'Package SHA-256 mismatch' }
$destination = [IO.Path]::GetFullPath((Join-Path $env:TSAN_UPDATE_STAGE 'package'))
$prefix = $destination + [IO.Path]::DirectorySeparatorChar
$archive = [IO.Compression.ZipFile]::OpenRead($zip)
try {
  $names = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
  [long]$expanded = 0
  if ($archive.Entries.Count -gt 20000) { throw 'Too many archive entries' }
  foreach ($entry in $archive.Entries) {
    $name = $entry.FullName.Replace('/', '\')
    $target = [IO.Path]::GetFullPath((Join-Path $destination $name))
    if ([IO.Path]::IsPathRooted($name) -or $name.Contains(':') -or ($name.Split('\') -contains '..') -or
        -not $target.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -or
        -not $names.Add($name) -or (($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000) {
      throw 'Unsafe archive entry'
    }
    $expanded += $entry.Length
    if ($expanded -gt 4GB) { throw 'Package expands beyond 4 GiB' }
  }
} finally { $archive.Dispose() }
[IO.Compression.ZipFile]::ExtractToDirectory($zip, $destination)
$manifest = Get-Content -Raw -LiteralPath (Join-Path $destination 'package.toml')
$expected = 'version = "' + $env:TSAN_UPDATE_VERSION + '"'
if (-not ($manifest.Split([char]10) | Where-Object { $_.Trim() -eq $expected })) { throw 'Package version mismatch' }
if (-not (Test-Path -LiteralPath (Join-Path $destination 'Setup.exe')) -or
    -not (Test-Path -LiteralPath (Join-Path $destination 'app\tsan-gui.exe'))) { throw 'Incomplete package' }
