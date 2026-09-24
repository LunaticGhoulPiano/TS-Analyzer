function New-RestartFixture([string]$Directory) {
  $source = Join-Path $Directory 'restart-fixture.rs'
  $exe = Join-Path $Directory 'restart-fixture.exe'
  @'
#![windows_subsystem = "windows"]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let root = executable.parent().ok_or("missing fixture root")?;
    std::fs::write(root.join("restarted.txt"), "restarted")?;
    Ok(())
}
'@ | Set-Content -LiteralPath $source -Encoding utf8
  & rustc --edition=2024 -C target-feature=+crt-static $source -o $exe
  if ($LASTEXITCODE -ne 0) { throw 'Restart fixture could not be compiled' }
  return $exe
}

function Invoke-UpdateWorkerTest([string]$Target, [string]$Stage, [string]$Mode, [string]$Version) {
  $productScripts = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../scripts/windows'))
  foreach ($name in @('stage-update.ps1','update-files.ps1')) {
    Copy-Item -LiteralPath (Join-Path $productScripts $name) -Destination (Join-Path $Stage $name)
  }
  $artifact = Join-Path $Stage $(if ($Mode -eq 'installed') { 'installer.exe' } else { 'package.zip' })
  $variables = @{
    TSAN_UPDATE_MODE = $Mode; TSAN_UPDATE_TARGET = $Target; TSAN_UPDATE_STAGE = $Stage
    TSAN_UPDATE_VERSION = $Version
    TSAN_UPDATE_BYTES = (Get-Item -LiteralPath $artifact).Length.ToString()
    TSAN_UPDATE_SHA256 = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash
    TSAN_HEADLESS_TEST = '1'
    PSModulePath = (Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/Modules')
  }
  $saved = @{}
  $shell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
  $parent = Start-Process -FilePath $shell -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 2') -WindowStyle Hidden -PassThru
  $variables.TSAN_UPDATE_PARENT = $parent.Id.ToString()
  try {
    foreach ($key in $variables.Keys) {
      $saved[$key] = [Environment]::GetEnvironmentVariable($key)
      [Environment]::SetEnvironmentVariable($key, $variables[$key], 'Process')
    }
    $arguments = @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',('"' + (Join-Path $productScripts 'apply-update.ps1') + '"'))
    $worker = Start-Process -FilePath $shell -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
    if ($worker.ExitCode -ne 0) {
      throw "Update worker failed: $(Get-Content -Raw -LiteralPath (Join-Path $Stage 'update.log'))"
    }
    for ($attempt = 0; $attempt -lt 30 -and -not (Test-Path -LiteralPath (Join-Path $Target 'restarted.txt')); $attempt++) {
      Start-Sleep -Milliseconds 100
    }
    if (-not (Test-Path -LiteralPath (Join-Path $Target 'restarted.txt'))) { throw 'Updated launcher was not restarted' }
  } finally {
    foreach ($key in $saved.Keys) { [Environment]::SetEnvironmentVariable($key, $saved[$key], 'Process') }
  }
  Write-Output "PASS $Mode background worker verifies, updates original location and restarts"
}
