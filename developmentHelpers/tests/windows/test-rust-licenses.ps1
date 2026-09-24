#requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
. (Join-Path $repository 'developmentHelpers/packaging/windows/rust-licenses.ps1')
$out = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $out | Out-Null
$metadataText = & cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $repository 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed during license verification.' }
$metadata = $metadataText | ConvertFrom-Json
$inventory = Copy-RustLicenses -Metadata $metadata -Repository $repository -Destination (Join-Path $out 'licenses')
$expectedCount = @($metadata.packages | Where-Object { $_.source }).Count
if ($inventory.Count -ne $expectedCount) { throw 'Rust license inventory omits a locked dependency.' }
$font = @($inventory | Where-Object { $_.name -eq 'epaint_default_fonts' })
if ($font.Count -ne 1) { throw 'Expected one embedded font package.' }
foreach ($name in @('OFL.txt','UFL.txt','Hack-Regular.txt','emoji-icon-font-mit-license.txt')) {
  if (-not @($font[0].files | Where-Object { $_.path.EndsWith("/fonts/$name") }).Count) {
    throw "Missing embedded font notice: $name"
  }
}
Write-Output ("PASS {0} locked Rust dependencies, {1} license/notice files, and all four embedded font notices" -f $inventory.Count, @($inventory | ForEach-Object { $_.files }).Count)

function Test-LicenseCase([string]$Name, [string]$ExpectedError) {
  $caseRoot = Join-Path $out $Name
  $cache = Join-Path $caseRoot 'crate'
  $supplement = Join-Path $caseRoot 'licenses/rust/fixture-1.0.0'
  New-Item -ItemType Directory -Force -Path $cache, $supplement | Out-Null
  $source = 'registry+https://github.com/rust-lang/crates.io-index'
  $dependency = [pscustomobject]@{ name='fixture'; version='1.0.0'; source=$source; license='MIT'; manifest_path=(Join-Path $cache 'Cargo.toml'); license_file=$null }
  $entry = @{ name='fixture'; version='1.0.0'; cargo_source=$source; license='MIT'; files=@() }
  if ($Name -eq 'nested') {
    New-Item -ItemType Directory -Path (Join-Path $cache 'assets') | Out-Null
    'Synthetic license fixture' | Set-Content -LiteralPath (Join-Path $cache 'assets/LICENSE.txt')
    'Synthetic notice fixture' | Set-Content -LiteralPath (Join-Path $cache 'assets/NOTICE.txt')
  } elseif ($Name -ne 'missing') {
    $license = Join-Path $supplement 'LICENSE'
    'Synthetic license fixture' | Set-Content -LiteralPath $license
    $hash = (Get-FileHash -LiteralPath $license -Algorithm SHA256).Hash.ToLowerInvariant()
    $file = @{ path='LICENSE'; sha256=$hash; source='https://example.invalid/fixture/LICENSE' }
    $entry.files = @($file)
    switch ($Name) {
      'missing-file' { $file.path = 'LICENSE-absent' }
      'tampered' { 'Changed fixture' | Set-Content -LiteralPath $license }
      'version' { $dependency.version = '1.0.1' }
      'source' { $entry.cargo_source = 'git+https://example.invalid/fixture' }
      'traversal' { $file.path = '../LICENSE' }
      'empty' { [IO.File]::WriteAllText($license, '') }
    }
  }
  @{ schema=1; packages=@($entry) } | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath (Join-Path $caseRoot 'licenses/rust/sources.json') -Encoding utf8
  if ($Name -in 'nested','missing') {
    @{ schema=1; packages=@() } | ConvertTo-Json |
      Set-Content -LiteralPath (Join-Path $caseRoot 'licenses/rust/sources.json') -Encoding utf8
  }
  try {
    $result = Copy-RustLicenses -Metadata @{ packages=@($dependency) } -Repository $caseRoot -Destination (Join-Path $caseRoot 'result')
  } catch {
    if (-not $ExpectedError -or $_.Exception.Message -notlike "*$ExpectedError*") { throw }
    Write-Output "PASS rejects $Name"
    return
  }
  if ($ExpectedError) { throw "Expected license rejection: $Name" }
  if ($result.Count -ne 1 -or $result[0].files.Count -ne 2) { throw 'Nested license/notice files were not both copied.' }
  Write-Output 'PASS nested license and notice discovery'
}
Test-LicenseCase 'nested' ''
Test-LicenseCase 'missing' 'No license text collected'
Test-LicenseCase 'missing-file' 'Missing or empty license text'
Test-LicenseCase 'tampered' 'checksum mismatch'
Test-LicenseCase 'version' 'No license text collected'
Test-LicenseCase 'source' 'differs from Cargo'
Test-LicenseCase 'traversal' 'Unsafe license path'
Test-LicenseCase 'empty' 'Missing or empty license text'
