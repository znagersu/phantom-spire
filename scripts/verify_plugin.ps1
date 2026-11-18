<#
.SYNOPSIS
    Automatic 4-step verification script for Phantom Spire:
      1. Build the plugin (VST3 + Standalone EXE)
      2. Crash-test in background with C:\Tools\pluginval\pluginval.exe (strictness level 5)
      3. Sound-check ("Ears"): render test notes + chord to temp .wav and check silence/clipping/static/note-off clicks
      4. Visual-check ("Eyes"): capture UI screenshot invisibly in background to .png for visual inspection
#>
[CmdletBinding()]
param(
    [string]$PluginvalPath = "C:\Tools\pluginval\pluginval.exe",
    [switch]$AllTabs
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot  = Split-Path -Parent $ScriptDir

# Ensure Cargo and WinLibs MinGW64 (GNU toolchain) are on PATH
$mingwBin = "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
$env:PATH = "$env:USERPROFILE\.cargo\bin;$mingwBin;$env:PATH"

$Vst3Path = Join-Path $RepoRoot "target\bundled\phantom_spire.vst3"
$ExePath  = Join-Path $RepoRoot "target\bundled\phantom_spire.exe"

Write-Host "============================================================"
Write-Host " Phantom Spire Automatic Verification (Build -> Crash -> Ears -> Eyes)"
Write-Host "============================================================"

# ----------------------------------------------------------------------------
# 1. Build the plugin
# ----------------------------------------------------------------------------
Write-Host "`n[1/4] Building plugin (cargo xtask bundle phantom_spire --release)..."
Push-Location $RepoRoot
try {
    & cargo xtask bundle phantom_spire --release
    if ($LASTEXITCODE -ne 0) {
        throw "Build failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}
if (-not (Test-Path $Vst3Path)) { throw "Missing VST3 bundle: $Vst3Path" }
if (-not (Test-Path $ExePath))  { throw "Missing Standalone EXE: $ExePath" }
Write-Host "[1/4] PASS: Built $Vst3Path and $ExePath"

# ----------------------------------------------------------------------------
# 2. Crash-test in background using C:\Tools\pluginval\pluginval.exe (level 5)
# ----------------------------------------------------------------------------
Write-Host "`n[2/4] Crash-testing VST3 in background with pluginval (strictness level 5)..."
if (-not (Test-Path $PluginvalPath)) {
    $fallback = Join-Path $RepoRoot "e2e\tools\pluginval\pluginval.exe"
    if (Test-Path $fallback) {
        New-Item -ItemType Directory -Force -Path (Split-Path $PluginvalPath -Parent) | Out-Null
        Copy-Item -Force $fallback $PluginvalPath
    } else {
        throw "Missing pluginval.exe at $PluginvalPath"
    }
}

$PvOutDir = Join-Path $env:TEMP "phantom_spire_pluginval"
New-Item -ItemType Directory -Force -Path $PvOutDir | Out-Null
$PvStdout = Join-Path $PvOutDir "pluginval_stdout.log"
$PvStderr = Join-Path $PvOutDir "pluginval_stderr.log"
Remove-Item $PvStdout, $PvStderr -Force -ErrorAction SilentlyContinue

$oldDisable = $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE
$oldSilent  = $env:PHANTOM_SPIRE_UPDATER_SILENT
$env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE = "1"
$env:PHANTOM_SPIRE_UPDATER_SILENT      = "1"

try {
    $pvArgs = @(
        "--strictness-level", "5",
        "--validate-in-process",
        "--sample-rates", "44100,48000,96000",
        "--block-sizes", "64,128,256,512,1024",
        "--timeout-ms", "60000",
        "--output-dir", "`"$PvOutDir`"",
        "--validate", "`"$Vst3Path`""
    )
    $pvProc = Start-Process `
        -FilePath $PluginvalPath `
        -ArgumentList $pvArgs `
        -WindowStyle Hidden `
        -RedirectStandardOutput $PvStdout `
        -RedirectStandardError $PvStderr `
        -PassThru

    $null = $pvProc.Handle
    if (-not $pvProc.WaitForExit(120000)) {
        try { $pvProc.Kill() } catch {}
        throw "pluginval timed out after 120s"
    }

    $stdoutText = if (Test-Path $PvStdout) { Get-Content $PvStdout -Raw } else { "" }
    $stderrText = if (Test-Path $PvStderr) { Get-Content $PvStderr -Raw } else { "" }
    if ($pvProc.ExitCode -ne 0) {
        Write-Host $stdoutText
        if ($stderrText) { Write-Host $stderrText }
        throw "pluginval failed with exit code $($pvProc.ExitCode)"
    }
    if ($stdoutText -match "FAILED|CRASH|Exception") {
        Write-Host $stdoutText
        throw "pluginval output reported failures"
    }
    Write-Host "[2/4] PASS: pluginval strictness level 5 passed (logs: $PvOutDir)"
} finally {
    $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE = $oldDisable
    $env:PHANTOM_SPIRE_UPDATER_SILENT      = $oldSilent
}

# ----------------------------------------------------------------------------
# 3. Sound-check ("Ears"): notes + chord -> temp .wav -> quality checks
# ----------------------------------------------------------------------------
Write-Host "`n[3/4] Running Sound-Check ('Ears') -> temp .wav..."
$SoundCheckDir = Join-Path $ScriptDir "sound_check"
$WavDir  = Join-Path $env:TEMP "phantom_spire_soundcheck"
$WavPath = Join-Path $WavDir "soundcheck.wav"
New-Item -ItemType Directory -Force -Path $WavDir | Out-Null

$env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE = "1"
Push-Location $SoundCheckDir
try {
    & cargo run --quiet -- --plugin $Vst3Path --wav $WavPath
    if ($LASTEXITCODE -ne 0) {
        throw "Sound-check ('Ears') failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
    $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE = $oldDisable
}
Write-Host "[3/4] PASS: Audio verified clean at $WavPath"

# ----------------------------------------------------------------------------
# 4. Visual-check ("Eyes"): invisible background launch -> .png screenshot
# ----------------------------------------------------------------------------
Write-Host "`n[4/4] Running Visual-Check ('Eyes') -> invisible UI capture..."
Add-Type -AssemblyName System.Drawing

$VisualTempDir = Join-Path $env:TEMP "phantom_spire_visual"
$LocalArtDir   = Join-Path $ScriptDir ".artifacts"
New-Item -ItemType Directory -Force -Path $VisualTempDir, $LocalArtDir | Out-Null

$tabsToCapture = if ($AllTabs) { @("synth", "fx", "mod", "presets") } else { @("synth") }
$capturedPngs  = @()

$oldShotOut = $env:PHANTOM_SPIRE_SCREENSHOT_OUT
$oldExitShot = $env:PHANTOM_SPIRE_EXIT_AFTER_SCREENSHOT
$oldUiTab   = $env:PHANTOM_SPIRE_UI_TAB

$env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE   = "1"
$env:PHANTOM_SPIRE_UPDATER_SILENT        = "1"
$env:PHANTOM_SPIRE_EXIT_AFTER_SCREENSHOT = "1"

try {
    foreach ($tab in $tabsToCapture) {
        $fileName = if ($tab -eq "synth" -and -not $AllTabs) { "ui_screenshot.png" } else { "ui_$tab.png" }
        $tempPng  = Join-Path $VisualTempDir $fileName
        $localPng = Join-Path $LocalArtDir $fileName
        Remove-Item $tempPng, $localPng -Force -ErrorAction SilentlyContinue

        $env:PHANTOM_SPIRE_UI_TAB         = $tab
        $env:PHANTOM_SPIRE_SCREENSHOT_OUT = $tempPng

        $uiProc = Start-Process `
            -FilePath $ExePath `
            -ArgumentList "--backend", "dummy", "--dpi-scale", "1.0" `
            -WindowStyle Hidden `
            -PassThru

        if (-not $uiProc.WaitForExit(15000)) {
            try { $uiProc.Kill() } catch {}
        }

        if (-not (Test-Path $tempPng)) {
            throw "Screenshot not created for tab '$tab' at $tempPng"
        }

        Copy-Item -Force $tempPng $localPng

        # Quick pixel sanity check (1220x760 and non-blank across quadrants)
        $bmp = [System.Drawing.Bitmap]::new($tempPng)
        try {
            if ($bmp.Width -lt 1000 -or $bmp.Height -lt 600) {
                throw "Unexpected screenshot dimensions: $($bmp.Width)x$($bmp.Height)"
            }
            $nonDark = 0
            for ($y = 20; $y -lt $bmp.Height; $y += 24) {
                for ($x = 20; $x -lt $bmp.Width; $x += 24) {
                    $c = $bmp.GetPixel($x, $y)
                    if ($c.R -gt 20 -or $c.G -gt 20 -or $c.B -gt 20) {
                        $nonDark++
                    }
                }
            }
            if ($nonDark -lt 30) {
                throw "Screenshot appears blank/solid-dark (nonDark=$nonDark)"
            }
            Write-Host "[4/4] Captured '$tab' ($($bmp.Width)x$($bmp.Height)): $tempPng (mirror: $localPng)"
        } finally {
            $bmp.Dispose()
        }
        $capturedPngs += $localPng
    }
} finally {
    $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE   = $oldDisable
    $env:PHANTOM_SPIRE_UPDATER_SILENT        = $oldSilent
    $env:PHANTOM_SPIRE_SCREENSHOT_OUT        = $oldShotOut
    $env:PHANTOM_SPIRE_EXIT_AFTER_SCREENSHOT = $oldExitShot
    $env:PHANTOM_SPIRE_UI_TAB                = $oldUiTab
}

Write-Host "`n============================================================"
Write-Host " ALL 4 CHECKS PASSED!"
Write-Host "   WAV: $WavPath"
foreach ($png in $capturedPngs) {
    Write-Host "   PNG (inspect with read_file): $png"
}
Write-Host "============================================================"
