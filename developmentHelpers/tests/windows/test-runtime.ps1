param(
  [Parameter(Mandatory=$true)][string]$Package,
  [Parameter(Mandatory=$true)][string]$Recordings,
  [Parameter(Mandatory=$true)][string]$GStreamerTools,
  [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a fresh test directory' }
$files = @(Get-ChildItem -LiteralPath $Recordings -Filter '*.ts' -File)
if ($files.Count -eq 0) { throw 'No .ts recordings found' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
# Copy only the diagnostic executable so Windows cannot resolve DLLs beside the developer tool.
$tool = Join-Path $OutputDirectory 'gst-launch-1.0.exe'
Copy-Item -LiteralPath (Join-Path $GStreamerTools 'gst-launch-1.0.exe') -Destination $tool

function Invoke-PackagedPipeline([IO.FileInfo]$File, [string]$Name, [string[]]$Branch) {
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
  $processInfo.Environment['GST_REGISTRY_1_0'] = $processInfo.Environment['GST_REGISTRY']
  $arguments = @('-v','filesrc',('location="' + $File.FullName.Replace('\','/') + '"'),'!','tsdemux','name=demux','demux.','!','queue','!') + $Branch
  foreach ($argument in $arguments) { $processInfo.ArgumentList.Add($argument) }
  $process = [Diagnostics.Process]::Start($processInfo)
  try {
    $output = $process.StandardOutput.ReadToEndAsync()
    $errorText = $process.StandardError.ReadToEndAsync()
    $exited = $process.WaitForExit(60000)
    if (-not $exited) { $process.Kill(); $process.WaitForExit() }
    $stdout = $output.GetAwaiter().GetResult()
    $stderr = $errorText.GetAwaiter().GetResult()
    $log = Join-Path $OutputDirectory ($File.BaseName + '-' + $Name + '.log')
    [IO.File]::WriteAllText($log, $stdout + $stderr)
    if (-not $exited) { throw "Pipeline timeout: $log" }
    if ($process.ExitCode -ne 0 -or $stdout -notmatch 'chain.*bytes' -or $stdout -notmatch 'Got EOS') {
      throw "Pipeline validation failed: $log"
    }
    return $stdout
  } finally {
    $process.Dispose()
  }
}

$results = @()
foreach ($file in $files) {
  # Negotiate video caps from the stream; filenames carry no codec information.
  $probe = Invoke-PackagedPipeline $file 'probe' @('video/x-h264;video/x-h265','!','identity','name=video-probe','eos-after=2','silent=false','!','fakesink','sync=false')
  $codec = [regex]::Match($probe, 'GstIdentity:video-probe\.GstPad:sink: caps = video/x-(h264|h265)').Groups[1].Value
  if (-not $codec) { throw "No supported H.264/H.265 video caps in $($file.Name); see probe log" }
  foreach ($backend in @('d3d11','d3d12')) {
    $null = Invoke-PackagedPipeline $file $backend @(($codec + 'parse'),'!',($backend + $codec + 'dec'),'!','identity','eos-after=30','silent=false','!','fakesink','sync=false')
    $results += [pscustomobject]@{ File=$file.Name; Codec=$codec; Backend=$backend; Result='PASS decoded frames and EOS' }
    Write-Output "PASS $($file.Name) $codec $backend"
  }
}
$results | ConvertTo-Json | Set-Content -Encoding utf8 -LiteralPath (Join-Path $OutputDirectory 'results.json')
