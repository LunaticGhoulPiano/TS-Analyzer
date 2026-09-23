param([Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.IO.Compression
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a fresh test output directory' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$script = Join-Path $PSScriptRoot 'stage-update.ps1'
$cases = @('valid','traversal','duplicate','symlink','size','hash','version','missing-app')
foreach ($case in $cases) {
  $stage = Join-Path $OutputDirectory $case
  New-Item -ItemType Directory -Path $stage | Out-Null
  $zip = Join-Path $stage 'package.zip'
  $archive = [IO.Compression.ZipFile]::Open($zip, [IO.Compression.ZipArchiveMode]::Create)
  try {
    $files = [Collections.Generic.Dictionary[string,string]]::new([StringComparer]::Ordinal)
    $files['package.toml'] = 'version = "0.2.0"'
    $files['Setup.exe'] = 'fixture only'
    $files['app/tsan-gui.exe'] = 'fixture only'
    if ($case -eq 'traversal') { $files['../outside.txt'] = 'must not extract' }
    if ($case -eq 'duplicate') { $files['APP/TSAN-GUI.EXE'] = 'must not extract' }
    if ($case -eq 'symlink') { $files['link'] = 'outside' }
    if ($case -eq 'missing-app') { $null = $files.Remove('app/tsan-gui.exe') }
    foreach ($item in $files.GetEnumerator()) {
      $entry = $archive.CreateEntry($item.Key)
      if ($case -eq 'symlink' -and $item.Key -eq 'link') { $entry.ExternalAttributes = [int](-1610612736) }
      $writer = New-Object IO.StreamWriter($entry.Open())
      try { $writer.Write($item.Value) } finally { $writer.Dispose() }
    }
  } finally { $archive.Dispose() }
  $env:TSAN_UPDATE_STAGE = $stage
  $env:TSAN_UPDATE_VERSION = if ($case -eq 'version') { '9.9.9' } else { '0.2.0' }
  $env:TSAN_UPDATE_SHA256 = if ($case -eq 'hash') { '0' * 64 } else { (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash }
  $env:TSAN_UPDATE_BYTES = ((Get-Item -LiteralPath $zip).Length + $(if ($case -eq 'size') { 1 } else { 0 })).ToString()
  $failed = $false
  try { & $script -VerifyLocalArchive } catch { $failed = $true; $reason = $_.Exception.Message }
  if ($case -eq 'valid' -and $failed) { throw "Valid fixture rejected: $reason" }
  if ($case -ne 'valid' -and -not $failed) { throw "Unsafe fixture accepted: $case" }
  if (Test-Path -LiteralPath (Join-Path $OutputDirectory 'outside.txt')) { throw 'Archive traversal escaped staging' }
  Write-Output "PASS $case"
}
