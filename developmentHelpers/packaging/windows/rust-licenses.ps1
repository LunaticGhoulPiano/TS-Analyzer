#requires -Version 7.0
# Copy locked Rust dependency notices plus reviewed supplements; no network access.
function Get-LicenseChildPath([string]$Root, [string]$Relative) {
  if ([IO.Path]::IsPathRooted($Relative) -or $Relative -match '(^|[\\/])\.\.([\\/]|$)|:') {
    throw "Unsafe license path: $Relative"
  }
  $base = [IO.Path]::GetFullPath($Root).TrimEnd('\','/')
  $full = [IO.Path]::GetFullPath((Join-Path $base $Relative))
  if (-not $full.StartsWith($base + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "License path escapes its root: $Relative"
  }
  return $full
}

function Copy-RustLicenses {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory=$true)]$Metadata,
    [Parameter(Mandatory=$true)][string]$Repository,
    [Parameter(Mandatory=$true)][string]$Destination
  )
  $supplementRoot = Join-Path $Repository 'licenses/rust'
  $catalogPath = Join-Path $supplementRoot 'sources.json'
  $catalog = Get-Content -LiteralPath $catalogPath -Raw | ConvertFrom-Json
  if ($catalog.schema -ne 1) { throw 'Unsupported Rust license supplement schema.' }
  $supplements = @{}
  foreach ($entry in $catalog.packages) {
    $key = "$($entry.name)-$($entry.version)"
    if ($supplements.ContainsKey($key)) { throw "Duplicate license supplement: $key" }
    $supplements[$key] = $entry
  }
  $inventory = @()
  $usedSupplements = @()
  foreach ($dependency in @($Metadata.packages | Where-Object { $_.source } | Sort-Object name,version)) {
    $key = "$($dependency.name)-$($dependency.version)"
    if ($dependency.name -notmatch '^[a-zA-Z0-9_-]+$' -or $dependency.version -notmatch '^[a-zA-Z0-9.+-]+$') {
      throw "Invalid Cargo package identity: $key"
    }
    $directory = Split-Path -Parent $dependency.manifest_path
    $files = @{}
    # Include nested notices and whole license directories, not just crate-root names.
    foreach ($file in Get-ChildItem -LiteralPath $directory -Recurse -File) {
      $relative = [IO.Path]::GetRelativePath($directory, $file.FullName).Replace('\','/')
      if ($relative -match '(^|/)(\.git|target)/') { continue }
      $notice = $file.Name -match '(?i)licen[sc]e|^(COPYING|NOTICE|COPYRIGHT|UNLICENSE|AUTHORS|OFL|UFL)([._-]|$)' -or
                $relative -match '(?i)(^|/)(licenses?|notices?)/'
      if (-not $notice -or $file.Extension -in '.rs','.c','.h','.cpp','.ttf','.otf') { continue }
      if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Linked license file is unsupported: $key/$relative" }
      $files[$relative] = @{ Source = $file.FullName; Origin = 'cargo-package'; ExpectedHash = $null }
    }
    if ($dependency.license_file) {
      $declared = $dependency.license_file
      if ([IO.Path]::IsPathRooted($declared)) { $declared = [IO.Path]::GetRelativePath($directory, $declared) }
      $source = Get-LicenseChildPath $directory $declared
      $files[$declared.Replace('\','/')] = @{ Source = $source; Origin = 'cargo-license-file'; ExpectedHash = $null }
    }
    if ($supplements.ContainsKey($key)) {
      $supplement = $supplements[$key]
      if ($supplement.cargo_source -ne $dependency.source -or $supplement.license -ne $dependency.license) {
        throw "License supplement source or SPDX metadata differs from Cargo: $key"
      }
      $usedSupplements += $supplement
      foreach ($notice in $supplement.files) {
        if ($notice.sha256 -cnotmatch '^[a-f0-9]{64}$' -or $notice.source -notmatch '^https://') {
          throw "Invalid license provenance: $key/$($notice.path)"
        }
        $source = Get-LicenseChildPath (Join-Path $supplementRoot $key) $notice.path
        $files[$notice.path] = @{ Source = $source; Origin = $notice.source; ExpectedHash = $notice.sha256 }
      }
    }
    $hasLicense = @($files.Keys | Where-Object {
      [IO.Path]::GetFileName($_) -match '(?i)^(LICEN[SC]ES?|COPYING|UNLICENSE|OFL|UFL)([._-]|$)'
    }).Count -gt 0
    if (-not $hasLicense -and -not $dependency.license_file) {
      throw "No license text collected for $key. Add exact-version upstream notices under licenses/rust and record them in sources.json."
    }
    $copied = @()
    foreach ($relative in @($files.Keys | Sort-Object)) {
      $notice = $files[$relative]
      if (-not (Test-Path -LiteralPath $notice.Source -PathType Leaf) -or (Get-Item -LiteralPath $notice.Source).Length -eq 0) {
        throw "Missing or empty license text: $key/$relative"
      }
      $hash = (Get-FileHash -LiteralPath $notice.Source -Algorithm SHA256).Hash.ToLowerInvariant()
      if ($notice.ExpectedHash -and $hash -cne $notice.ExpectedHash) { throw "License supplement checksum mismatch: $key/$relative" }
      $target = Get-LicenseChildPath $Destination ("rust/$key/$relative")
      New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
      if (Test-Path -LiteralPath $target) { throw "License destination already exists: $target" }
      Copy-Item -LiteralPath $notice.Source -Destination $target
      $copied += [pscustomobject]@{ path = "rust/$key/$relative"; sha256 = $hash; origin = $notice.Origin }
    }
    $inventory += [pscustomobject]@{
      name = $dependency.name; version = $dependency.version; license = $dependency.license
      source = $dependency.source; files = $copied
    }
  }
  New-Item -ItemType Directory -Force -Path (Join-Path $Destination 'rust') | Out-Null
  $inventory | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $Destination 'rust-dependencies.json') -Encoding utf8
  @{ schema = 1; packages = $usedSupplements } | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath (Join-Path $Destination 'rust/sources.json') -Encoding utf8
  return ,$inventory
}
