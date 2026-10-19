$ErrorActionPreference = "Stop"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Mod-O-Matic Publishing Script" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan

# 1. Check & Install Global Dependencies via Winget
function Ensure-WingetPackage {
    param(
        [string]$PackageId,
        [string]$CommandCheck
    )
    if (Get-Command $CommandCheck -ErrorAction SilentlyContinue) {
        Write-Host "[OK] $CommandCheck is already installed." -ForegroundColor Green
    } else {
        Write-Host "[INFO] Installing $PackageId via winget..." -ForegroundColor Yellow
        winget install --id $PackageId --silent --accept-source-agreements --accept-package-agreements
        # Refresh PATH in the current process
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
    }
}

Write-Host "`n[1/4] Checking global build dependencies..." -ForegroundColor Yellow
Ensure-WingetPackage -PackageId "Rustlang.Rustup" -CommandCheck "cargo"

# Ensure Rust environment is loaded into PATH if installed
if (Test-Path "$env:USERPROFILE\.cargo\bin") {
    if ($env:Path -notlike "*\.cargo\bin*") {
        $env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
    }
}

# 2. Close any currently running instance of the app so the exe is not locked
$runningProcess = Get-Process -Name "Mod-O-Matic", "Mod_O_Matic" -ErrorAction SilentlyContinue
if ($runningProcess) {
    Write-Warning "Mod-O-Matic is currently running (PID: $($runningProcess.Id -join ', ')). Terminating it to allow recompilation..."
    $runningProcess | Stop-Process -Force
    Start-Sleep -Seconds 1
}

# 3. Compile in Release Mode
Write-Host "`n[2/4] Compiling Mod-O-Matic in Release mode..." -ForegroundColor Yellow
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Error "Cargo build failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

# 4. Locate the Release Exe
Write-Host "`n[3/4] Locating executable..." -ForegroundColor Yellow
$projectRoot = $PSScriptRoot
if (-not $projectRoot) { $projectRoot = (Get-Location).Path }

$releaseDir = Join-Path $projectRoot "target\release"
$exePath = Join-Path $releaseDir "Mod-O-Matic.exe"

if (-not (Test-Path $exePath)) {
    $found = Get-ChildItem -Path $releaseDir -Filter "*Mod*Matic*.exe" | Select-Object -First 1
    if ($found) {
        $exePath = $found.FullName
    } else {
        Write-Error "Could not find built executable in $releaseDir"
        exit 1
    }
}

Write-Host "[OK] Executable found at: $exePath" -ForegroundColor Green

# 5. Zip up the executable in the target\release folder
Write-Host "`n[4/4] Creating zip archive..." -ForegroundColor Yellow
$zipName = "Mod-O-Matic-v0.1.0-win64.zip"
$zipPath = Join-Path $releaseDir $zipName

if (Test-Path $zipPath) {
    Remove-Item $zipPath -Force
}

Compress-Archive -Path $exePath -DestinationPath $zipPath -Force
Write-Host "[SUCCESS] Zip file created at: $zipPath" -ForegroundColor Green

Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "  Build & Publish Complete!" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan