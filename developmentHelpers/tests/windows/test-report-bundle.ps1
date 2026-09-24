param(
  [Parameter(Mandatory=$true)][string]$Package,
  [Parameter(Mandatory=$true)][string]$Recordings,
  [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a fresh test directory' }
$files = @(Get-ChildItem -LiteralPath $Recordings -Filter '*.ts' -File)
if ($files.Count -eq 0) { throw 'No .ts recordings found' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$env:TSAN_HEADLESS_TEST = '1'
$results = @()
foreach ($file in $files) {
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
  $texPrefix = [IO.Path]::GetFullPath((Join-Path $Package 'runtime/tex')).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
  $reportPrefix = [IO.Path]::GetFullPath($directory).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
  foreach ($line in Get-Content -LiteralPath (Join-Path $directory 'report_sections/build/report.fls')) {
    if (-not $line.StartsWith('INPUT ')) { continue }
    $inputPath = $line.Substring(6).Trim('"')
    if ([IO.Path]::GetExtension($inputPath) -notin @('.tex','.sty','.cls','.fmt','.def','.cfg','.clo','.ldf')) { continue }
    $absolute = if ([IO.Path]::IsPathRooted($inputPath)) { [IO.Path]::GetFullPath($inputPath) }
                else { [IO.Path]::GetFullPath([IO.Path]::Combine($directory, $inputPath)) }
    if (-not $absolute.StartsWith($texPrefix, [StringComparison]::OrdinalIgnoreCase) -and
        -not $absolute.StartsWith($reportPrefix, [StringComparison]::OrdinalIgnoreCase)) {
      throw "PDF used TeX sources outside its package/report directories: $absolute"
    }
  }
  $results += [pscustomobject]@{ File=$file.Name; Result=$message; AnalysisAndFourExportsSeconds=[Math]::Round($watch.Elapsed.TotalSeconds,2) }
  Write-Output "PASS $($file.Name) CBOR/XLSX/TeX/PDF"
}
$results | ConvertTo-Json | Set-Content -Encoding utf8 -LiteralPath (Join-Path $OutputDirectory 'results.json')
