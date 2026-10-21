<#
.SYNOPSIS
  Lists and removes Cargo build folders that no longer belong to a live worktree.

.DESCRIPTION
  Parallel agents build into their own folders (`target\agent-*` in the repository and `~\.imt-target-*`), and a
  finished one is never needed again; on 2026-10-21 they had grown to 2 TB. This script lists them with their sizes
  and, with -Remove, deletes the ones not named in -Keep. The repository's own `target\debug`, `target\release`,
  `target\rust-analyzer` and `target\flycheck*` (editor and Serena) are never touched.

.EXAMPLE
  scripts\clean-build-dirs.ps1                      # list with sizes
  scripts\clean-build-dirs.ps1 -Remove -Keep 6c,sdk # delete all except ~\.imt-target-6c and ~\.imt-target-sdk
#>
param(
  [switch]$Remove,
  # Suffixes of build folders still in use: `6c` keeps `~\.imt-target-6c*` and `target\agent-6c*`.
  [string[]]$Keep = @()
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$protected = @('debug', 'release', 'rust-analyzer', 'doc', 'tmp')

function Size-GB([string]$path) {
  $sum = (Get-ChildItem -LiteralPath $path -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
  [math]::Round(($sum ?? 0) / 1GB, 1)
}

function Kept([string]$name) {
  foreach ($k in $Keep) {
    if ($name -like ".imt-target-$k*" -or $name -like "agent-$k*") { return $true }
  }
  return $false
}

$candidates = @()
$candidates += Get-ChildItem -LiteralPath $HOME -Directory -Force -Filter '.imt-target*' -ErrorAction SilentlyContinue
$target = Join-Path $repo 'target'
if (Test-Path $target) {
  $candidates += Get-ChildItem -LiteralPath $target -Directory -ErrorAction SilentlyContinue |
    Where-Object { $protected -notcontains $_.Name -and $_.Name -notlike 'flycheck*' }
}

foreach ($dir in $candidates) {
  $gb = Size-GB $dir.FullName
  $keep = Kept $dir.Name
  $action = if (-not $Remove) { 'found' } elseif ($keep) { 'kept' } else { 'removing' }
  '{0,8:N1} GB  {1,-9} {2}' -f $gb, $action, $dir.FullName
  if ($Remove -and -not $keep) {
    Remove-Item -LiteralPath $dir.FullName -Recurse -Force -ErrorAction Continue
  }
}

$drive = Get-PSDrive -Name ((Get-Item $repo).PSDrive.Name)
'Free on {0}: {1:N1} GB' -f $drive.Name, ($drive.Free / 1GB)
