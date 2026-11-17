param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$Repo = "znagersu/phantom-spire",

    [switch]$SkipPublish,

    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"
$cleanVersion = $Version.Trim().TrimStart('v', 'V')
if ($cleanVersion -notmatch '^\d+\.\d+\.\d+([\-+][0-9A-Za-z\.\-]+)?$') {
    throw "Invalid SemVer version: '$Version'"
}
$tag = "v$cleanVersion"

# Ensure Cargo and WinLibs MinGW64 (GNU toolchain) are on PATH
$mingwBin = "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
$env:PATH = "$env:USERPROFILE\.cargo\bin;$mingwBin;$env:PATH"

$cargoToml = Join-Path $PSScriptRoot "Cargo.toml"
$content = Get-Content $cargoToml -Raw
$updated = [regex]::Replace($content, '(?m)^version = "[^"]+"', "version = `"$cleanVersion`"", 1)
[System.IO.File]::WriteAllText($cargoToml, $updated, [System.Text.UTF8Encoding]::new($false))

if (-not $SkipTests) {
    Write-Host "Running library & updater tests for $tag..."
    cargo test --lib
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    cargo test --test updater_tests
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

Write-Host "Building release bundles (phantom_spire.vst3 + phantom_spire.exe)..."
cargo xtask bundle phantom_spire --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$src = Join-Path $PSScriptRoot "target\bundled"
$vst3Bundle = Join-Path $src "phantom_spire.vst3"
$vst3Binary = Join-Path $vst3Bundle "Contents\x86_64-win\phantom_spire.vst3"
$exeBinary  = Join-Path $src "phantom_spire.exe"
$clapBinary = Join-Path $src "phantom_spire.clap"

if (-not (Test-Path $vst3Binary)) {
    throw "Missing built VST3 binary: $vst3Binary"
}
if (-not (Test-Path $exeBinary)) {
    throw "Missing built Standalone EXE: $exeBinary"
}

# Sync canonical local Windows install targets cleanly (no nested bundles)
$canonVst3Parent = "$env:LOCALAPPDATA\Programs\Common\VST3"
$canonVst3Dir    = Join-Path $canonVst3Parent "phantom_spire.vst3"
$canonExeParent  = "$env:LOCALAPPDATA\Programs\PhantomSpire"
$canonExePath    = Join-Path $canonExeParent "phantom_spire.exe"
New-Item -ItemType Directory -Force -Path $canonVst3Parent, $canonExeParent | Out-Null

if (Test-Path $canonVst3Dir) { Remove-Item -Recurse -Force $canonVst3Dir }
Copy-Item -Recurse -Force $vst3Bundle $canonVst3Parent
Copy-Item -Force $exeBinary $canonExePath

# Stage release archive + SHA-256 + latest.json
$staging = Join-Path $PSScriptRoot "release-staging"
if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
New-Item -ItemType Directory -Force -Path $staging | Out-Null

$zipName      = "PhantomSpire-win64.zip"
$zipPath      = Join-Path $staging $zipName
$shaPath      = Join-Path $staging "$zipName.sha256"
$manifestPath = Join-Path $staging "latest.json"

$archiveItems = @($vst3Bundle, $exeBinary)
if (Test-Path $clapBinary) {
    $archiveItems += $clapBinary
}

Compress-Archive -Path $archiveItems -DestinationPath $zipPath -Force

$zipHash  = (Get-FileHash -Algorithm SHA256 -Path $zipPath).Hash.ToLowerInvariant()
$vst3Hash = (Get-FileHash -Algorithm SHA256 -Path $vst3Binary).Hash.ToLowerInvariant()
$exeHash  = (Get-FileHash -Algorithm SHA256 -Path $exeBinary).Hash.ToLowerInvariant()

[System.IO.File]::WriteAllText(
    $shaPath,
    "$zipHash  $zipName`n",
    [System.Text.UTF8Encoding]::new($false)
)

# Pin zip_url directly to the exact release tag so CDN caches never mix versions
$manifest = [ordered]@{
    version      = $cleanVersion
    zip_url      = "https://github.com/$Repo/releases/download/$tag/$zipName"
    sha256       = $zipHash
    vst3_sha256  = $vst3Hash
    exe_sha256   = $exeHash
    published_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
} | ConvertTo-Json -Depth 4

[System.IO.File]::WriteAllText($manifestPath, $manifest, [System.Text.UTF8Encoding]::new($false))

Write-Host "Staged release assets in ${staging}:"
Write-Host "  ZIP:    $zipPath (SHA-256: $zipHash)"
Write-Host "  VST3:   SHA-256 $vst3Hash"
Write-Host "  EXE:    SHA-256 $exeHash"
Write-Host "  JSON:   $manifestPath"

if ($SkipPublish) {
    Write-Host "SkipPublish specified; stopping before git tag / GitHub Release upload."
    exit 0
}

git add -A
git diff --cached --quiet
if ($LASTEXITCODE -ne 0) {
    git commit -m "Release $tag"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
git tag -f $tag
git push -u origin main
git push -f origin $tag

$ErrorActionPreference = "Continue"
gh release view $tag *>$null
$releaseExists = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = "Stop"

if ($releaseExists) {
    gh release upload $tag $zipPath $shaPath $manifestPath --clobber
} else {
    gh release create $tag $zipPath $shaPath $manifestPath `
        --title "Phantom Spire $tag" `
        --notes "Persona 5 inspired Spire-style polyphonic synthesizer (Windows VST3 + Standalone EXE).`n`nSHA-256 (`$zipName`): ``$zipHash``"
}

Write-Host "Published $tag to GitHub Releases! Phantom Spire will auto-detect it."
