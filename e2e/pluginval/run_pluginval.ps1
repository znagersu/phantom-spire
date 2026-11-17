<#
.SYNOPSIS
    Downloads/caches Tracktion pluginval (Windows) and validates phantom_spire.vst3 at strictness level 5.
.DESCRIPTION
    Opaque-box VST3 validation runner for Phantom Spire.
    Supports -DryRun to verify pluginval installation, version, and invocation guardrails without needing the built .vst3.
#>
[CmdletBinding()]
param(
    [string]$PluginPath = "",
    [string]$OutDir = "",
    [int]$Strictness = 5,
    [string]$SampleRates = "44100,48000,96000",
    [string]$BlockSizes = "64,128,256,512,1024",
    [int]$TimeoutMs = 60000,
    [string]$PluginvalVersion = "v1.0.4",
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$E2eRoot = Split-Path -Parent $ScriptDir
$RepoRoot = Split-Path -Parent $E2eRoot

if ([string]::IsNullOrWhiteSpace($PluginPath)) {
    $PluginPath = Join-Path $RepoRoot "target\bundled\phantom_spire.vst3"
}
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $E2eRoot ".artifacts"
}

$ToolsDir = Join-Path $E2eRoot "tools\pluginval"
$PluginvalExe = Join-Path $ToolsDir "pluginval.exe"
$PluginvalOutDir = Join-Path $OutDir "pluginval"
$ReportPath = Join-Path $OutDir "pluginval_report.json"

New-Item -ItemType Directory -Force -Path $ToolsDir | Out-Null
New-Item -ItemType Directory -Force -Path $PluginvalOutDir | Out-Null

function Write-JsonReport {
    param(
        [bool]$IsDryRun,
        [array]$Cases
    )
    $passed = @($Cases | Where-Object { $_.status -eq "pass" }).Count
    $failed = @($Cases | Where-Object { $_.status -eq "fail" }).Count
    $report = [ordered]@{
        suite     = "pluginval"
        dry_run   = $IsDryRun
        timestamp = (Get-Date).ToString("o")
        plugin    = $PluginPath
        tool      = $PluginvalExe
        total     = $Cases.Count
        passed    = $passed
        failed    = $failed
        cases     = $Cases
    }
    $report | ConvertTo-Json -Depth 6 | Set-Content -Path $ReportPath -Encoding UTF8
    return $failed
}

function Ensure-Pluginval {
    if (Test-Path $PluginvalExe) {
        return $true
    }
    Write-Host "[pluginval] Downloading Tracktion pluginval $PluginvalVersion..."
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $zipUrl = "https://github.com/Tracktion/pluginval/releases/download/$PluginvalVersion/pluginval_Windows.zip"
    $zipPath = Join-Path $ToolsDir "pluginval_Windows.zip"

    $downloaded = $false
    try {
        Invoke-WebRequest -Uri $zipUrl -OutFile $zipPath -UseBasicParsing -TimeoutSec 120
        $downloaded = (Test-Path $zipPath) -and ((Get-Item $zipPath).Length -gt 100000)
    } catch {
        Write-Warning "[pluginval] Invoke-WebRequest failed: $($_.Exception.Message). Trying curl.exe..."
        & curl.exe -fSL --retry 3 --connect-timeout 20 -o $zipPath $zipUrl
        $downloaded = ($LASTEXITCODE -eq 0) -and (Test-Path $zipPath) -and ((Get-Item $zipPath).Length -gt 100000)
    }

    if (-not $downloaded) {
        throw "Failed to download pluginval from $zipUrl"
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $extractTmp = Join-Path $ToolsDir "_extract_tmp"
    if (Test-Path $extractTmp) { Remove-Item -Recurse -Force $extractTmp }
    [System.IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $extractTmp)

    $foundExe = Get-ChildItem -Path $extractTmp -Recurse -Filter "pluginval.exe" | Select-Object -First 1
    if (-not $foundExe) {
        throw "pluginval.exe not found inside downloaded archive $zipPath"
    }

    Copy-Item -Path $foundExe.FullName -Destination $PluginvalExe -Force
    Remove-Item -Recurse -Force $extractTmp -ErrorAction SilentlyContinue
    Remove-Item -Force $zipPath -ErrorAction SilentlyContinue
    Write-Host "[pluginval] Cached at $PluginvalExe"
    return $true
}

function Invoke-PluginvalCapture {
    param(
        [string[]]$ArgsList,
        [int]$WaitMs = 30000
    )
    $tmpOut = Join-Path $PluginvalOutDir ("pv_out_" + [Guid]::NewGuid().ToString("N") + ".log")
    $tmpErr = Join-Path $PluginvalOutDir ("pv_err_" + [Guid]::NewGuid().ToString("N") + ".log")
    try {
        $proc = Start-Process -FilePath $PluginvalExe -ArgumentList $ArgsList -NoNewWindow -PassThru -RedirectStandardOutput $tmpOut -RedirectStandardError $tmpErr
        $null = $proc.Handle
        if (-not $proc.WaitForExit($WaitMs)) {
            try { $proc.Kill() } catch {}
            return [ordered]@{ TimedOut = $true; ExitCode = -1; StdOut = ""; StdErr = "Timed out" }
        }
        $proc.WaitForExit()
        $so = if (Test-Path $tmpOut) { Get-Content $tmpOut -Raw } else { "" }
        $se = if (Test-Path $tmpErr) { Get-Content $tmpErr -Raw } else { "" }
        return [ordered]@{ TimedOut = $false; ExitCode = $proc.ExitCode; StdOut = $so; StdErr = $se }
    } finally {
        Remove-Item -Force $tmpOut, $tmpErr -ErrorAction SilentlyContinue
    }
}

$cases = @()

# Step 1: Ensure pluginval binary is cached and responsive
$sw = [System.Diagnostics.Stopwatch]::StartNew()
try {
    Ensure-Pluginval | Out-Null
    $verRes = Invoke-PluginvalCapture -ArgsList @("--version") -WaitMs 15000
    $sw.Stop()
    $verStr = ("$($verRes.StdOut) $($verRes.StdErr)").Trim()
    if (-not $verRes.TimedOut -and $verRes.ExitCode -eq 0 -and -not [string]::IsNullOrWhiteSpace($verStr)) {
        $cases += [ordered]@{
            id          = "PV-01-TOOL-READY"
            name        = "pluginval binary cached and --version responsive"
            status      = "pass"
            duration_ms = [int]$sw.ElapsedMilliseconds
            details     = $verStr
        }
        Write-Host "[pluginval] Version: $verStr"
    } else {
        $cases += [ordered]@{
            id          = "PV-01-TOOL-READY"
            name        = "pluginval binary cached and --version responsive"
            status      = "fail"
            duration_ms = [int]$sw.ElapsedMilliseconds
            details     = "TimedOut=$($verRes.TimedOut) ExitCode=$($verRes.ExitCode) Output=$verStr"
        }
    }
} catch {
    $sw.Stop()
    $cases += [ordered]@{
        id          = "PV-01-TOOL-READY"
        name        = "pluginval binary cached and --version responsive"
        status      = "fail"
        duration_ms = [int]$sw.ElapsedMilliseconds
        details     = $_.Exception.Message
    }
}

if ($DryRun) {
    $swDry = [System.Diagnostics.Stopwatch]::StartNew()
    $helpRes = Invoke-PluginvalCapture -ArgsList @("--help") -WaitMs 15000
    $helpText = "$($helpRes.StdOut)`n$($helpRes.StdErr)"
    $requiredFlags = @("--strictness-level", "--validate", "--sample-rates", "--block-sizes", "--timeout-ms", "--output-dir")
    $missingFlags = @()
    foreach ($flag in $requiredFlags) {
        if ($helpText -notmatch [regex]::Escape($flag)) {
            $missingFlags += $flag
        }
    }
    $swDry.Stop()
    if ($missingFlags.Count -eq 0) {
        $cases += [ordered]@{
            id          = "PV-02-CLI-FLAGS"
            name        = "pluginval supports all required strict validation flags"
            status      = "pass"
            duration_ms = [int]$swDry.ElapsedMilliseconds
            details     = "Verified flags: $($requiredFlags -join ', ')"
        }
    } else {
        $cases += [ordered]@{
            id          = "PV-02-CLI-FLAGS"
            name        = "pluginval supports all required strict validation flags"
            status      = "fail"
            duration_ms = [int]$swDry.ElapsedMilliseconds
            details     = "Missing flags: $($missingFlags -join ', ')"
        }
    }

    $failCount = Write-JsonReport -IsDryRun $true -Cases $cases
    Write-Host "[pluginval] Dry-run complete. Report: $ReportPath (failed=$failCount)"
    if ($failCount -gt 0) { exit 1 } else { exit 0 }
}

# Full validation mode
if (-not (Test-Path $PluginPath)) {
    $cases += [ordered]@{
        id          = "PV-02-BUNDLE-EXISTS"
        name        = "VST3 bundle exists at target path"
        status      = "fail"
        duration_ms = 0
        details     = "Missing VST3 bundle: $PluginPath"
    }
    Write-JsonReport -IsDryRun $false -Cases $cases | Out-Null
    Write-Error "[pluginval] Missing VST3 bundle: $PluginPath"
    exit 1
}

$vst3Dll = Join-Path $PluginPath "Contents\x86_64-win\phantom_spire.vst3"
if (Test-Path $vst3Dll) {
    $cases += [ordered]@{
        id          = "PV-02-BUNDLE-STRUCTURE"
        name        = "Standard VST3 Windows x86_64 bundle layout"
        status      = "pass"
        duration_ms = 0
        details     = "Found $vst3Dll ($((Get-Item $vst3Dll).Length) bytes)"
    }
} else {
    $cases += [ordered]@{
        id          = "PV-02-BUNDLE-STRUCTURE"
        name        = "Standard VST3 Windows x86_64 bundle layout"
        status      = "fail"
        duration_ms = 0
        details     = "Missing expected binary $vst3Dll"
    }
}

# Run pluginval at strictness level 5 across sample rates and block sizes
# Ensure updater auto-check is disabled during pluginval
$oldDisableUpdate = $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE
$oldSilent = $env:PHANTOM_SPIRE_UPDATER_SILENT
$env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE = "1"
$env:PHANTOM_SPIRE_UPDATER_SILENT = "1"

$logFile = Join-Path $PluginvalOutDir "pluginval_stdout.log"
$errFile = Join-Path $PluginvalOutDir "pluginval_stderr.log"
$swRun = [System.Diagnostics.Stopwatch]::StartNew()
try {
    $argsList = @(
        "--strictness-level", "$Strictness",
        "--sample-rates", $SampleRates,
        "--block-sizes", $BlockSizes,
        "--timeout-ms", "$TimeoutMs",
        "--output-dir", $PluginvalOutDir,
        "--validate", $PluginPath
    )
    Write-Host "[pluginval] Running: $PluginvalExe $($argsList -join ' ')"
    $proc = Start-Process -FilePath $PluginvalExe -ArgumentList $argsList -NoNewWindow -PassThru -RedirectStandardOutput $logFile -RedirectStandardError $errFile
    $null = $proc.Handle

    # Max wall clock guard (5 minutes)
    $maxWaitMs = 300000
    if (-not $proc.WaitForExit($maxWaitMs)) {
        try { $proc.Kill() } catch {}
        $swRun.Stop()
        $cases += [ordered]@{
            id          = "PV-03-STRICT-LEVEL-$Strictness"
            name        = "pluginval strictness level $Strictness validation"
            status      = "fail"
            duration_ms = [int]$swRun.ElapsedMilliseconds
            details     = "Timed out after $maxWaitMs ms"
        }
    } else {
        $proc.WaitForExit()
        $swRun.Stop()
        $exitCode = $proc.ExitCode
        $stdoutText = if (Test-Path $logFile) { Get-Content $logFile -Raw } else { "" }
        $stderrText = if (Test-Path $errFile) { Get-Content $errFile -Raw } else { "" }
        $combined = "$stdoutText`n$stderrText"

        $failedMatches = [regex]::Matches($combined, "(?im)FAILED|FAILURE|CRASHED|TIMED OUT")
        $passedTests = [regex]::Matches($combined, "(?im)SUCCESS|All tests completed successfully")

        if ($exitCode -eq 0 -and $failedMatches.Count -eq 0) {
            $cases += [ordered]@{
                id          = "PV-03-STRICT-LEVEL-$Strictness"
                name        = "pluginval strictness level $Strictness validation (SR=$SampleRates, BS=$BlockSizes)"
                status      = "pass"
                duration_ms = [int]$swRun.ElapsedMilliseconds
                details     = "ExitCode=0, PassedMarkers=$($passedTests.Count), Log=$logFile"
            }
            Write-Host "[pluginval] PASS (exit=0, $([int]$swRun.ElapsedMilliseconds) ms)"
        } else {
            $tail = ($combined -split "`r?`n" | Select-Object -Last 25) -join "`n"
            $cases += [ordered]@{
                id          = "PV-03-STRICT-LEVEL-$Strictness"
                name        = "pluginval strictness level $Strictness validation (SR=$SampleRates, BS=$BlockSizes)"
                status      = "fail"
                duration_ms = [int]$swRun.ElapsedMilliseconds
                details     = "ExitCode=$exitCode FailedMarkers=$($failedMatches.Count) Log=$logFile`n$tail"
            }
            Write-Host "[pluginval] FAIL (exit=$exitCode)"
        }
    }
} finally {
    if ($null -eq $oldDisableUpdate) { Remove-Item Env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE -ErrorAction SilentlyContinue } else { $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE = $oldDisableUpdate }
    if ($null -eq $oldSilent) { Remove-Item Env:PHANTOM_SPIRE_UPDATER_SILENT -ErrorAction SilentlyContinue } else { $env:PHANTOM_SPIRE_UPDATER_SILENT = $oldSilent }
}

$failCount = Write-JsonReport -IsDryRun $false -Cases $cases
Write-Host "[pluginval] Report written to $ReportPath (failed=$failCount)"
if ($failCount -gt 0) { exit 1 } else { exit 0 }
