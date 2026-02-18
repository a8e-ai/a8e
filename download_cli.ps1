##############################################################################
# a8e (Articulate) CLI Install Script for Windows PowerShell
#
# Downloads the latest stable 'a8e' CLI binary from GitHub releases.
#
# Usage:
#   Invoke-WebRequest -Uri "https://github.com/a8e-ai/a8e/releases/download/stable/download_cli.ps1" -OutFile "download_cli.ps1"; .\download_cli.ps1
#
# Environment variables:
#   $env:A8E_BIN_DIR  - Install directory (default: $env:USERPROFILE\.local\bin)
#   $env:A8E_VERSION  - Specific version (e.g., "v0.1.0")
#   $env:A8E_PROVIDER - Provider for a8e
#   $env:A8E_MODEL    - Model for a8e
#   $env:CANARY       - If "true", downloads canary instead of stable
#   $env:CONFIGURE    - If "false", skips a8e configure
##############################################################################

$ErrorActionPreference = "Stop"

$REPO = "a8e-ai/a8e"
$OUT_FILE = "a8e.exe"

if (-not $env:A8E_BIN_DIR) {
    $env:A8E_BIN_DIR = Join-Path $env:USERPROFILE ".local\bin"
}

$RELEASE = if ($env:CANARY -eq "true") { "true" } else { "false" }
$CONFIGURE = if ($env:CONFIGURE -eq "false") { "false" } else { "true" }

if ($env:A8E_VERSION) {
    if ($env:A8E_VERSION -notmatch '^v?[0-9]+\.[0-9]+\.[0-9]+(-.*)?$') {
        Write-Error "Invalid version '$env:A8E_VERSION'. Expected: vX.Y.Z, vX.Y.Z-suffix, or X.Y.Z"
        exit 1
    }
    $RELEASE_TAG = if ($env:A8E_VERSION.StartsWith("v")) { $env:A8E_VERSION } else { "v$env:A8E_VERSION" }
} else {
    $RELEASE_TAG = if ($RELEASE -eq "true") { "canary" } else { "stable" }
}

# --- Detect Architecture ---
$ARCH = $env:PROCESSOR_ARCHITECTURE
if ($ARCH -eq "AMD64") {
    $ARCH = "x86_64"
} elseif ($ARCH -eq "ARM64") {
    Write-Error "Windows ARM64 is not currently supported."
    exit 1
} else {
    Write-Error "Unsupported architecture '$ARCH'."
    exit 1
}

# --- Download ---
$FILE = "a8e-$ARCH-pc-windows-msvc.zip"
$DOWNLOAD_URL = "https://github.com/$REPO/releases/download/$RELEASE_TAG/$FILE"

Write-Host "Downloading a8e ($RELEASE_TAG): $FILE..." -ForegroundColor Green

try {
    Invoke-WebRequest -Uri $DOWNLOAD_URL -OutFile $FILE -UseBasicParsing
} catch {
    Write-Error "Failed to download $DOWNLOAD_URL. Error: $($_.Exception.Message)"
    exit 1
}

# --- Extract ---
$TMP_DIR = Join-Path $env:TEMP "a8e_install_$(Get-Random)"
New-Item -ItemType Directory -Path $TMP_DIR -Force | Out-Null

try {
    Expand-Archive -Path $FILE -DestinationPath $TMP_DIR -Force
} catch {
    Write-Error "Failed to extract $FILE. Error: $($_.Exception.Message)"
    Remove-Item -Path $TMP_DIR -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

Remove-Item -Path $FILE -Force

$EXTRACT_DIR = $TMP_DIR
if (Test-Path (Join-Path $TMP_DIR "a8e-package")) {
    $EXTRACT_DIR = Join-Path $TMP_DIR "a8e-package"
}

# --- Install ---
if (-not (Test-Path $env:A8E_BIN_DIR)) {
    New-Item -ItemType Directory -Path $env:A8E_BIN_DIR -Force | Out-Null
}

$SOURCE_EXE = Join-Path $EXTRACT_DIR "a8e.exe"
$DEST_EXE = Join-Path $env:A8E_BIN_DIR $OUT_FILE

if (Test-Path $SOURCE_EXE) {
    if (Test-Path $DEST_EXE) { Remove-Item -Path $DEST_EXE -Force }
    Move-Item -Path $SOURCE_EXE -Destination $DEST_EXE -Force
} else {
    Write-Error "a8e.exe not found in extracted files"
    Remove-Item -Path $TMP_DIR -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

$DLL_FILES = Get-ChildItem -Path $EXTRACT_DIR -Filter "*.dll" -ErrorAction SilentlyContinue
foreach ($dll in $DLL_FILES) {
    $DEST_DLL = Join-Path $env:A8E_BIN_DIR $dll.Name
    if (Test-Path $DEST_DLL) { Remove-Item -Path $DEST_DLL -Force }
    Move-Item -Path $dll.FullName -Destination $DEST_DLL -Force
}

Remove-Item -Path $TMP_DIR -Recurse -Force -ErrorAction SilentlyContinue

# --- Configure ---
if ($CONFIGURE -eq "true") {
    Write-Host ""
    Write-Host "Running a8e configure..." -ForegroundColor Green
    try {
        & $DEST_EXE configure
    } catch {
        Write-Warning "Failed to run a8e configure. Run it manually later."
    }
} else {
    Write-Host "Skipping 'a8e configure' - run it manually later if needed." -ForegroundColor Yellow
}

# --- PATH check ---
if ($env:PATH -notlike "*$env:A8E_BIN_DIR*") {
    Write-Host ""
    Write-Host "Warning: a8e installed, but $env:A8E_BIN_DIR is not in your PATH." -ForegroundColor Yellow
    Write-Host "Add to user PATH (no admin required):" -ForegroundColor Yellow
    Write-Host "    [Environment]::SetEnvironmentVariable('PATH', `$env:PATH + ';$env:A8E_BIN_DIR', 'User')" -ForegroundColor Cyan
    Write-Host ""
}

Write-Host ""
Write-Host "a8e (Articulate) installed successfully!" -ForegroundColor Green
Write-Host "Installed at: $DEST_EXE" -ForegroundColor Green
Write-Host "Run 'a8e --help' to get started." -ForegroundColor Green
