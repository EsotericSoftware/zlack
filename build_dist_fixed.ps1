# Builds a Windows installer that bundles a PRIVATE, fixed-version WebView2 runtime
# inside Zlack's own directory (issue #2). This lets users write a software-firewall
# rule for Zlack's own msedgewebview2.exe instead of allowing the shared, system-wide
# WebView2 (which any app can ride out to the internet).
#
# This is a SEPARATE build from the normal one (build_dist.ps1). The normal build stays
# small and uses the shared system runtime; this variant is ~150-180MB larger.
#
# The fixed runtime is NOT committed to the repo. Acquire it once, then this script
# reuses the extracted copy in src-tauri/webview2-runtime/.
#
# Acquiring the runtime (pick one):
#   A) Download the "Fixed Version" (.cab) for your architecture from
#        https://developer.microsoft.com/microsoft-edge/webview2/  (Download section)
#      then either:
#        - set  $env:WEBVIEW2_FIXED_CAB = "C:\path\to\Microsoft.WebView2.FixedVersionRuntime.<ver>.x64.cab"
#        - or pass it as the first argument:  ./build_dist_fixed.ps1 "C:\path\to.cab"
#   B) Or provide a direct download URL via  $env:WEBVIEW2_FIXED_URL
#   C) Or manually extract a cab so that src-tauri\webview2-runtime\msedgewebview2.exe exists.

param(
    [string]$CabPath = $env:WEBVIEW2_FIXED_CAB
)

$ErrorActionPreference = "Stop"

$runtimeDir = Join-Path $PSScriptRoot "src-tauri\webview2-runtime"
$runtimeExe = Join-Path $runtimeDir "msedgewebview2.exe"
$cabUrl = $env:WEBVIEW2_FIXED_URL

function Expand-FixedRuntime {
    param([string]$Cab)

    Write-Host "Extracting WebView2 fixed runtime from: $Cab"
    $tmp = Join-Path $env:TEMP ("zlack-webview2-" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    try {
        # `expand` ships with Windows and handles .cab files.
        & expand.exe "$Cab" -F:* "$tmp" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "expand.exe failed with exit code $LASTEXITCODE" }

        # The cab contains a top-level "Microsoft.WebView2.FixedVersionRuntime.<ver>.<arch>" folder.
        # Flatten it into webview2-runtime/ so the config path stays version-independent.
        $inner = Get-ChildItem -Path $tmp -Directory |
            Where-Object { Test-Path (Join-Path $_.FullName "msedgewebview2.exe") } |
            Select-Object -First 1
        $sourceRoot = if ($inner) { $inner.FullName } else { $tmp }

        if (-not (Test-Path (Join-Path $sourceRoot "msedgewebview2.exe"))) {
            throw "Extracted cab does not contain msedgewebview2.exe. Is this a Fixed Version runtime cab?"
        }

        if (Test-Path $runtimeDir) { Remove-Item -Recurse -Force $runtimeDir }
        New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null
        Copy-Item -Path (Join-Path $sourceRoot "*") -Destination $runtimeDir -Recurse -Force
        Write-Host "  OK Runtime extracted to $runtimeDir"
    }
    finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

# --- 1. Ensure the fixed runtime is present ---------------------------------
if (Test-Path $runtimeExe) {
    Write-Host "OK Reusing existing fixed runtime at $runtimeDir"
}
elseif ($CabPath -and (Test-Path $CabPath)) {
    Expand-FixedRuntime -Cab $CabPath
}
elseif ($cabUrl) {
    $dl = Join-Path $env:TEMP "zlack-webview2-fixed.cab"
    Write-Host "Downloading WebView2 fixed runtime from $cabUrl"
    Invoke-WebRequest -Uri $cabUrl -OutFile $dl
    Expand-FixedRuntime -Cab $dl
    Remove-Item -Force $dl -ErrorAction SilentlyContinue
}
else {
    Write-Error @"
WebView2 fixed runtime not found.

Acquire it once (see header of this script), then re-run. Quick options:
  - ./build_dist_fixed.ps1 "C:\path\to\Microsoft.WebView2.FixedVersionRuntime.<ver>.x64.cab"
  - `$env:WEBVIEW2_FIXED_CAB = "C:\path\to.cab"; ./build_dist_fixed.ps1`
  - `$env:WEBVIEW2_FIXED_URL = "https://.../...cab"; ./build_dist_fixed.ps1`
Download page: https://developer.microsoft.com/microsoft-edge/webview2/  (Fixed Version)
"@
    exit 1
}

# --- 2. Build with the fixed-runtime config patch --------------------------
Write-Host "Starting Zlack Build (fixed WebView2 runtime)..."
npm run tauri -- build --config "src-tauri/tauri.fixed.conf.json"
if ($LASTEXITCODE -ne 0) { throw "tauri build failed with exit code $LASTEXITCODE" }

# --- 3. Copy artifacts to dists/ with a distinguishing suffix ---------------
$distDir = "dists"
if (-not (Test-Path $distDir)) {
    New-Item -ItemType Directory -Force -Path $distDir | Out-Null
}

Write-Host "Copying artifacts to $distDir..."

function Copy-WithSuffix {
    param([string]$Glob, [string]$Label)
    $items = Get-ChildItem -Path $Glob -ErrorAction SilentlyContinue
    foreach ($item in $items) {
        $name = "$($item.BaseName)_webview2fixed$($item.Extension)"
        Copy-Item $item.FullName -Destination (Join-Path $distDir $name) -Force
        Write-Host "  OK $Label copied as $name"
    }
}

Copy-WithSuffix "src-tauri/target/release/bundle/msi/*.msi" "MSI"
Copy-WithSuffix "src-tauri/target/release/bundle/nsis/*.exe" "Setup EXE"

Write-Host "Fixed-runtime build complete! Artifacts are in the '$distDir' folder."
