param(
  [Parameter(Mandatory=$true)][string]$Package,
  [Parameter(Mandatory=$true)][string]$Recordings,
  [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a fresh test directory' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$env:TSAN_HEADLESS_TEST = '1'
$results = @()
foreach ($file in Get-ChildItem -LiteralPath $Recordings -Filter '*.ts' -File) {
  $directory = Join-Path $OutputDirectory $file.BaseName
  $env:TSAN_ERROR_FILE = Join-Path $OutputDirectory ($file.BaseName + '-launcher-error.txt')
  $watch = [Diagnostics.Stopwatch]::StartNew()
  $run = Start-Process -FilePath (Join-Path $Package 'TS-Analyzer.exe') -WindowStyle Hidden -Wait -PassThru -ArgumentList @(
    '--wait','--package-smoke',('"' + $file.FullName + '"'),('"' + $directory + '"'))
  $watch.Stop()
  if ($run.ExitCode -ne 0) { throw "Report smoke failed: $directory" }
  $message = Get-Content -Raw -LiteralPath (Join-Path $directory 'smoke-result.txt')
  if (-not $message.StartsWith('PASS')) { throw $message }
  foreach ($extension in @('cbor','xlsx','tex','pdf')) {
    if ((Get-Item -LiteralPath (Join-Path $directory "report.$extension")).Length -eq 0) { throw "Empty report: $extension" }
  }
  $recorder = Get-Content -Raw -LiteralPath (Join-Path $directory 'report_sections/build/report.fls')
  if ($recorder -match '(?im)^INPUT ["]?[cC]:[/\\](texlive|Program Files)') { throw 'PDF used developer installation files' }
  $results += [pscustomobject]@{ File=$file.Name; Result=$message; AnalysisAndFourExportsSeconds=[Math]::Round($watch.Elapsed.TotalSeconds,2) }
  Write-Output "PASS $($file.Name) CBOR/XLSX/TeX/PDF"
}
$results | ConvertTo-Json | Set-Content -Encoding utf8 -LiteralPath (Join-Path $OutputDirectory 'results.json')
