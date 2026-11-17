<#
.SYNOPSIS
    Launches Phantom Spire standalone, captures 1220x760 screenshots of SYNTH, FX & MASTER, and MOD & ARP tabs,
    and runs quantitative P5 pixel validation via analyze_ui.ps1.
.DESCRIPTION
    Supports -DryRun to verify capture helper compilation and analyze_ui.ps1 self-tests without phantom_spire.exe.
#>
[CmdletBinding()]
param(
    [string]$ExePath = "",
    [string]$OutDir = "",
    [int]$ExpectedWidth = 1220,
    [int]$ExpectedHeight = 760,
    [int]$StartupWaitMs = 2500,
    [int]$TabSwitchWaitMs = 700,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$E2eRoot = Split-Path -Parent $ScriptDir
$RepoRoot = Split-Path -Parent $E2eRoot
$AnalyzeScript = Join-Path $ScriptDir "analyze_ui.ps1"

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $RepoRoot "target\bundled\phantom_spire.exe"
}
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $E2eRoot ".artifacts"
}

$ShotDir = Join-Path $OutDir "screenshots"
$ReportPath = Join-Path $OutDir "ui_report.json"
$AnalyzeJson = Join-Path $OutDir "ui_metrics.json"

New-Item -ItemType Directory -Force -Path $ShotDir | Out-Null

# Load Win32 DPI-aware capture + input helpers (strict C# 5 syntax for PowerShell 5.1)
if (-not ([System.Management.Automation.PSTypeName]'PhantomUiCapture.Win32').Type) {
    $csharpCode = @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;

namespace PhantomUiCapture
{
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X; public int Y; }

    public static class Win32
    {
        public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

        [DllImport("user32.dll")]
        public static extern bool SetProcessDPIAware();

        [DllImport("user32.dll")]
        public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

        [DllImport("user32.dll")]
        public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

        [DllImport("user32.dll")]
        public static extern bool IsWindowVisible(IntPtr hWnd);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

        [DllImport("user32.dll")]
        public static extern bool GetClientRect(IntPtr hWnd, out RECT lpRect);

        [DllImport("user32.dll")]
        public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);

        [DllImport("user32.dll")]
        public static extern bool ClientToScreen(IntPtr hWnd, ref POINT lpPoint);

        [DllImport("user32.dll")]
        public static extern bool SetForegroundWindow(IntPtr hWnd);

        [DllImport("user32.dll")]
        public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

        [DllImport("user32.dll")]
        public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, uint nFlags);

        [DllImport("user32.dll")]
        public static extern bool SetCursorPos(int X, int Y);

        [DllImport("user32.dll")]
        public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, UIntPtr dwExtraInfo);

        private const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
        private const uint MOUSEEVENTF_LEFTUP = 0x0004;
        private const uint PW_CLIENTONLY = 0x00000001;
        private const uint PW_RENDERFULLCONTENT = 0x00000002;

        public static IntPtr FindAppWindow(uint targetPid)
        {
            IntPtr best = IntPtr.Zero;
            int bestScore = 0;
            EnumWindows(delegate(IntPtr h, IntPtr lp)
            {
                uint pid;
                GetWindowThreadProcessId(h, out pid);
                if (pid == targetPid && IsWindowVisible(h))
                {
                    RECT cr;
                    GetClientRect(h, out cr);
                    int w = cr.Right - cr.Left;
                    int hh = cr.Bottom - cr.Top;
                    if (w >= 400 && hh >= 300)
                    {
                        StringBuilder title = new StringBuilder(256);
                        GetWindowText(h, title, 256);
                        int score = w * hh;
                        if (title.ToString().IndexOf("Phantom Spire", StringComparison.OrdinalIgnoreCase) >= 0)
                        {
                            score += 10000000;
                        }
                        if (score > bestScore)
                        {
                            bestScore = score;
                            best = h;
                        }
                    }
                }
                return true;
            }, IntPtr.Zero);
            return best;
        }

        public static void ClickClient(IntPtr hwnd, int clientX, int clientY)
        {
            POINT pt = new POINT();
            pt.X = clientX;
            pt.Y = clientY;
            ClientToScreen(hwnd, ref pt);
            SetForegroundWindow(hwnd);
            SetCursorPos(pt.X, pt.Y);
            System.Threading.Thread.Sleep(60);
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
            System.Threading.Thread.Sleep(40);
            mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, UIntPtr.Zero);
        }

        public static bool CaptureClientArea(IntPtr hwnd, string outPath, out int width, out int height)
        {
            RECT cr;
            GetClientRect(hwnd, out cr);
            width = cr.Right - cr.Left;
            height = cr.Bottom - cr.Top;
            if (width <= 0 || height <= 0)
            {
                return false;
            }

            using (Bitmap bmp = new Bitmap(width, height, PixelFormat.Format32bppArgb))
            {
                bool ok = false;
                using (Graphics g = Graphics.FromImage(bmp))
                {
                    IntPtr hdc = g.GetHdc();
                    try
                    {
                        ok = PrintWindow(hwnd, hdc, PW_CLIENTONLY | PW_RENDERFULLCONTENT);
                    }
                    finally
                    {
                        g.ReleaseHdc(hdc);
                    }
                }

                bool hasColor = false;
                if (ok)
                {
                    for (int y = 20; y < height; y += 37)
                    {
                        for (int x = 20; x < width; x += 41)
                        {
                            Color c = bmp.GetPixel(x, y);
                            if (c.R > 8 || c.G > 8 || c.B > 8)
                            {
                                hasColor = true;
                                break;
                            }
                        }
                        if (hasColor) break;
                    }
                }

                if (!hasColor)
                {
                    try
                    {
                        SetForegroundWindow(hwnd);
                        System.Threading.Thread.Sleep(150);
                        POINT topLeft = new POINT();
                        topLeft.X = 0;
                        topLeft.Y = 0;
                        ClientToScreen(hwnd, ref topLeft);
                        using (Graphics g2 = Graphics.FromImage(bmp))
                        {
                            g2.CopyFromScreen(topLeft.X, topLeft.Y, 0, 0, new Size(width, height), CopyPixelOperation.SourceCopy);
                        }
                    }
                    catch
                    {
                        // Keep PrintWindow buffer if CopyFromScreen is unavailable
                    }
                }

                bmp.Save(outPath, ImageFormat.Png);
                return true;
            }
        }
    }
}
"@
    Add-Type -TypeDefinition $csharpCode -ReferencedAssemblies System.Drawing
}

[PhantomUiCapture.Win32]::SetProcessDPIAware() | Out-Null

function Write-UiReport {
    param([bool]$IsDryRun, [array]$Cases)
    $passed = @($Cases | Where-Object { $_.status -eq "pass" }).Count
    $failed = @($Cases | Where-Object { $_.status -eq "fail" }).Count
    $report = [ordered]@{
        suite       = "ui_judge"
        dry_run     = $IsDryRun
        timestamp   = (Get-Date).ToString("o")
        exe         = $ExePath
        screenshots = $ShotDir
        metrics     = $AnalyzeJson
        total       = $Cases.Count
        passed      = $passed
        failed      = $failed
        cases       = $Cases
    }
    $report | ConvertTo-Json -Depth 6 | Set-Content -Path $ReportPath -Encoding UTF8
    return $failed
}

$cases = @()

if ($DryRun) {
    $cases += [ordered]@{
        id      = "UI-01-WIN32-HELPER"
        name    = "Win32 DPI-aware capture & input helper compiled cleanly"
        status  = "pass"
        details = "PhantomUiCapture.Win32 loaded with SetProcessDPIAware"
    }

    & powershell -NoProfile -ExecutionPolicy Bypass -File $AnalyzeScript -DryRun -OutJson $AnalyzeJson
    $anCode = $LASTEXITCODE
    $cases += [ordered]@{
        id      = "UI-02-ANALYZER-SELFTEST"
        name    = "Pixel analyzer passes valid synthetic P5 image and rejects blank image"
        status  = $(if ($anCode -eq 0) { "pass" } else { "fail" })
        details = "ExitCode=$anCode Metrics=$AnalyzeJson"
    }

    $failCount = Write-UiReport -IsDryRun $true -Cases $cases
    Write-Host "[ui_judge] Dry-run complete. Report: $ReportPath (failed=$failCount)"
    if ($failCount -gt 0) { exit 1 } else { exit 0 }
}

if (-not (Test-Path $ExePath)) {
    $cases += [ordered]@{
        id      = "UI-01-EXE-EXISTS"
        name    = "Standalone executable exists"
        status  = "fail"
        details = "Missing $ExePath"
    }
    Write-UiReport -IsDryRun $false -Cases $cases | Out-Null
    Write-Error "[ui_judge] Missing executable: $ExePath"
    exit 1
}

$oldDisable = $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE
$oldSilent = $env:PHANTOM_SPIRE_UPDATER_SILENT
$oldTab = $env:PHANTOM_SPIRE_UI_TAB
$env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE = "1"
$env:PHANTOM_SPIRE_UPDATER_SILENT = "1"

$tabSpecs = @(
    @{ id = "synth";     envTab = "synth"; label = "SYNTH" },
    @{ id = "fx_master"; envTab = "fx";    label = "FX & MASTER" },
    @{ id = "mod_arp";   envTab = "mod";   label = "MOD & ARP" }
)

$capturedFiles = @()
try {
    foreach ($tab in $tabSpecs) {
        $outPng = Join-Path $ShotDir ("$($tab.id).png")
        Remove-Item -Force $outPng -ErrorAction SilentlyContinue
        $env:PHANTOM_SPIRE_UI_TAB = $tab.envTab
        $env:PHANTOM_SPIRE_SCREENSHOT_OUT = $outPng
        $proc = $null
        try {
            Write-Host "[ui_judge] Launching $ExePath (tab=$($tab.envTab))..."
            $proc = Start-Process -FilePath $ExePath -ArgumentList @("--backend", "dummy", "--dpi-scale", "1.0") -PassThru

            # Wait up to 12 seconds for real Phantom Spire window handle (ignoring UAC overlay windows)
            $hwnd = [IntPtr]::Zero
            $deadline = (Get-Date).AddMilliseconds(12000)
            while ((Get-Date) -lt $deadline) {
                if ($proc.HasExited) {
                    throw "phantom_spire.exe exited prematurely with code $($proc.ExitCode)"
                }
                $found = [PhantomUiCapture.Win32]::FindAppWindow([uint32]$proc.Id)
                if ($found -ne [IntPtr]::Zero) {
                    $hwnd = $found
                    break
                }
                Start-Sleep -Milliseconds 150
            }

            if ($hwnd -eq [IntPtr]::Zero) {
                throw "Timed out waiting for Phantom Spire window handle (tab=$($tab.envTab))"
            }

            [PhantomUiCapture.Win32]::ShowWindow($hwnd, 9) | Out-Null
            [PhantomUiCapture.Win32]::SetForegroundWindow($hwnd) | Out-Null
            Start-Sleep -Milliseconds $StartupWaitMs

            $capW = 0
            $capH = 0
            $ok = $false
            if ((Test-Path $outPng) -and ((Get-Item $outPng).Length -gt 1024)) {
                $imgCheck = [System.Drawing.Bitmap]::FromFile($outPng)
                $capW = $imgCheck.Width
                $capH = $imgCheck.Height
                $imgCheck.Dispose()
                $ok = $true
                Write-Host "[ui_judge] Captured $($tab.label) via built-in screenshot: $outPng (${capW}x${capH})"
            } else {
                $ok = [PhantomUiCapture.Win32]::CaptureClientArea($hwnd, $outPng, [ref]$capW, [ref]$capH)
                Write-Host "[ui_judge] Captured $($tab.label) via Win32 fallback: $outPng (${capW}x${capH})"
            }

            if ($ok -and (Test-Path $outPng)) {
                $capturedFiles += $outPng
                $cases += [ordered]@{
                    id      = "UI-CAP-$($tab.id.ToUpper())"
                    name    = "Captured $($tab.label) screenshot"
                    status  = $(if ($capW -eq $ExpectedWidth -and $capH -eq $ExpectedHeight) { "pass" } else { "fail" })
                    details = "Path=$outPng Size=${capW}x${capH} Expected=${ExpectedWidth}x${ExpectedHeight}"
                }
            } else {
                $cases += [ordered]@{
                    id      = "UI-CAP-$($tab.id.ToUpper())"
                    name    = "Captured $($tab.label) screenshot"
                    status  = "fail"
                    details = "Capture failed for $($tab.label)"
                }
            }

            if ($proc -and -not $proc.HasExited) {
                $proc.CloseMainWindow() | Out-Null
                if (-not $proc.WaitForExit(3000)) {
                    $proc.Kill()
                }
            }
        } finally {
            Remove-Item Env:PHANTOM_SPIRE_SCREENSHOT_OUT -ErrorAction SilentlyContinue
            if ($proc -and -not $proc.HasExited) {
                try { $proc.Kill() } catch {}
            }
        }
    }

    if ($capturedFiles.Count -gt 0) {
        $joinedPaths = ($capturedFiles -join ",")
        & powershell -NoProfile -ExecutionPolicy Bypass -File $AnalyzeScript -ImagePathsCsv $joinedPaths -ExpectedWidth $ExpectedWidth -ExpectedHeight $ExpectedHeight -OutJson $AnalyzeJson
        $anCode = $LASTEXITCODE
        $cases += [ordered]@{
            id      = "UI-METRICS-P5"
            name    = "Quantitative P5 pixel metrics & tab differentiation across all tabs"
            status  = $(if ($anCode -eq 0) { "pass" } else { "fail" })
            details = "ExitCode=$anCode Metrics=$AnalyzeJson"
        }
    }
} catch {
    $cases += [ordered]@{
        id      = "UI-RUNTIME-ERROR"
        name    = "Standalone launch and UI capture"
        status  = "fail"
        details = $_.Exception.Message
    }
} finally {
    if ($null -eq $oldDisable) { Remove-Item Env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE -ErrorAction SilentlyContinue } else { $env:PHANTOM_SPIRE_DISABLE_AUTO_UPDATE = $oldDisable }
    if ($null -eq $oldSilent) { Remove-Item Env:PHANTOM_SPIRE_UPDATER_SILENT -ErrorAction SilentlyContinue } else { $env:PHANTOM_SPIRE_UPDATER_SILENT = $oldSilent }
    if ($null -eq $oldTab) { Remove-Item Env:PHANTOM_SPIRE_UI_TAB -ErrorAction SilentlyContinue } else { $env:PHANTOM_SPIRE_UI_TAB = $oldTab }
}

$failCount = Write-UiReport -IsDryRun $false -Cases $cases
Write-Host "[ui_judge] Complete. Report: $ReportPath (failed=$failCount)"
if ($failCount -gt 0) { exit 1 } else { exit 0 }
