param(
  [Parameter(Mandatory=$true)][ValidateSet('windows','macos','linux')][string]$TargetOs,
  [Parameter(Mandatory=$true)][ValidatePattern('^[a-zA-Z0-9_]+$')][string]$TargetArch,
  [Parameter(Mandatory=$true)][ValidateSet('installed','portable')][string]$Mode
)
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$candidates = @()
$pattern = '^' + [regex]::Escape($TargetOs) + '-v(\d+\.\d+\.\d+)$'
for ($page = 1; $page -le 20; $page++) {
  try {
    # Assign the JSON array first so each release is filtered and counted separately.
    $response = Invoke-RestMethod -Uri ("https://api.github.com/repos/LunaticGhoulPiano/TS-Analyzer/releases?per_page=100&page=$page") -Headers @{'User-Agent'='TS-Analyzer';'Accept'='application/vnd.github+json'} -TimeoutSec 20
    $items = @($response)
  } catch {
    if ($_.Exception.Response.StatusCode.value__ -eq 404) { 'NO_RELEASE'; return }
    throw
  }
  $candidates += @($items | Where-Object { -not $_.draft -and -not $_.prerelease -and $_.tag_name -match $pattern })
  if ($items.Count -lt 100) { break }
  if ($page -eq 20) { throw 'Release history exceeds the check limit; cannot establish the newest platform version.' }
}
$r = $candidates | Sort-Object { [version]($_.tag_name -replace $pattern,'$1') } -Descending | Select-Object -First 1
if (-not $r) { 'NO_RELEASE'; return }
$r.tag_name
$suffix = if ($Mode -eq 'installed') { '-setup.exe' } else { '-portable.zip' }
$name = 'TS-Analyzer-' + $r.tag_name + '-' + $TargetArch + $suffix
$a = @($r.assets | Where-Object { $_.name -eq $name })
if ($a.Count -ne 1 -or -not $a[0].digest) { 'NO_PACKAGE'; return }
$r.html_url
$a[0].name
$a[0].browser_download_url
$a[0].digest
$a[0].size
