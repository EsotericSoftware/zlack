$ErrorActionPreference = "Stop"

Write-Host "🚧 Starting Zlack Build..."
npm run tauri -- build
if ($LASTEXITCODE -ne 0) { throw "tauri build failed with exit code $LASTEXITCODE" }

$distDir = "dists"
if (-not (Test-Path $distDir)) {
    New-Item -ItemType Directory -Force -Path $distDir | Out-Null
}

Write-Host "📦 Copying artifacts to $distDir..."

# Copy MSI
$msiPath = "src-tauri/target/release/bundle/msi/*.msi"
if (Test-Path $msiPath) {
    Copy-Item $msiPath -Destination $distDir -Force
    Write-Host "  ✅ MSI copied."
}

# Copy NSIS Setup
$nsisPath = "src-tauri/target/release/bundle/nsis/*.exe"
if (Test-Path $nsisPath) {
    Copy-Item $nsisPath -Destination $distDir -Force
    Write-Host "  ✅ Setup EXE copied."
}

Write-Host "✨ Build complete! Artifacts are in the '$distDir' folder."
