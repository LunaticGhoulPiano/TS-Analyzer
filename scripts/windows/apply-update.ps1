$ErrorActionPreference = 'Stop'
$stage = [IO.Path]::GetFullPath($env:TSAN_UPDATE_STAGE)
$log = Join-Path $stage 'update.log'
$lock = $null
try {
  . (Join-Path $stage 'update-files.ps1')
  $target = [IO.Path]::GetFullPath($env:TSAN_UPDATE_TARGET)
  $mode = $env:TSAN_UPDATE_MODE
  Assert-Deployment $target $mode
  # No forced process termination: wait for the requesting application to shut down.
  $parent = Get-Process -Id ([int]$env:TSAN_UPDATE_PARENT) -ErrorAction SilentlyContinue
  if ($parent -and -not $parent.WaitForExit(60000)) { throw 'TS Analyzer did not exit within 60 seconds. Close it and retry the update.' }
  $lockRoot = if ($mode -eq 'portable') { Join-Path $target 'data' } else { Join-Path $env:LOCALAPPDATA 'TS-Analyzer' }
  Assert-PlainPath $lockRoot
  New-Item -ItemType Directory -Force -Path $lockRoot | Out-Null
  $lock = [IO.File]::Open((Join-Path $lockRoot 'update.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
  # Recheck downloaded bytes after the UI exits; do not trust a previously extracted directory.
  $verified = & (Join-Path $stage 'stage-update.ps1') -VerifyLocalArchive -ExtractName ('apply-' + [guid]::NewGuid().ToString('N'))
  if ($mode -eq 'installed') {
    $arguments = @('/SP-','/SILENT','/SUPPRESSMSGBOXES','/NORESTART','/RESTARTEXITCODE=3010','/NORESTARTAPPLICATIONS',
      ('/DIR="' + $target + '"'), ('/LOG="' + (Join-Path $stage 'installer.log') + '"'))
    $process = Start-Process -FilePath $verified -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -eq 3010) { throw 'Windows must restart before this update can be launched. See installer.log.' }
    if ($process.ExitCode -ne 0) { throw "Installer exited with code $($process.ExitCode); see installer.log" }
  } else {
    Update-PortableFiles $verified $target (Join-Path $stage ('backup-' + [guid]::NewGuid().ToString('N')))
  }
  Assert-Deployment $target $mode
  $manifest = Get-Content -Raw -LiteralPath (Join-Path $target 'package.toml')
  if ($manifest -notmatch ('(?m)^version\s*=\s*"' + [regex]::Escape($env:TSAN_UPDATE_VERSION) + '"\s*$')) { throw 'Installed version did not match the requested release' }
  "Updated $mode package at $target to $($env:TSAN_UPDATE_VERSION)" | Set-Content -LiteralPath $log -Encoding UTF8
  $lock.Dispose()
  $lock = $null
  Start-Process -FilePath (Join-Path $target 'TS-Analyzer.exe') -WindowStyle Hidden
} catch {
  $message = "Update failed: $($_.Exception.Message)"
  $message | Set-Content -LiteralPath $log -Encoding UTF8
  if ($env:TSAN_HEADLESS_TEST -ne '1') {
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.MessageBox]::Show($message + [Environment]::NewLine + "Log: $log", 'TS Analyzer') | Out-Null
  }
  exit 1
} finally {
  if ($lock) { $lock.Dispose() }
}
