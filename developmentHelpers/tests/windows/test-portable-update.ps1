param([Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../../scripts/windows/update-files.ps1')
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a fresh test output directory' }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

function Write-Fixture([string]$Root, [string]$Version) {
  New-Item -ItemType Directory -Path (Join-Path $Root 'app'),(Join-Path $Root 'runtime') -Force | Out-Null
  foreach ($relative in @('TS-Analyzer.exe','app/tsan-gui.exe','runtime/z-locked.dll')) {
    "fixture $Version" | Set-Content -LiteralPath (Join-Path $Root $relative)
  }
  ('version = "' + $Version + '"'), 'target = "windows-x86_64"' | Set-Content -LiteralPath (Join-Path $Root 'package.toml')
  'schema = 1','app_id = "TSAnalyzer"','mode = "portable"' | Set-Content -LiteralPath (Join-Path $Root 'deployment.toml')
  $extra = if ($Version -eq '0.2.1') { 'runtime/obsolete.dll' } else { 'runtime/added.dll' }
  'managed runtime' | Set-Content -LiteralPath (Join-Path $Root $extra)
  if ($Version -ne '0.2.1') {
    New-Item -ItemType Directory -Path (Join-Path $Root 'scripts/windows') -Force | Out-Null
    "release check $Version" | Set-Content -LiteralPath (Join-Path $Root 'scripts/windows/fetch-release.ps1')
  }
  $hashes = Get-ChildItem -LiteralPath $Root -Recurse -File | ForEach-Object {
    (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash + '  ' + $_.FullName.Substring($Root.Length + 1).Replace('\','/')
  }
  $hashes | Set-Content -LiteralPath (Join-Path $Root 'SHA256SUMS') -Encoding UTF8
}

. (Join-Path $PSScriptRoot 'update-worker-test.ps1')
$restartFixture = New-RestartFixture $OutputDirectory

foreach ($case in @('success','locked-file','user-collision','tampered-source','traversal','script-traversal','installed-target','junction')) {
  $directory = Join-Path $OutputDirectory $case
  $target = Join-Path $directory 'portable folder'
  $source = Join-Path $directory 'new package'
  $backup = Join-Path $directory 'backup'
  Write-Fixture $target '0.2.1'
  Write-Fixture $source '0.2.2'
  New-Item -ItemType Directory -Path (Join-Path $target 'data'),(Join-Path $target 'reports') | Out-Null
  'theme = "Dark"' | Set-Content -LiteralPath (Join-Path $target 'data/tsan-config.toml')
  'user report' | Set-Content -LiteralPath (Join-Path $target 'reports/result.pdf')
  $handle = $null
  if ($case -eq 'locked-file') {
    $handle = [IO.File]::Open((Join-Path $target 'runtime/z-locked.dll'), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
  }
  if ($case -eq 'user-collision') { 'user owned' | Set-Content -LiteralPath (Join-Path $target 'runtime/added.dll') }
  if ($case -eq 'tampered-source') { 'tampered' | Set-Content -LiteralPath (Join-Path $source 'app/tsan-gui.exe') }
  if ($case -eq 'traversal') { ('a' * 64 + '  ../outside.txt') | Add-Content -LiteralPath (Join-Path $target 'SHA256SUMS') }
  if ($case -eq 'script-traversal') { ('a' * 64 + '  scripts/../data/tsan-config.toml') | Add-Content -LiteralPath (Join-Path $target 'SHA256SUMS') }
  if ($case -eq 'installed-target') {
    'schema = 1','app_id = "TSAnalyzer"','mode = "installed"' | Set-Content -LiteralPath (Join-Path $target 'deployment.toml')
  }
  if ($case -eq 'junction') {
    $alias = Join-Path $directory 'linked-target'
    New-Item -ItemType Junction -Path $alias -Target $target | Out-Null
    $target = $alias
  }
  $before = (Get-FileHash -LiteralPath (Join-Path $target 'app/tsan-gui.exe')).Hash
  $failed = $false
  try { Update-PortableFiles $source $target $backup } catch { $failed = $true; $reason = $_.Exception.Message }
  finally { if ($handle) { $handle.Dispose() } }
  if ($case -eq 'success') {
    if ($failed) { throw "Successful update rejected: $reason" }
    $null = Read-ManagedInventory $target -VerifyHashes
    if ((Get-Content -Raw -LiteralPath (Join-Path $target 'scripts/windows/fetch-release.ps1')).Trim() -ne 'release check 0.2.2') { throw 'Shared update script was not installed' }
    if (Test-Path -LiteralPath (Join-Path $target 'runtime/obsolete.dll')) { throw 'Obsolete managed file was not removed' }
    if (-not (Test-Path -LiteralPath (Join-Path $backup 'app/tsan-gui.exe'))) { throw 'Backup was not preserved' }
  } else {
    if (-not $failed) { throw "Invalid update accepted: $case" }
    if ((Get-FileHash -LiteralPath (Join-Path $target 'app/tsan-gui.exe')).Hash -ne $before) { throw 'Failed update did not restore the previous executable' }
    if ($case -in @('locked-file','user-collision')) { $null = Read-ManagedInventory $target -VerifyHashes }
  }
  if ((Get-Content -Raw -LiteralPath (Join-Path $target 'data/tsan-config.toml')).Trim() -ne 'theme = "Dark"' -or
      (Get-Content -Raw -LiteralPath (Join-Path $target 'reports/result.pdf')).Trim() -ne 'user report') { throw 'User data changed' }
  Write-Output "PASS portable $case"
}

$workerRoot = Join-Path $OutputDirectory 'background-worker'
$target = Join-Path $workerRoot 'portable folder'
$source = Join-Path $workerRoot 'new package'
Write-Fixture $target '0.2.1'
Write-Fixture $source '0.2.2'
Copy-Item -LiteralPath $restartFixture -Destination (Join-Path $source 'TS-Analyzer.exe')
$hashes = Get-ChildItem -LiteralPath $source -Recurse -File | Where-Object Name -ne 'SHA256SUMS' | ForEach-Object {
  (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash + '  ' + $_.FullName.Substring($source.Length + 1).Replace('\','/')
}
$hashes | Set-Content -LiteralPath (Join-Path $source 'SHA256SUMS') -Encoding UTF8
$stage = Join-Path $target 'data/updates/test'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($source, (Join-Path $stage 'package.zip'))
Invoke-UpdateWorkerTest $target $stage 'portable' '0.2.2'
$null = Read-ManagedInventory $target -VerifyHashes
