# Runs without network access; only the GitHub HTTP boundary is replaced.
param([string]$ScriptPath = (Join-Path $PSScriptRoot '../../../scripts/windows/fetch-release.ps1'))
$ErrorActionPreference = 'Stop'
$ScriptPath = (Resolve-Path -LiteralPath $ScriptPath).Path
$fixture = [pscustomobject]@{ Pages = @{}; RequestedPages = @(); HttpStatus = 200 }

function Invoke-RestMethod {
  param([string]$Uri, [hashtable]$Headers, [int]$TimeoutSec)
  if ($Uri -notmatch '^https://api\.github\.com/repos/LunaticGhoulPiano/TS-Analyzer/releases\?per_page=100&page=(\d+)$') {
    throw "Unexpected release endpoint: $Uri"
  }
  $page = [int]$Matches[1]
  $fixture.RequestedPages += $page
  if ($fixture.HttpStatus -ne 200) {
    $failure = New-Object System.Exception "HTTP $($fixture.HttpStatus)"
    $failure | Add-Member NoteProperty Response ([pscustomobject]@{ StatusCode = [pscustomobject]@{ value__ = $fixture.HttpStatus } })
    throw $failure
  }
  # Invoke-RestMethod emits a JSON array as one pipeline object.
  return ,($fixture.Pages[$page])
}
# Keep fixture state bound to this test when the mock is called by another script.
Set-Item Function:Invoke-RestMethod (${function:Invoke-RestMethod}.GetNewClosure())

function New-Release([string]$Tag) {
  $assets = foreach ($suffix in @('portable.zip','setup.exe')) {
    $name = "TS-Analyzer-$Tag-x86_64-$suffix"
    [pscustomobject]@{
      name = $name
      browser_download_url = "https://github.com/LunaticGhoulPiano/TS-Analyzer/releases/download/$Tag/$name"
      digest = 'sha256:' + ('a' * 64)
      size = 123
    }
  }
  return [pscustomobject]@{
    tag_name = $Tag
    html_url = "https://github.com/LunaticGhoulPiano/TS-Analyzer/releases/tag/$Tag"
    draft = $false
    prerelease = $false
    assets = @($assets)
  }
}

function Assert-Response([string]$Mode, [string[]]$Expected) {
  $actual = @(& $ScriptPath -TargetOs windows -TargetArch x86_64 -Mode $Mode)
  if (($actual -join "`n") -cne ($Expected -join "`n")) {
    throw "Unexpected response for ${Mode}: $($actual -join '; ')"
  }
}

$latest = New-Release 'windows-v99.10.0'
$draft = New-Release 'windows-v100.0.0'
$draft.draft = $true
$prerelease = New-Release 'windows-v101.0.0'
$prerelease.prerelease = $true
$fixture.Pages[1] = @((New-Release 'windows-v99.2.0'), $latest, $draft, $prerelease,
  (New-Release 'macos-v200.0.0'), (New-Release 'windows-v102.0.0-beta'))
foreach ($mode in @('portable','installed')) {
  $asset = $latest.assets[[int]($mode -eq 'installed')]
  Assert-Response $mode @($latest.tag_name, $latest.html_url, $asset.name, $asset.browser_download_url, $asset.digest, '123')
}
Write-Output 'PASS numeric version ordering, stable platform filtering, and deployment-specific assets'

# A full first page must not hide a newer release on a later page.
$fixture.Pages[1] = @((New-Release 'windows-v99.2.0')) * 100
$fixture.Pages[2] = @($latest)
$fixture.RequestedPages = @()
$asset = $latest.assets[0]
Assert-Response 'portable' @($latest.tag_name, $latest.html_url, $asset.name, $asset.browser_download_url, $asset.digest, '123')
if (($fixture.RequestedPages -join ',') -ne '1,2') { throw 'Release pagination stopped early' }
Write-Output 'PASS release pagination'

$fixture.Pages = @{ 1 = @($latest) }
foreach ($mode in @('portable','installed')) {
  $singleAsset = $latest.assets[[int]($mode -eq 'installed')]
  Assert-Response $mode @($latest.tag_name, $latest.html_url, $singleAsset.name, $singleAsset.browser_download_url, $singleAsset.digest, '123')
}
Write-Output 'PASS single-release JSON array'

$fixture.Pages[1] = @($draft, $prerelease, (New-Release 'macos-v200.0.0'))
Assert-Response 'portable' @('NO_RELEASE')
Write-Output 'PASS no stable release for the requested platform'

$fixture.Pages[1] = @($latest)
$asset.digest = $null
Assert-Response 'portable' @($latest.tag_name, 'NO_PACKAGE')
$asset.digest = 'sha256:' + ('a' * 64)
$latest.assets = @($asset, $asset)
Assert-Response 'portable' @($latest.tag_name, 'NO_PACKAGE')
$latest.assets = @()
Assert-Response 'installed' @($latest.tag_name, 'NO_PACKAGE')
$fixture.Pages[1] = @()
Assert-Response 'portable' @('NO_RELEASE')
$fixture.HttpStatus = 404
Assert-Response 'portable' @('NO_RELEASE')
Write-Output 'PASS missing/duplicate assets, missing digest, empty releases, and HTTP 404'

$fixture.HttpStatus = 403
$failed = $false
try { Assert-Response 'portable' @('NO_RELEASE') } catch { $failed = $_.Exception.Message -like '*HTTP 403*' }
if (-not $failed) { throw 'HTTP failure was suppressed' }
$fixture.HttpStatus = 200
$fixture.Pages = @{}
foreach ($page in 1..20) { $fixture.Pages[$page] = @((New-Release 'windows-v99.2.0')) * 100 }
$failed = $false
try { Assert-Response 'portable' @('NO_RELEASE') } catch { $failed = $_.Exception.Message -like '*Release history exceeds*' }
if (-not $failed) { throw 'Unbounded release history was accepted' }
Write-Output 'PASS HTTP failure propagation and release history limit'
