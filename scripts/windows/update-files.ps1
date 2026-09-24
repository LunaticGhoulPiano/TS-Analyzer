# Shared by the update worker and isolated filesystem tests. Windows PowerShell 5.1 compatible.
function Assert-PlainPath([string]$Path) {
  $current = [IO.Path]::GetFullPath($Path)
  while ($current) {
    if (Test-Path -LiteralPath $current) {
      if (((Get-Item -LiteralPath $current -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Update path contains a junction or symbolic link: $current"
      }
    }
    $parent = [IO.Path]::GetDirectoryName($current)
    if ($parent -eq $current) { break }
    $current = $parent
  }
}

function Assert-Deployment([string]$Root, [string]$Mode) {
  Assert-PlainPath $Root
  foreach ($required in @('TS-Analyzer.exe','app/tsan-gui.exe','package.toml','deployment.toml')) {
    $file = Join-Path $Root $required
    Assert-PlainPath $file
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Incomplete update target: $required" }
  }
  $text = Get-Content -Raw -LiteralPath (Join-Path $Root 'deployment.toml')
  if ($Mode -notin @('portable','installed') -or
      $text -notmatch ('(?m)^mode\s*=\s*"' + $Mode + '"\s*$') -or
      $text -notmatch '(?m)^schema\s*=\s*1\s*$' -or
      $text -notmatch '(?m)^app_id\s*=\s*"TSAnalyzer"\s*$') { throw 'Update target has a different deployment mode or application identity' }
}

function Get-ManagedPath([string]$Root, [string]$Name) {
  $relative = $Name.Replace('/', '\')
  if ($relative -match '[:*?"<>|]' -or [IO.Path]::IsPathRooted($relative) -or
      $relative -match '(^|\\)(\.{1,2}|[^\\]*[. ])(\\|$)' -or
      $relative -match '(?i)(^|\\)(con|prn|aux|nul|com[1-9]|lpt[1-9])(\.|\\|$)' -or
      $relative -notmatch '^(?i:app\\|scripts\\|runtime\\|docs\\|licenses\\|TS-Analyzer\.exe$|package\.toml$|deployment\.toml$|build-info\.toml$|README\.txt$|SHA256SUMS$)') {
    throw "Not a managed package file: $Name"
  }
  $fullRoot = [IO.Path]::GetFullPath($Root).TrimEnd('\','/')
  $full = [IO.Path]::GetFullPath((Join-Path $fullRoot $relative))
  if (-not $full.StartsWith($fullRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Managed path escapes package root' }
  Assert-PlainPath $full
  return $full
}

function Read-ManagedInventory([string]$Root, [switch]$VerifyHashes) {
  $inventory = @{}
  $hashFile = Get-ManagedPath $Root 'SHA256SUMS'
  foreach ($line in Get-Content -LiteralPath $hashFile) {
    if ($line -notmatch '^([a-fA-F0-9]{64})  (.+)$') { throw 'Malformed package inventory' }
    $digest = $Matches[1]
    $name = $Matches[2].Replace('\','/')
    $file = Get-ManagedPath $Root $name
    if ($name -eq 'SHA256SUMS' -or $inventory.ContainsKey($name)) { throw 'Duplicate or recursive package inventory entry' }
    $inventory[$name] = $digest
    if ($VerifyHashes -and ((-not (Test-Path -LiteralPath $file -PathType Leaf)) -or
        (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $digest)) { throw "Packaged file checksum mismatch: $name" }
  }
  foreach ($required in @('TS-Analyzer.exe','app/tsan-gui.exe','package.toml','deployment.toml')) {
    if (-not $inventory.ContainsKey($required)) { throw "Package inventory is missing $required" }
  }
  $inventory['SHA256SUMS'] = ''
  return $inventory
}

function Update-PortableFiles([string]$Source, [string]$Target, [string]$Backup) {
  Assert-Deployment $Source 'portable'
  Assert-Deployment $Target 'portable'
  Assert-PlainPath $Backup
  if (Test-Path -LiteralPath $Backup) { throw 'Use a fresh backup directory' }
  $new = Read-ManagedInventory $Source -VerifyHashes
  $old = Read-ManagedInventory $Target
  $names = @(@($old.Keys) + @($new.Keys) | Sort-Object -Unique)
  foreach ($name in $names) { $null = Get-ManagedPath $Target $name }
  New-Item -ItemType Directory -Path $Backup -Force | Out-Null
  $changed = New-Object 'System.Collections.Generic.List[object]'
  try {
    foreach ($name in $names) {
      $destination = Get-ManagedPath $Target $name
      $saved = Get-ManagedPath $Backup $name
      # New package files must not silently replace an unrelated user file.
      if (-not $old.ContainsKey($name) -and (Test-Path -LiteralPath $destination)) {
        throw "A user file occupies a new package path: $name"
      }
      $item = [pscustomobject]@{ Name = $name; Saved = $false; Written = $false }
      $changed.Add($item)
      if (Test-Path -LiteralPath $destination) {
        if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) { throw "Package file path is a directory: $name" }
        New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($saved)) | Out-Null
        Move-Item -LiteralPath $destination -Destination $saved -ErrorAction Stop
        $item.Saved = $true
      }
      if ($new.ContainsKey($name)) {
        New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($destination)) | Out-Null
        $item.Written = $true
        Copy-Item -LiteralPath (Get-ManagedPath $Source $name) -Destination $destination -ErrorAction Stop
      }
    }
    $null = Read-ManagedInventory $Target -VerifyHashes
  } catch {
    $original = $_
    $rollbackErrors = @()
    for ($i = $changed.Count - 1; $i -ge 0; $i--) {
      $item = $changed[$i]
      try {
        $destination = Get-ManagedPath $Target $item.Name
        if ($item.Written -and (Test-Path -LiteralPath $destination -PathType Leaf)) {
          Remove-Item -LiteralPath $destination -ErrorAction Stop
        }
        if ($item.Saved) {
          Move-Item -LiteralPath (Get-ManagedPath $Backup $item.Name) -Destination $destination -ErrorAction Stop
        }
      } catch { $rollbackErrors += $_.Exception.Message }
    }
    if ($rollbackErrors.Count) { throw "Update failed: $original. Recovery files are in $Backup. Rollback errors: $($rollbackErrors -join '; ')" }
    throw "Update failed; the previous package was restored: $original"
  }
}
