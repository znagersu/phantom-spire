<#
.SYNOPSIS
    Runs the Phantom Spire batch preset tester against every factory preset (and optional refs/*.wav).
.PARAMETER Fix
    Automatically normalize volumes and repair any clipping, muffled, piercing, or quiet presets.
.PARAMETER Rewrite
    Rewrite all presets from scratch grouped by instrument role, matching any .wav files in refs/.
.PARAMETER PreviewOut
    Optional path to stitch top 2 scored presets per category into a single preview .wav file.
#>
[CmdletBinding()]
param(
    [switch]$Fix,
    [switch]$Rewrite,
    [string]$PresetsDir = "presets\factory",
    [string]$RefsDir = "refs",
    [string]$PreviewOut = ""
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot  = Split-Path -Parent $ScriptDir
$BatchDir  = Join-Path $ScriptDir "preset_batch_check"

$mingwBin = "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
$env:PATH = "$env:USERPROFILE\.cargo\bin;$mingwBin;$env:PATH"

$exeArgs = @("--presets", $PresetsDir, "--refs", $RefsDir)
if ($Rewrite) { $exeArgs += "--rewrite" }
if ($Fix)     { $exeArgs += "--fix" }
if (-not [string]::IsNullOrWhiteSpace($PreviewOut)) {
    $exeArgs += @("--preview-out", $PreviewOut)
}

Push-Location $RepoRoot
try {
    & cargo run --manifest-path (Join-Path $BatchDir "Cargo.toml") --release -- @exeArgs
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
} finally {
    Pop-Location
}
