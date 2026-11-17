<#
.SYNOPSIS
    Unified E2E verification runner for Phantom Spire (pluginval, MIDI-to-audio, UI screenshot & P5 judge, mock GitHub updater).
.DESCRIPTION
    Supports -Tier (1|2|3|4|All), -Suite (All|Pluginval|Audio|Ui|Updater), -DryRun, -SkipBuild, -OutDir.
    Exit code 0 iff all selected suites pass with 0 failed and 0 skipped tests.
    Produces consolidated e2e_summary.json and e2e_summary.md in OutDir.
#>
[CmdletBinding()]
param(
    [ValidateSet("1", "2", "3", "4", "All")]
    [string]$Tier = "All",

    [ValidateSet("All", "Pluginval", "Audio", "Ui", "Updater")]
    [string]$Suite = "All",

    [switch]$DryRun,
    [switch]$SkipBuild,
    [string]$OutDir = ""
)

$ErrorActionPreference = "Stop"

$E2eRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $E2eRoot

if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $E2eRoot ".artifacts"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$SummaryJson = Join-Path $OutDir "e2e_summary.json"
$SummaryMd   = Join-Path $OutDir "e2e_summary.md"

$PluginPath = Join-Path $RepoRoot "target\bundled\phantom_spire.vst3"
$ExePath    = Join-Path $RepoRoot "target\bundled\phantom_spire.exe"

Write-Host "============================================================"
Write-Host " Phantom Spire Unified E2E Runner"
Write-Host " Suite=$Suite | Tier=$Tier | DryRun=$($DryRun.IsPresent) | OutDir=$OutDir"
Write-Host "============================================================"

# Optional build step when not DryRun and not SkipBuild
if (-not $DryRun -and -not $SkipBuild) {
    if (-not (Test-Path $PluginPath) -or -not (Test-Path $ExePath)) {
        Write-Host "[run_e2e] Bundled artifacts missing; running cargo xtask bundle phantom_spire --release..."
        Push-Location $RepoRoot
        try {
            & cargo xtask bundle phantom_spire --release
            if ($LASTEXITCODE -ne 0) {
                throw "cargo xtask bundle phantom_spire --release failed with exit code $LASTEXITCODE"
            }
        } finally {
            Pop-Location
        }
    }
}

$suiteResults = @()

function Run-PluginvalSuite {
    Write-Host "`n--- [1/4] Suite: Pluginval ---"
    $script = Join-Path $E2eRoot "pluginval\run_pluginval.ps1"
    $argsList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $script, "-PluginPath", $PluginPath, "-OutDir", $OutDir)
    if ($DryRun) { $argsList += "-DryRun" }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & powershell @argsList | Out-Host
    $rc = $LASTEXITCODE
    $sw.Stop()

    $repPath = Join-Path $OutDir "pluginval_report.json"
    $rep = if (Test-Path $repPath) { Get-Content $repPath -Raw | ConvertFrom-Json } else { $null }
    return [pscustomobject][ordered]@{
        suite       = "pluginval"
        exit_code   = $rc
        passed      = $(if ($rep) { [int]$rep.passed } else { 0 })
        failed      = $(if ($rep) { [int]$rep.failed } else { $(if ($rc -eq 0) { 0 } else { 1 }) })
        total       = $(if ($rep) { [int]$rep.total } else { 0 })
        duration_ms = [int]$sw.ElapsedMilliseconds
        report_path = $repPath
        status      = $(if ($rc -eq 0 -and $rep -and [int]$rep.failed -eq 0) { "pass" } else { "fail" })
    }
}

function Run-AudioSuite {
    Write-Host "`n--- [2/4] Suite: MIDI-to-Audio (Opaque VST3 Harness + Feature Integration Tests) ---"
    $harnessDir = Join-Path $E2eRoot "midi_audio_harness"
    $vst3ReportPath = Join-Path $OutDir "audio_vst3_report.json"
    $audioReportPath = Join-Path $OutDir "audio_report.json"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()

    $totalPassed = 0
    $totalFailed = 0
    $cases = @()

    # Part A: Standalone opaque-box VST3 COM harness
    Push-Location $harnessDir
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        if ($DryRun) {
            & cargo run --quiet -- --dry-run --out $vst3ReportPath 2>&1 | Out-Host
        } else {
            & cargo run --quiet -- --plugin $PluginPath --out $vst3ReportPath 2>&1 | Out-Host
        }
        $vst3Rc = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prevEap
        Pop-Location
    }

    if (Test-Path $vst3ReportPath) {
        $vrep = Get-Content $vst3ReportPath -Raw | ConvertFrom-Json
        $totalPassed += [int]$vrep.passed
        $totalFailed += [int]$vrep.failed
        foreach ($c in $vrep.cases) { $cases += $c }
    } else {
        $totalFailed += 1
        $cases += [ordered]@{ id = "AUD-VST3-HARNESS"; name = "Opaque VST3 harness"; status = "fail"; details = "ExitCode=$vst3Rc, missing report" }
    }

    # Part B: Comprehensive Rust feature-by-feature MIDI-to-audio integration tests (`tests/audio_e2e_tests.rs`)
    if (-not $DryRun) {
        $intTestPath = Join-Path $RepoRoot "tests\audio_e2e_tests.rs"
        if (Test-Path $intTestPath) {
            Write-Host "[run_e2e] Running cargo test --test audio_e2e_tests..."
            Push-Location $RepoRoot
            $prevEap2 = $ErrorActionPreference
            $ErrorActionPreference = "Continue"
            try {
                $tOut = & cargo test --test audio_e2e_tests 2>&1
                $tRc = $LASTEXITCODE
            } finally {
                $ErrorActionPreference = $prevEap2
                Pop-Location
            }
            $tOut | Out-Host
            $tText = ($tOut -join "`n")
            $summaryMatch = [regex]::Match($tText, "test result: (ok|FAILED)\. (\d+) passed; (\d+) failed;")
            if ($summaryMatch.Success) {
                $pCnt = [int]$summaryMatch.Groups[2].Value
                $fCnt = [int]$summaryMatch.Groups[3].Value
                $totalPassed += $pCnt
                $totalFailed += $fCnt
                $cases += [ordered]@{
                    id      = "AUD-RUST-INTEGRATION"
                    name    = "Comprehensive Spire DSP/FX/Mod/Preset MIDI-to-audio integration suite"
                    status  = $(if ($tRc -eq 0 -and $fCnt -eq 0) { "pass" } else { "fail" })
                    details = "passed=$pCnt failed=$fCnt"
                }
            } else {
                $okT = ($tRc -eq 0)
                if ($okT) { $totalPassed += 1 } else { $totalFailed += 1 }
                $cases += [ordered]@{
                    id      = "AUD-RUST-INTEGRATION"
                    name    = "Comprehensive Spire DSP/FX/Mod/Preset MIDI-to-audio integration suite"
                    status  = $(if ($okT) { "pass" } else { "fail" })
                    details = "ExitCode=$tRc"
                }
            }
        }
    }

    $sw.Stop()
    $combined = [ordered]@{
        suite     = "audio"
        dry_run   = $DryRun.IsPresent
        timestamp = (Get-Date).ToString("o")
        total     = $totalPassed + $totalFailed
        passed    = $totalPassed
        failed    = $totalFailed
        cases     = $cases
    }
    $combined | ConvertTo-Json -Depth 6 | Set-Content -Path $audioReportPath -Encoding UTF8

    return [pscustomobject][ordered]@{
        suite       = "audio"
        exit_code   = $(if ($totalFailed -eq 0) { 0 } else { 1 })
        passed      = $totalPassed
        failed      = $totalFailed
        total       = $totalPassed + $totalFailed
        duration_ms = [int]$sw.ElapsedMilliseconds
        report_path = $audioReportPath
        status      = $(if ($totalFailed -eq 0 -and $totalPassed -gt 0) { "pass" } else { "fail" })
    }
}

function Run-UiSuite {
    Write-Host "`n--- [3/4] Suite: UI Capture & P5 Quantitative Judge ---"
    $script = Join-Path $E2eRoot "ui_judge\capture_ui.ps1"
    $argsList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $script, "-ExePath", $ExePath, "-OutDir", $OutDir)
    if ($DryRun) { $argsList += "-DryRun" }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & powershell @argsList | Out-Host
    $rc = $LASTEXITCODE
    $sw.Stop()

    $repPath = Join-Path $OutDir "ui_report.json"
    $rep = if (Test-Path $repPath) { Get-Content $repPath -Raw | ConvertFrom-Json } else { $null }
    return [pscustomobject][ordered]@{
        suite       = "ui_judge"
        exit_code   = $rc
        passed      = $(if ($rep) { [int]$rep.passed } else { 0 })
        failed      = $(if ($rep) { [int]$rep.failed } else { $(if ($rc -eq 0) { 0 } else { 1 }) })
        total       = $(if ($rep) { [int]$rep.total } else { 0 })
        duration_ms = [int]$sw.ElapsedMilliseconds
        report_path = $repPath
        status      = $(if ($rc -eq 0 -and $rep -and [int]$rep.failed -eq 0) { "pass" } else { "fail" })
    }
}

function Run-UpdaterSuite {
    Write-Host "`n--- [4/4] Suite: Updater, Cleansing & Offline Failure Modes ---"
    $script = Join-Path $E2eRoot "updater_mock_server\test_updater_e2e.ps1"
    $argsList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $script, "-ExePath", $ExePath, "-OutDir", $OutDir)
    if ($DryRun) { $argsList += "-DryRun" }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & powershell @argsList | Out-Host
    $rc = $LASTEXITCODE
    $sw.Stop()

    $repPath = Join-Path $OutDir "updater_report.json"
    $rep = if (Test-Path $repPath) { Get-Content $repPath -Raw | ConvertFrom-Json } else { $null }
    return [pscustomobject][ordered]@{
        suite       = "updater"
        exit_code   = $rc
        passed      = $(if ($rep) { [int]$rep.passed } else { 0 })
        failed      = $(if ($rep) { [int]$rep.failed } else { $(if ($rc -eq 0) { 0 } else { 1 }) })
        total       = $(if ($rep) { [int]$rep.total } else { 0 })
        duration_ms = [int]$sw.ElapsedMilliseconds
        report_path = $repPath
        status      = $(if ($rc -eq 0 -and $rep -and [int]$rep.failed -eq 0) { "pass" } else { "fail" })
    }
}

if ($Suite -eq "All" -or $Suite -eq "Pluginval") { $suiteResults += Run-PluginvalSuite }
if ($Suite -eq "All" -or $Suite -eq "Audio")     { $suiteResults += Run-AudioSuite }
if ($Suite -eq "All" -or $Suite -eq "Ui")        { $suiteResults += Run-UiSuite }
if ($Suite -eq "All" -or $Suite -eq "Updater")   { $suiteResults += Run-UpdaterSuite }

$grandTotal  = ($suiteResults | Measure-Object -Property total -Sum).Sum
$grandPassed = ($suiteResults | Measure-Object -Property passed -Sum).Sum
$grandFailed = ($suiteResults | Measure-Object -Property failed -Sum).Sum
$failedSuites = @($suiteResults | Where-Object { $_.status -ne "pass" }).Count
$overallPass = ($failedSuites -eq 0) -and ($grandFailed -eq 0) -and ($grandTotal -gt 0)

$summaryObj = [ordered]@{
    timestamp    = (Get-Date).ToString("o")
    dry_run      = $DryRun.IsPresent
    tier         = $Tier
    suite_filter = $Suite
    overall_pass = $overallPass
    total        = $grandTotal
    passed       = $grandPassed
    failed       = $grandFailed
    suites       = $suiteResults
}
$summaryObj | ConvertTo-Json -Depth 6 | Set-Content -Path $SummaryJson -Encoding UTF8

# Generate concise markdown summary
$md = @()
$md += "# Phantom Spire E2E Verification Summary"
$md += ""
$md += "- **Timestamp**: $($summaryObj.timestamp)"
$md += "- **Mode**: $(if ($DryRun) { 'DryRun' } else { 'Full E2E' }) | **Tier**: $Tier | **Suite**: $Suite"
$md += "- **Overall Result**: $(if ($overallPass) { 'PASS' } else { 'FAIL' }) ($grandPassed / $grandTotal passed, $grandFailed failed)"
$md += ""
$md += "| Suite | Status | Passed | Failed | Total | Duration (ms) | Report |"
$md += "|-------|--------|-------:|-------:|------:|--------------:|--------|"
foreach ($s in $suiteResults) {
    $md += "| $($s.suite) | $($s.status.ToUpper()) | $($s.passed) | $($s.failed) | $($s.total) | $($s.duration_ms) | ``$($s.report_path)`` |"
}
$md -join "`n" | Set-Content -Path $SummaryMd -Encoding UTF8

Write-Host "`n============================================================"
Write-Host " E2E SUMMARY: $(if ($overallPass) { 'PASS' } else { 'FAIL' }) ($grandPassed/$grandTotal passed, $grandFailed failed)"
Write-Host " JSON: $SummaryJson"
Write-Host " MD:   $SummaryMd"
Write-Host "============================================================"

if ($overallPass) { exit 0 } else { exit 1 }
