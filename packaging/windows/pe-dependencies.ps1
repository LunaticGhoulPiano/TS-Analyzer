# Read PE import tables without installing dependency inspection tools.
function Get-PeImports([string]$Path) {
  $data = [IO.File]::ReadAllBytes($Path)
  if ($data.Length -lt 64 -or [BitConverter]::ToUInt16($data, 0) -ne 0x5A4D) { throw "Not a PE file: $Path" }
  $pe = [BitConverter]::ToInt32($data, 0x3C)
  if ($pe -lt 0 -or $pe + 24 -gt $data.Length -or [BitConverter]::ToUInt32($data, $pe) -ne 0x4550) { throw "Invalid PE header: $Path" }
  $count = [BitConverter]::ToUInt16($data, $pe + 6)
  $optionalSize = [BitConverter]::ToUInt16($data, $pe + 20)
  $optional = $pe + 24
  $magic = [BitConverter]::ToUInt16($data, $optional)
  if ($magic -eq 0x20B) { $directories = $optional + 112; $imageBase = [BitConverter]::ToUInt64($data, $optional + 24) }
  elseif ($magic -eq 0x10B) { $directories = $optional + 96; $imageBase = [BitConverter]::ToUInt32($data, $optional + 28) }
  else { throw "Unsupported PE optional header: $Path" }
  $sections = @()
  for ($i = 0; $i -lt $count; $i++) {
    $offset = $optional + $optionalSize + 40 * $i
    $sections += [pscustomobject]@{
      Rva = [BitConverter]::ToUInt32($data, $offset + 12)
      Size = [Math]::Max([BitConverter]::ToUInt32($data, $offset + 8), [BitConverter]::ToUInt32($data, $offset + 16))
      Raw = [BitConverter]::ToUInt32($data, $offset + 20)
    }
  }
  function Rva-Offset([long]$Rva) {
    foreach ($section in $sections) {
      if ($Rva -ge $section.Rva -and $Rva -lt ([long]$section.Rva + $section.Size)) {
        $value = [long]$section.Raw + $Rva - $section.Rva
        if ($value -lt 0 -or $value -ge $data.Length) { throw "Invalid PE RVA: $Path" }
        return [int]$value
      }
    }
    if ($Rva -ge 0 -and $Rva -lt $optional + $optionalSize) { return [int]$Rva }
    throw "Unmapped PE RVA: $Path"
  }
  $names = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
  foreach ($table in @(@(1,20,12), @(13,32,4))) {
    $directory = $directories + 8 * $table[0]
    if ($directory + 8 -gt $optional + $optionalSize) { continue }
    $rva = [BitConverter]::ToUInt32($data, $directory)
    $length = [BitConverter]::ToUInt32($data, $directory + 4)
    if ($rva -eq 0 -or $length -eq 0) { continue }
    $start = Rva-Offset $rva
    for ($offset = $start; $offset -lt $start + $length; $offset += $table[1]) {
      $nameRva = [long][BitConverter]::ToUInt32($data, $offset + $table[2])
      if ($nameRva -eq 0) { break }
      if ($table[0] -eq 13 -and ([BitConverter]::ToUInt32($data, $offset) -band 1) -eq 0) { $nameRva -= $imageBase }
      $nameOffset = Rva-Offset $nameRva
      $end = $nameOffset
      while ($end -lt $data.Length -and $data[$end] -ne 0 -and $end - $nameOffset -lt 1024) { $end++ }
      if ($end -ge $data.Length -or $end - $nameOffset -ge 1024) { throw "Invalid PE import name: $Path" }
      $name = [Text.Encoding]::ASCII.GetString($data, $nameOffset, $end - $nameOffset)
      if ([IO.Path]::GetFileName($name) -ne $name) { throw "Invalid PE import path: $Path" }
      $null = $names.Add($name)
    }
  }
  $names | Sort-Object
}

function Copy-PeDependencies([string[]]$Seeds, [string[]]$SearchRoots, [string]$RelativeDestination) {
  $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
  $pending = New-Object 'System.Collections.Generic.Queue[string]'
  foreach ($seed in $Seeds) { $pending.Enqueue($seed) }
  while ($pending.Count -gt 0) {
    $file = $pending.Dequeue()
    if (-not $seen.Add([IO.Path]::GetFileName($file))) { continue }
    foreach ($name in @(Get-PeImports $file)) {
      $dependency = $null
      foreach ($root in $SearchRoots) {
        $candidate = Join-Path $root $name
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { $dependency = $candidate; break }
      }
      if ($dependency) {
        Copy-One $dependency ($RelativeDestination + '/' + $name)
        $pending.Enqueue($dependency)
      } elseif ($name -notmatch '^(api-ms-|ext-ms-)' -and -not (Test-Path -LiteralPath (Join-Path $env:SystemRoot "System32/$name"))) {
        throw "Unresolved dependency $name required by $file"
      }
    }
  }
}
