#requires -Version 7.0
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
. (Join-Path $repository 'developmentHelpers/scripts/windows/environment.ps1')
$roots = @((Join-Path $repository 'developmentHelpers'), (Join-Path $repository '.vscode/windows'), (Join-Path $repository 'scripts/windows'))
foreach ($root in $roots) {
  foreach ($file in Get-ChildItem -LiteralPath $root -Recurse -File -Filter '*.ps1' | Where-Object { $_.FullName -notlike '*\developmentHelpers\outputs\*' }) {
    $tokens = $null
    $errors = $null
    $null = [Management.Automation.Language.Parser]::ParseFile($file.FullName, [ref]$tokens, [ref]$errors)
    if ($errors.Count -gt 0) { throw "$($file.FullName): $($errors -join '; ')" }
  }
}
$tasks = Get-Content -Raw -LiteralPath (Join-Path $repository '.vscode/tasks.json') | ConvertFrom-Json
$inputs = @{}
foreach ($inputDefinition in $tasks.inputs) {
  if ($inputs.ContainsKey($inputDefinition.id)) { throw "Duplicate input: $($inputDefinition.id)" }
  $inputs[$inputDefinition.id] = $true
}
$labels = @{}
foreach ($task in $tasks.tasks) {
  if ($labels.ContainsKey($task.label)) { throw "Duplicate task: $($task.label)" }
  $labels[$task.label] = $true
  $index = [Array]::IndexOf([object[]]$task.args, '-File')
  if ($index -lt 0) { throw "Missing script argument: $($task.label)" }
  $script = $task.args[$index + 1].Replace('$' + '{workspaceFolder}', $repository)
  if (-not (Test-Path -LiteralPath $script -PathType Leaf)) { throw "Missing task entry: $script" }
  if ($task.type -ne 'process') { throw 'Use process tasks so spaces and shell settings cannot alter argument quoting.' }
  foreach ($argument in $task.args) {
    foreach ($reference in [regex]::Matches($argument, '\$\{input:([^}]+)\}')) {
      if (-not $inputs.ContainsKey($reference.Groups[1].Value)) { throw "Undefined task input: $($reference.Value)" }
    }
    if ($argument -match '^[A-Za-z]:[\\/]|^\\\\') { throw "Task arguments must not contain fixed machine paths: $($task.label)" }
  }
}
$relocated = 'D:\Work Folder\TS Analyzer'
$resolved = Resolve-DevelopmentPath 'developmentHelpers/test-data/inputs/local' $relocated
if ($resolved -ne 'D:\Work Folder\TS Analyzer\developmentHelpers\test-data\inputs\local') { throw "Relative resolution failed: $resolved" }
if ((Resolve-DevelopmentPath 'E:\Streams\test.ts' $relocated) -ne 'E:\Streams\test.ts') { throw 'Absolute override changed.' }
$failed = $false
try { $null = Get-DevelopmentConfiguration $repository 'developmentHelpers/config/not-present.local.psd1' } catch { $failed = $true }
if (-not $failed) { throw 'Missing explicit configuration was silently ignored.' }
$failed = $false
try { Invoke-DevelopmentTool (Get-Process -Id $PID).Path @('-NoProfile','-Command','exit 7') } catch { $failed = $true }
if (-not $failed) { throw 'External command failure did not propagate.' }
$originalPath = $env:PATH
$null = & (Join-Path $repository '.vscode/windows/Invoke-Development.ps1') -Action CheckEnvironment
if ($env:PATH -cne $originalPath) { throw 'Entry point leaked PATH into the caller.' }
Write-Output 'PASS PowerShell syntax, VS Code task paths and inputs, relocatable paths, explicit config errors, exit codes, and caller environment'

$fixture = New-DevelopmentOutput (Join-Path $repository 'developmentHelpers/outputs/windows') 'development-tests'
$vsRoot = Join-Path $fixture 'SDK with spaces'
$runtime = Join-Path $vsRoot 'VC/Redist/custom/deep/layout/x64/CRT'
$licenses = Join-Path $vsRoot 'Licenses'
New-Item -ItemType Directory -Force -Path $runtime, $licenses | Out-Null
if ((Find-MsvcLicenses $runtime) -ne $licenses) { throw 'MSVC licenses depend on a fixed ancestor count.' }
if (Find-MsvcLicenses (Join-Path $fixture 'missing')) { throw 'Missing runtime must not select unrelated licenses.' }
Write-Output 'PASS MSVC license discovery with a relocated SDK and variable nesting depth'
