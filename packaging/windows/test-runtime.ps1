param(
  [Parameter(Mandatory=$true)][string]$Package,
  [Parameter(Mandatory=$true)][string]$Recordings,
  [Parameter(Mandatory=$true)][string]$GStreamerTools,
  [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a fresh test directory' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
# Copy only the diagnostic executable so Windows cannot resolve DLLs beside the developer tool.
$tool = Join-Path $OutputDirectory 'gst-launch-1.0.exe'
Copy-Item -LiteralPath (Join-Path $GStreamerTools 'gst-launch-1.0.exe') -Destination $tool
$results = @()
foreach ($file in Get-ChildItem -LiteralPath $Recordings -Filter '*.ts' -File) {
  $codec = if ($file.Name -match '^(M25|V1)') { 'h264' } else { 'h265' }
  foreach ($backend in @('d3d11','d3d12')) {
    $processInfo = [Diagnostics.ProcessStartInfo]::new($tool)
    $processInfo.UseShellExecute = $false
    $processInfo.CreateNoWindow = $true
    $processInfo.RedirectStandardOutput = $true
    $processInfo.RedirectStandardError = $true
    $processInfo.Environment['PATH'] = (Join-Path $Package 'runtime/gstreamer/bin') + ';' + (Join-Path $Package 'runtime/msvc') + ';' + (Join-Path $env:SystemRoot 'System32')
    $processInfo.Environment['GST_PLUGIN_PATH'] = ''
    $processInfo.Environment['GST_PLUGIN_PATH_1_0'] = ''
    $processInfo.Environment['GST_PLUGIN_SYSTEM_PATH'] = Join-Path $Package 'runtime/gstreamer/lib/gstreamer-1.0'
    $processInfo.Environment['GST_PLUGIN_SYSTEM_PATH_1_0'] = $processInfo.Environment['GST_PLUGIN_SYSTEM_PATH']
    $processInfo.Environment['GST_PLUGIN_SCANNER'] = Join-Path $Package 'runtime/gstreamer/libexec/gstreamer-1.0/gst-plugin-scanner.exe'
    $processInfo.Environment['GST_PLUGIN_SCANNER_1_0'] = $processInfo.Environment['GST_PLUGIN_SCANNER']
    $processInfo.Environment['GST_REGISTRY'] = Join-Path $OutputDirectory 'registry.bin'
    foreach ($argument in @('-v','filesrc',('location="' + $file.FullName.Replace('\','/') + '"'),'!','tsdemux','name=demux',
      'demux.','!','queue','!',($codec + 'parse'),'!',($backend + $codec + 'dec'),'!',
      'identity','eos-after=30','silent=false','!','fakesink','sync=false')) { $processInfo.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::Start($processInfo)
    $output = $process.StandardOutput.ReadToEndAsync()
    $errorText = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(60000)) { $process.Kill(); throw "Decoder timeout: $($file.Name) $backend" }
    $stdout = $output.GetAwaiter().GetResult()
    $stderr = $errorText.GetAwaiter().GetResult()
    $log = Join-Path $OutputDirectory ($file.BaseName + '-' + $backend + '.log')
    [IO.File]::WriteAllText($log, $stdout + $stderr)
    if ($process.ExitCode -ne 0 -or $stdout -notmatch 'chain.*bytes' -or $stdout -notmatch 'Got EOS') { throw "Decoder validation failed: $log" }
    $results += [pscustomobject]@{ File=$file.Name; Backend=$backend; Result='PASS decoded frames and EOS' }
    Write-Output "PASS $($file.Name) $backend"
  }
}
$results | ConvertTo-Json | Set-Content -Encoding utf8 -LiteralPath (Join-Path $OutputDirectory 'results.json')
