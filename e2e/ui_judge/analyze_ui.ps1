<#
.SYNOPSIS
    Quantitative pixel-metric analyzer for Phantom Spire UI screenshots (1220x760 P5 theme verification).
.DESCRIPTION
    Pure PowerShell 5.1 + .NET System.Drawing image inspection.
    Validates dimensions, non-blank whole/quadrant luminance stddev, P5 palette coverage,
    left crimson zone, bottom piano strip contrast, and optional pairwise tab differentiation.
    Supports -DryRun self-test against a synthetic valid P5 image (must PASS) and solid-black image (must FAIL).
#>
[CmdletBinding()]
param(
    [string[]]$ImagePaths = @(),
    [string]$ImagePathsCsv = "",
    [int]$ExpectedWidth = 1220,
    [int]$ExpectedHeight = 760,
    [string]$OutJson = "",
    [switch]$DryRun
)

if (-not [string]::IsNullOrWhiteSpace($ImagePathsCsv)) {
    $ImagePaths = $ImagePathsCsv.Split(",") | ForEach-Object { $_.Trim() } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
}

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

function Test-ColorNear {
    param([int]$R, [int]$G, [int]$B, [int]$TR, [int]$TG, [int]$TB, [int]$Tol)
    return ([Math]::Abs($R - $TR) -le $Tol) -and ([Math]::Abs($G - $TG) -le $Tol) -and ([Math]::Abs($B - $TB) -le $Tol)
}

function Analyze-UiImage {
    param(
        [string]$Path,
        [int]$ExpW,
        [int]$ExpH
    )
    if (-not (Test-Path $Path)) {
        return [ordered]@{
            path   = $Path
            passed = $false
            errors = @("File not found: $Path")
        }
    }

    $bmp = [System.Drawing.Bitmap]::FromFile($Path)
    try {
        $w = $bmp.Width
        $h = $bmp.Height
        $errors = @()
        if ($w -ne $ExpW -or $h -ne $ExpH) {
            $errors += "Dimension mismatch: got ${w}x${h}, expected ${ExpW}x${ExpH}"
        }

        # Sample on a 2px stride for fast, deterministic full-frame coverage (~231k pixels)
        $step = 2
        $nTotal = 0
        $sumLum = 0.0
        $sumLum2 = 0.0

        $quadN = @(0, 0, 0, 0)
        $quadSum = @(0.0, 0.0, 0.0, 0.0)
        $quadSum2 = @(0.0, 0.0, 0.0, 0.0)

        $darkCount = 0
        $crimsonCount = 0
        $whiteCount = 0
        $accentCount = 0

        $leftTotal = 0
        $leftCrimson = 0

        $pianoTotal = 0
        $pianoWhite = 0
        $pianoBlack = 0

        $halfW = [int]($w / 2)
        $halfH = [int]($h / 2)
        $leftMaxX = [Math]::Min(275, $w)
        $pianoMinY = [int]($h * 0.85)

        $rect = New-Object System.Drawing.Rectangle(0, 0, $w, $h)
        $bmpData = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $stride = [Math]::Abs($bmpData.Stride)
            $bytes = New-Object byte[] ($stride * $h)
            [System.Runtime.InteropServices.Marshal]::Copy($bmpData.Scan0, $bytes, 0, $bytes.Length)

            for ($y = 0; $y -lt $h; $y += $step) {
                $rowOff = $y * $stride
                $qRow = if ($y -lt $halfH) { 0 } else { 2 }
                $isPiano = ($y -ge $pianoMinY)

                for ($x = 0; $x -lt $w; $x += $step) {
                    $idx = $rowOff + ($x * 4)
                    $b = [int]$bytes[$idx]
                    $g = [int]$bytes[$idx + 1]
                    $r = [int]$bytes[$idx + 2]

                    $lum = 0.2126 * $r + 0.7152 * $g + 0.0722 * $b
                    $nTotal++
                    $sumLum += $lum
                    $sumLum2 += ($lum * $lum)

                    $qIdx = $qRow + $(if ($x -lt $halfW) { 0 } else { 1 })
                    $quadN[$qIdx]++
                    $quadSum[$qIdx] += $lum
                    $quadSum2[$qIdx] += ($lum * $lum)

                    # P5 dark obsidian (#0B0B0E / #0C0C10 / #131318 / #000000)
                    if ($r -le 36 -and $g -le 36 -and $b -le 44) {
                        $darkCount++
                    }
                    # P5 crimson (#E6001A / #8C0412)
                    $isCrim = ($r -ge 115 -and $g -le 48 -and $b -le 58 -and ($r - $g) -ge 85)
                    if ($isCrim) {
                        $crimsonCount++
                    }
                    # P5 crisp white (#F8F8FA)
                    if ($r -ge 215 -and $g -ge 215 -and $b -ge 215) {
                        $whiteCount++
                    }
                    # P5 accents: gold (#F7B801), cyan (#1CE8FF), magenta (#FF3EA5)
                    $isGold = ($r -ge 190 -and $g -ge 130 -and $b -le 65)
                    $isCyan = ($r -le 95 -and $g -ge 175 -and $b -ge 190)
                    $isMagenta = ($r -ge 195 -and $g -le 110 -and $b -ge 125)
                    if ($isGold -or $isCyan -or $isMagenta) {
                        $accentCount++
                    }

                    if ($x -lt $leftMaxX -and $y -lt $pianoMinY) {
                        $leftTotal++
                        if ($isCrim) { $leftCrimson++ }
                    }
                    if ($isPiano) {
                        $pianoTotal++
                        if ($lum -ge 210.0) { $pianoWhite++ }
                        elseif ($lum -le 40.0) { $pianoBlack++ }
                    }
                }
            }
        } finally {
            $bmp.UnlockBits($bmpData)
        }

        $mean = if ($nTotal -gt 0) { $sumLum / $nTotal } else { 0.0 }
        $var = if ($nTotal -gt 0) { [Math]::Max(0.0, ($sumLum2 / $nTotal) - ($mean * $mean)) } else { 0.0 }
        $stddev = [Math]::Sqrt($var)

        $quadStddevs = @()
        for ($q = 0; $q -lt 4; $q++) {
            $qn = $quadN[$q]
            if ($qn -gt 0) {
                $qm = $quadSum[$q] / $qn
                $qv = [Math]::Max(0.0, ($quadSum2[$q] / $qn) - ($qm * $qm))
                $quadStddevs += [Math]::Round([Math]::Sqrt($qv), 2)
            } else {
                $quadStddevs += 0.0
            }
        }

        $darkRatio = if ($nTotal -gt 0) { $darkCount / $nTotal } else { 0.0 }
        $crimsonRatio = if ($nTotal -gt 0) { $crimsonCount / $nTotal } else { 0.0 }
        $whiteRatio = if ($nTotal -gt 0) { $whiteCount / $nTotal } else { 0.0 }
        $accentRatio = if ($nTotal -gt 0) { $accentCount / $nTotal } else { 0.0 }
        $leftCrimsonRatio = if ($leftTotal -gt 0) { $leftCrimson / $leftTotal } else { 0.0 }
        $pianoWhiteRatio = if ($pianoTotal -gt 0) { $pianoWhite / $pianoTotal } else { 0.0 }
        $pianoBlackRatio = if ($pianoTotal -gt 0) { $pianoBlack / $pianoTotal } else { 0.0 }

        if ($stddev -le 20.0) {
            $errors += "Whole-image luminance stddev too low ($([Math]::Round($stddev, 2)) <= 20.0): image appears blank or flat"
        }
        for ($q = 0; $q -lt 4; $q++) {
            if ($quadStddevs[$q] -le 15.0) {
                $errors += "Quadrant $q luminance stddev too low ($($quadStddevs[$q]) <= 15.0)"
            }
        }
        if ($darkRatio -lt 0.15) {
            $errors += "Dark obsidian canvas ratio too low ($([Math]::Round($darkRatio * 100, 2))% < 15%)"
        }
        if ($crimsonRatio -lt 0.04) {
            $errors += "P5 crimson coverage too low ($([Math]::Round($crimsonRatio * 100, 2))% < 4%)"
        }
        if ($leftCrimsonRatio -lt 0.20) {
            $errors += "Left Crimson Command Zone red coverage too low ($([Math]::Round($leftCrimsonRatio * 100, 2))% < 20%)"
        }
        if ($whiteRatio -lt 0.04) {
            $errors += "Crisp white contrast ratio too low ($([Math]::Round($whiteRatio * 100, 2))% < 4%)"
        }
        if ($accentRatio -lt 0.002) {
            $errors += "P5 accent (gold/cyan/magenta) coverage too low ($([Math]::Round($accentRatio * 100, 3))% < 0.2%)"
        }
        if ($pianoWhiteRatio -lt 0.15 -or $pianoBlackRatio -lt 0.08) {
            $errors += "Bottom Live Monitor piano contrast missing (white=$([Math]::Round($pianoWhiteRatio*100,1))%, black=$([Math]::Round($pianoBlackRatio*100,1))%)"
        }

        return [ordered]@{
            path               = $Path
            width              = $w
            height             = $h
            luminance_mean     = [Math]::Round($mean, 2)
            luminance_stddev   = [Math]::Round($stddev, 2)
            quadrant_stddevs   = $quadStddevs
            dark_ratio         = [Math]::Round($darkRatio, 4)
            crimson_ratio      = [Math]::Round($crimsonRatio, 4)
            left_crimson_ratio = [Math]::Round($leftCrimsonRatio, 4)
            white_ratio        = [Math]::Round($whiteRatio, 4)
            accent_ratio       = [Math]::Round($accentRatio, 4)
            piano_white_ratio  = [Math]::Round($pianoWhiteRatio, 4)
            piano_black_ratio  = [Math]::Round($pianoBlackRatio, 4)
            passed             = ($errors.Count -eq 0)
            errors             = $errors
        }
    } finally {
        $bmp.Dispose()
    }
}

function Compare-ImagesDiffRatio {
    param([string]$PathA, [string]$PathB)
    $a = [System.Drawing.Bitmap]::FromFile($PathA)
    $b = [System.Drawing.Bitmap]::FromFile($PathB)
    try {
        if ($a.Width -ne $b.Width -or $a.Height -ne $b.Height) {
            return 1.0
        }
        $w = $a.Width
        $h = $a.Height
        $rect = New-Object System.Drawing.Rectangle(0, 0, $w, $h)
        $da = $a.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $db = $b.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $stride = [Math]::Abs($da.Stride)
            $ba = New-Object byte[] ($stride * $h)
            $bb = New-Object byte[] ($stride * $h)
            [System.Runtime.InteropServices.Marshal]::Copy($da.Scan0, $ba, 0, $ba.Length)
            [System.Runtime.InteropServices.Marshal]::Copy($db.Scan0, $bb, 0, $bb.Length)
            $diff = 0
            $tot = 0
            for ($y = 0; $y -lt $h; $y += 4) {
                $rowOff = $y * $stride
                for ($x = 270; $x -lt $w; $x += 4) {
                    $idx = $rowOff + ($x * 4)
                    $dr = [Math]::Abs([int]$ba[$idx + 2] - [int]$bb[$idx + 2])
                    $dg = [Math]::Abs([int]$ba[$idx + 1] - [int]$bb[$idx + 1])
                    $dbv = [Math]::Abs([int]$ba[$idx] - [int]$bb[$idx])
                    if (($dr + $dg + $dbv) -gt 24) { $diff++ }
                    $tot++
                }
            }
            if ($tot -eq 0) { return 0.0 }
            return [Math]::Round($diff / $tot, 4)
        } finally {
            $a.UnlockBits($da)
            $b.UnlockBits($db)
        }
    } finally {
        $a.Dispose()
        $b.Dispose()
    }
}

if ($DryRun) {
    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("PhantomSpireUiDryRun_" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
    try {
        # 1) Create valid synthetic 1220x760 P5 image
        $validPath = Join-Path $tmpDir "synthetic_p5_valid.png"
        $bmp = New-Object System.Drawing.Bitmap(1220, 760)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.Clear([System.Drawing.Color]::FromArgb(11, 11, 14))
        $redBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(230, 0, 26))
        $whiteBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(248, 248, 250))
        $blackBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(0, 0, 0))
        $goldBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(247, 184, 1))
        $cyanBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(28, 232, 255))
        $whitePen = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(248, 248, 250), 3)

        # Left crimson panel
        $g.FillRectangle($redBrush, 10, 10, 260, 625)
        $g.FillRectangle($whiteBrush, 20, 20, 240, 50)
        $g.FillRectangle($blackBrush, 30, 30, 100, 30)
        # Right cards + accents + ACTIVE NOTE trapezoid
        $g.DrawRectangle($whitePen, 290, 80, 440, 260)
        $g.DrawRectangle($whitePen, 750, 80, 450, 260)
        $g.DrawRectangle($whitePen, 290, 360, 910, 275)
        $g.FillRectangle($whiteBrush, 960, 14, 240, 55)
        $g.FillEllipse($goldBrush, 330, 120, 60, 60)
        $g.FillEllipse($cyanBrush, 800, 120, 60, 60)
        $g.FillRectangle($whiteBrush, 320, 400, 180, 40)
        $g.FillRectangle($goldBrush, 820, 420, 120, 35)
        # Bottom Live Monitor piano keys
        for ($k = 0; $k -lt 28; $k++) {
            $kx = 10 + $k * 43
            $g.FillRectangle($whiteBrush, $kx, 650, 40, 100)
            if (($k % 7) -ne 2 -and ($k % 7) -ne 6) {
                $g.FillRectangle($blackBrush, $kx + 26, 650, 24, 62)
            }
        }
        $bmp.Save($validPath, [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose()
        $bmp.Dispose()

        # 2) Create invalid solid-black 1220x760 image
        $blankPath = Join-Path $tmpDir "synthetic_blank.png"
        $bmp2 = New-Object System.Drawing.Bitmap(1220, 760)
        $g2 = [System.Drawing.Graphics]::FromImage($bmp2)
        $g2.Clear([System.Drawing.Color]::FromArgb(0, 0, 0))
        $bmp2.Save($blankPath, [System.Drawing.Imaging.ImageFormat]::Png)
        $g2.Dispose()
        $bmp2.Dispose()

        $validRes = Analyze-UiImage -Path $validPath -ExpW 1220 -ExpH 760
        $blankRes = Analyze-UiImage -Path $blankPath -ExpW 1220 -ExpH 760

        $ok = ($validRes.passed -eq $true) -and ($blankRes.passed -eq $false)
        $outObj = [ordered]@{
            dry_run           = $true
            valid_synthetic   = $validRes
            blank_negative    = $blankRes
            self_test_passed  = $ok
        }
        if (-not [string]::IsNullOrWhiteSpace($OutJson)) {
            $outObj | ConvertTo-Json -Depth 6 | Set-Content -Path $OutJson -Encoding UTF8
        }
        Write-Host "[analyze_ui] Dry-run valid_passed=$($validRes.passed) blank_rejected=$(-not $blankRes.passed)"
        if ($ok) { exit 0 } else { exit 1 }
    } finally {
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    }
}

if ($ImagePaths.Count -eq 0) {
    Write-Error "Provide -ImagePaths or -DryRun"
    exit 1
}

$results = @()
$allPass = $true
foreach ($img in $ImagePaths) {
    $res = Analyze-UiImage -Path $img -ExpW $ExpectedWidth -ExpH $ExpectedHeight
    $results += $res
    if (-not $res.passed) { $allPass = $false }
}

$pairDiffs = @()
if ($ImagePaths.Count -ge 2) {
    for ($i = 0; $i -lt $ImagePaths.Count; $i++) {
        for ($j = $i + 1; $j -lt $ImagePaths.Count; $j++) {
            $ratio = Compare-ImagesDiffRatio -PathA $ImagePaths[$i] -PathB $ImagePaths[$j]
            $pairDiffs += [ordered]@{
                image_a    = $ImagePaths[$i]
                image_b    = $ImagePaths[$j]
                diff_ratio = $ratio
                distinct   = ($ratio -ge 0.03)
            }
            if ($ratio -lt 0.03) { $allPass = $false }
        }
    }
}

$summary = [ordered]@{
    dry_run    = $false
    all_passed = $allPass
    images     = $results
    pair_diffs = $pairDiffs
}

if (-not [string]::IsNullOrWhiteSpace($OutJson)) {
    $summary | ConvertTo-Json -Depth 6 | Set-Content -Path $OutJson -Encoding UTF8
}

Write-Host "[analyze_ui] Analyzed $($results.Count) images; all_passed=$allPass"
if ($allPass) { exit 0 } else { exit 1 }
