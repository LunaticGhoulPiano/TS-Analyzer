# Shared release naming and completion of a local delivery build.
function Get-WindowsReleaseVersion([string]$Repository) {
  $platforms = Get-Content -Raw -LiteralPath (Join-Path $Repository 'developmentHelpers/packaging/platforms.toml')
  $section = [regex]::Match($platforms, '(?ms)^\[windows\]\s*(.*?)(?=^\[|\z)').Groups[1].Value
  $version = [regex]::Match($section, '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$').Groups[1].Value
  if (-not $version) { throw 'Missing Windows major.minor.patch version in platforms.toml.' }
  return $version
}

function Get-WindowsPackageName([string]$Version) {
  if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Version must be major.minor.patch.' }
  return "TS-Analyzer-windows-v$Version-x86_64"
}

function Complete-WindowsDeployment([string]$Artifacts, [string]$DeployDirectory, [string]$Version) {
  $name = Get-WindowsPackageName $Version
  $deploy = [IO.Path]::TrimEndingDirectorySeparator([IO.Path]::GetFullPath($DeployDirectory))
  $destination = Join-Path $deploy $name
  if (Test-Path -LiteralPath $destination) { throw "Delivery already exists: $destination. Use a new release version or explicitly remove an unpublished local build." }
  $files = @("$name-portable.zip", "$name-portable.zip.sha256", "$name-setup.exe", "$name-setup.exe.sha256")
  $entries = @(Get-ChildItem -LiteralPath $Artifacts -Force)
  if ($entries.Count -ne $files.Count) { throw 'A complete deployment requires only the ZIP, installer, and their two checksums.' }
  foreach ($file in $files) {
    $source = Join-Path $Artifacts $file
    if (-not (Test-Path -LiteralPath $source -PathType Leaf) -or
        (Get-Item -LiteralPath $source).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Missing or linked deployment file: $file" }
  }
  foreach ($file in @("$name-portable.zip", "$name-setup.exe")) {
    $source = Join-Path $Artifacts $file
    if ((Get-Item -LiteralPath $source).Length -eq 0) { throw "Empty deployment file: $file" }
    $digest = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    if ((Get-Content -Raw -LiteralPath "$source.sha256").Trim() -cne "$digest  $file") { throw "Deployment checksum mismatch: $file" }
  }
  New-Item -ItemType Directory -Force -Path $deploy | Out-Null
  # Copy beside the final directory so the final rename also works with cross-drive overrides.
  $staging = Join-Path $deploy ('.building-' + [Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $staging | Out-Null
  try {
    foreach ($file in $files) { [IO.File]::Copy((Join-Path $Artifacts $file), (Join-Path $staging $file)) }
    if ([IO.Path]::GetDirectoryName($staging) -ne $deploy -or
        [IO.Path]::GetDirectoryName($destination) -ne $deploy) { throw 'Delivery paths escaped the selected deployment directory.' }
    [IO.Directory]::Move($staging, $destination)
  } finally {
    # Only remove the four files owned by this invocation; never recursively delete a delivery.
    if (Test-Path -LiteralPath $staging) {
      foreach ($file in $files) { [IO.File]::Delete((Join-Path $staging $file)) }
      [IO.Directory]::Delete($staging)
    }
  }
  return $destination
}
