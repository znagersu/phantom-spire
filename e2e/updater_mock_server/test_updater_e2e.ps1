<#
.SYNOPSIS
    End-to-end updater, old-file cleansing, and offline failure-mode test suite for Phantom Spire.
.DESCRIPTION
    Spins up a local mock GitHub Releases HTTP server via py.exe (stdlib only), verifies all manifest/ZIP/failure endpoints,
    runs Rust `tests/updater_tests.rs` integration tests, and launches `target/bundled/phantom_spire.exe` against
    unreachable and failing manifest URLs to guarantee zero hangs or crashes.
#>
[CmdletBinding()]
param(
    [string]$ExePath = "",
    [string]$OutDir = "",
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$E2eRoot = Split-Path -Parent $ScriptDir
$RepoRoot = Split-Path -Parent $E2eRoot
$ServerScript = Join-Path $ScriptDir "mock_github_server.py"

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $RepoRoot "target\bundled\phantom_spire.exe"
}
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $E2eRoot ".artifacts"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$ReportPath = Join-Path $OutDir "updater_report.json"

function Write-UpdaterReport {
    param([bool]$IsDryRun, [array]$Cases)
    $passed = @($Cases | Where-Object { $_.status -eq "pass" }).Count
    $failed = @($Cases | Where-Object { $_.status -eq "fail" }).Count
    $report = [ordered]@{
        suite     = "updater"
        dry_run   = $IsDryRun
        timestamp = (Get-Date).ToString("o")
        exe       = $ExePath
        total     = $Cases.Count
        passed    = $passed
        failed    = $failed
        cases     = $Cases
    }
    $report | ConvertTo-Json -Depth 6 | Set-Content -Path $ReportPath -Encoding UTF8
    return $failed
}

$cases = @()
$readyFile = Join-Path ([System.IO.Path]::GetTempPath()) ("ps_mock_srv_" + [Guid]::NewGuid().ToString("N") + ".json")
$serverProc = $null

try {
    # 1. Start mock GitHub Releases HTTP server via py.exe
    $serverProc = Start-Process -FilePath "py.exe" -ArgumentList @($ServerScript, "--ready-file", $readyFile) -NoNewWindow -PassThru
    $deadline = (Get-Date).AddSeconds(10)
    while (-not (Test-Path $readyFile) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
    if (-not (Test-Path $readyFile)) {
        throw "Mock GitHub server failed to start within 10 seconds"
    }
    $srv = Get-Content $readyFile -Raw | ConvertFrom-Json
    $baseUrl = $srv.base_url
    Write-Host "[updater_e2e] Mock GitHub server running at $baseUrl (zip sha256=$($srv.zip_sha256))"

    # 2. Verify valid manifest + cache-busting query tolerance
    $resp = Invoke-WebRequest -Uri "$baseUrl/latest.json?t=123456" -UseBasicParsing -TimeoutSec 5
    $manifest = $resp.Content | ConvertFrom-Json
    $okManifest = ($resp.StatusCode -eq 200) -and ($manifest.version -eq "9.9.9") -and ($manifest.sha256 -eq $srv.zip_sha256)
    $cases += [ordered]@{
        id      = "UPD-01-MOCK-MANIFEST"
        name    = "Mock GitHub latest.json serves valid SemVer + SHA-256 with cache-bust query"
        status  = $(if ($okManifest) { "pass" } else { "fail" })
        details = "Status=$($resp.StatusCode) Version=$($manifest.version)"
    }

    # 3. Verify UTF-8 BOM manifest endpoint
    $wc = New-Object System.Net.WebClient
    $bomBytes = $wc.DownloadData("$baseUrl/latest_bom.json")
    $hasBom = ($bomBytes.Length -ge 3) -and ($bomBytes[0] -eq 0xEF) -and ($bomBytes[1] -eq 0xBB) -and ($bomBytes[2] -eq 0xBF)
    $bomText = [System.Text.Encoding]::UTF8.GetString($bomBytes, 3, $bomBytes.Length - 3)
    $bomObj = $bomText | ConvertFrom-Json
    $cases += [ordered]@{
        id      = "UPD-02-MOCK-BOM"
        name    = "Mock server serves UTF-8 BOM manifest parseable after stripping BOM"
        status  = $(if ($hasBom -and $bomObj.version -eq "9.9.9") { "pass" } else { "fail" })
        details = "HasBom=$hasBom Version=$($bomObj.version)"
    }

    # 4. Verify ZIP download, SHA-256 hash match, and dual (.vst3 + .exe) archive structure
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $tmpZip = Join-Path ([System.IO.Path]::GetTempPath()) ("ps_mock_" + [Guid]::NewGuid().ToString("N") + ".zip")
    try {
        $wc.DownloadFile($srv.zip_url, $tmpZip)
        $actualHash = (Get-FileHash -Path $tmpZip -Algorithm SHA256).Hash.ToLowerInvariant()
        $zipArch = [System.IO.Compression.ZipFile]::OpenRead($tmpZip)
        $entryNames = $zipArch.Entries | ForEach-Object { $_.FullName }
        $zipArch.Dispose()
        $hasVst3 = $entryNames -contains "phantom_spire.vst3/Contents/x86_64-win/phantom_spire.vst3"
        $hasExe = $entryNames -contains "phantom_spire.exe"
        $okZip = ($actualHash -eq $srv.zip_sha256) -and $hasVst3 -and $hasExe
        $cases += [ordered]@{
            id      = "UPD-03-MOCK-ZIP-SHA256"
            name    = "Release ZIP matches SHA-256 and contains both phantom_spire.vst3 and phantom_spire.exe"
            status  = $(if ($okZip) { "pass" } else { "fail" })
            details = "HashMatch=$($actualHash -eq $srv.zip_sha256) HasVst3=$hasVst3 HasExe=$hasExe"
        }
    } finally {
        Remove-Item -Force $tmpZip -ErrorAction SilentlyContinue
    }

    # 5. Verify failure-mode endpoints (404, 500, malformed JSON, untrusted URL, bad SHA-256)
    $err404 = 0
    try { Invoke-WebRequest -Uri "$baseUrl/latest_404.json" -UseBasicParsing -TimeoutSec 5 | Out-Null } catch { $err404 = [int]$_.Exception.Response.StatusCode }
    $err500 = 0
    try { Invoke-WebRequest -Uri "$baseUrl/latest_500.json" -UseBasicParsing -TimeoutSec 5 | Out-Null } catch { $err500 = [int]$_.Exception.Response.StatusCode }
    $malformedRaw = (Invoke-WebRequest -Uri "$baseUrl/latest_malformed.json" -UseBasicParsing -TimeoutSec 5).Content
    $malformedFailedParse = $false
    try { $malformedRaw | ConvertFrom-Json | Out-Null } catch { $malformedFailedParse = $true }
    $untrustedObj = (Invoke-WebRequest -Uri "$baseUrl/latest_untrusted.json" -UseBasicParsing -TimeoutSec 5).Content | ConvertFrom-Json
    $badShaObj = (Invoke-WebRequest -Uri "$baseUrl/latest_badsha.json" -UseBasicParsing -TimeoutSec 5).Content | ConvertFrom-Json

    $okFailures = ($err404 -eq 404) -and ($err500 -eq 500) -and $malformedFailedParse -and ($untrustedObj.url -notlike "$baseUrl*") -and ($badShaObj.sha256 -ne $srv.zip_sha256)
    $cases += [ordered]@{
        id      = "UPD-04-MOCK-FAILURE-ENDPOINTS"
        name    = "Mock server failure endpoints (404, 500, malformed JSON, untrusted URL, bad SHA-256)"
        status  = $(if ($okFailures) { "pass" } else { "fail" })
        details = "HTTP404=$err404 HTTP500=$err500 MalformedRejected=$malformedFailedParse"
    }

    if (-not $DryRun) {
        # 6. Run Rust updater integration tests (`tests/updater_tests.rs`)
        $updaterTestFile = Join-Path $RepoRoot "tests\updater_tests.rs"
        if (Test-Path $updaterTestFile) {
            Write-Host "[updater_e2e] Running cargo test --test updater_tests..."
            $swTest = [System.Diagnostics.Stopwatch]::StartNew()
            Push-Location $RepoRoot
            $prevEap = $ErrorActionPreference
            $ErrorActionPreference = "Continue"
            try {
                $cargoOut = & cargo test --test updater_tests -- --nocapture 2>&1
                $cargoCode = $LASTEXITCODE
            } finally {
                $ErrorActionPreference = $prevEap
                Pop-Location
            }
            $swTest.Stop()
            $tail = ($cargoOut | Select-Object -Last 20) -join "`n"
            $cases += [ordered]@{
                id          = "UPD-05-RUST-INTEGRATION-SUITE"
                name        = "Rust updater_tests (SemVer, SHA-256, NTFS hot-swap, cleansing, failure modes)"
                status      = $(if ($cargoCode -eq 0) { "pass" } else { "fail" })
                duration_ms = [int]$swTest.ElapsedMilliseconds
                details     = "ExitCode=$cargoCode`n$tail"
            }
            Write-Host "[updater_e2e] cargo test --test updater_tests exit=$cargoCode"
        } else {
            $cases += [ordered]@{
                id      = "UPD-05-RUST-INTEGRATION-SUITE"
                name    = "Rust updater_tests exists and passes"
                status  = "fail"
                details = "Missing $updaterTestFile"
            }
        }

        # 7. Launch standalone EXE against unreachable port & 404/malformed manifest URLs (auto-update ENABLED, silent mode)
        if (-not (Test-Path $ExePath)) {
            $cases += [ordered]@{
                id      = "UPD-06-OFFLINE-EXE-LAUNCH"
                name    = "Standalone exe launches cleanly offline with unreachable manifest URL"
                status  = "fail"
                details = "Missing $ExePath"
            }
        } else {
            $scenarios = @(
                @{ id = "UPD-06-OFFLINE-UNREACHABLE"; url = "http://127.0.0.1:1/latest.json"; name = "Standalone stays responsive with unreachable manifest port" },
                @{ id = "UPD-07-OFFLINE-HTTP404";     url = "$baseUrl/latest_404.json";       name = "Standalone stays responsive when GitHub returns HTTP 404" },
                @{ id = "UPD-08-OFFLINE-MALFORMED";   url = "$baseUrl/latest_malformed.json"; name = "Standalone stays responsive on malformed JSON manifest" }
            )

            foreach ($sc in $scenarios) {
                $oldUrl = $env:PHANTOM_SPIRE_MANIFEST_URL
                $oldPrefix = $env:PHANTOM_SPIRE_TRUSTED_PREFIX
                $oldDisable = $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE
                $oldSilent = $env:PHANTOM_SPIRE_UPDATER_SILENT
                $env:PHANTOM_SPIRE_MANIFEST_URL = $sc.url
                $env:PHANTOM_SPIRE_TRUSTED_PREFIX = $baseUrl
                Remove-Item Env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE -ErrorAction SilentlyContinue
                $env:PHANTOM_SPIRE_UPDATER_SILENT = "1"

                $p = $null
                $swSc = [System.Diagnostics.Stopwatch]::StartNew()
                try {
                    $p = Start-Process -FilePath $ExePath -ArgumentList @("--backend", "dummy", "--dpi-scale", "1.0") -PassThru
                    $gotWindow = $false
                    $waitLimit = (Get-Date).AddSeconds(10)
                    while ((Get-Date) -lt $waitLimit) {
                        if ($p.HasExited) { break }
                        $p.Refresh()
                        if ($p.MainWindowHandle -ne [IntPtr]::Zero) {
                            $gotWindow = $true
                            break
                        }
                        Start-Sleep -Milliseconds 150
                    }

                    # Keep running 2.5s to ensure background updater thread fails cleanly without panicking/hanging UI
                    if ($gotWindow -and -not $p.HasExited) {
                        Start-Sleep -Milliseconds 2500
                        $p.Refresh()
                    }
                    $stillResponsive = $gotWindow -and (-not $p.HasExited) -and $p.Responding
                    if (-not $p.HasExited) {
                        $p.CloseMainWindow() | Out-Null
                        if (-not $p.WaitForExit(3000)) { $p.Kill() }
                    }
                    $swSc.Stop()
                    $cases += [ordered]@{
                        id          = $sc.id
                        name        = $sc.name
                        status      = $(if ($stillResponsive) { "pass" } else { "fail" })
                        duration_ms = [int]$swSc.ElapsedMilliseconds
                        details     = "URL=$($sc.url) GotWindow=$gotWindow Responsive=$stillResponsive"
                    }
                    Write-Host "[updater_e2e] $($sc.id): pass=$stillResponsive"
                } catch {
                    $swSc.Stop()
                    $cases += [ordered]@{
                        id          = $sc.id
                        name        = $sc.name
                        status      = "fail"
                        duration_ms = [int]$swSc.ElapsedMilliseconds
                        details     = $_.Exception.Message
                    }
                } finally {
                    if ($p -and -not $p.HasExited) { try { $p.Kill() } catch {} }
                    if ($null -eq $oldUrl) { Remove-Item Env:PHANTOM_SPIRE_MANIFEST_URL -ErrorAction SilentlyContinue } else { $env:PHANTOM_SPIRE_MANIFEST_URL = $oldUrl }
                    if ($null -eq $oldPrefix) { Remove-Item Env:PHANTOM_SPIRE_TRUSTED_PREFIX -ErrorAction SilentlyContinue } else { $env:PHANTOM_SPIRE_TRUSTED_PREFIX = $oldPrefix }
                    if ($null -eq $oldDisable) { Remove-Item Env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE -ErrorAction SilentlyContinue } else { $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE = $oldDisable }
                    if ($null -eq $oldSilent) { Remove-Item Env:PHANTOM_SPIRE_UPDATER_SILENT -ErrorAction SilentlyContinue } else { $env:PHANTOM_SPIRE_UPDATER_SILENT = $oldSilent }
                }
            }
        }
    }
} catch {
    $cases += [ordered]@{
        id      = "UPD-RUNTIME-ERROR"
        name    = "Updater E2E harness execution"
        status  = "fail"
        details = $_.Exception.Message
    }
} finally {
    if ($srv -and $srv.base_url) {
        try { Invoke-WebRequest -Uri "$($srv.base_url)/shutdown" -UseBasicParsing -TimeoutSec 2 | Out-Null } catch {}
    }
    if ($serverProc -and -not $serverProc.HasExited) {
        try { $serverProc.Kill() } catch {}
    }
    Remove-Item -Force $readyFile -ErrorAction SilentlyContinue
}

$failCount = Write-UpdaterReport -IsDryRun $DryRun.IsPresent -Cases $cases
Write-Host "[updater_e2e] Complete. Report: $ReportPath (failed=$failCount)"
if ($failCount -gt 0) { exit 1 } else { exit 0 }
