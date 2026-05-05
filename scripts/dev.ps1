# Local dev launcher (Windows / PowerShell).
# Requires: Rust, Node 20+, Redis running on 127.0.0.1:6379.

$ErrorActionPreference = "Stop"
Push-Location $PSScriptRoot\..

Write-Host "Building Rust workspace (release)..." -ForegroundColor Cyan
cargo build --release

Write-Host "Starting services..." -ForegroundColor Cyan
$ingestor = Start-Process -FilePath ".\target\release\ingestor.exe" -PassThru -WindowStyle Hidden
$analyzer = Start-Process -FilePath ".\target\release\analyzer.exe" -PassThru -WindowStyle Hidden
$api      = Start-Process -FilePath ".\target\release\api.exe"      -PassThru -WindowStyle Hidden

Write-Host "Ingestor PID:  $($ingestor.Id)"
Write-Host "Analyzer PID:  $($analyzer.Id)"
Write-Host "API PID:       $($api.Id)"
Write-Host ""
Write-Host "Backend running. API on http://127.0.0.1:8080"
Write-Host "Now run: cd web; npm install; npm run dev" -ForegroundColor Yellow
Write-Host ""
Write-Host "To stop: Stop-Process -Id $($ingestor.Id),$($analyzer.Id),$($api.Id)"

Pop-Location
