#Requires -RunAsAdministrator
param(
    [ValidateRange(1, 65535)]
    [int]$Port = 7878,
    [string]$BinaryPath = "",
    [switch]$NoBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$TaskName   = "ClawDaemon"
$TaskDesc   = "CheetahClaws always-on AI daemon (claw daemon)"
$RepoRoot   = Split-Path -Parent $PSScriptRoot
$RustDir    = Join-Path $RepoRoot "rust"
$ReleaseBin = Join-Path $RustDir "target\release\claw.exe"
$LogDir     = Join-Path $env:USERPROFILE ".claw"
$LogFile    = Join-Path $LogDir "daemon.log"

# 1. Resolve binary
if ($BinaryPath -eq "") { $BinaryPath = $ReleaseBin }

if (-not $NoBuild) {
    Write-Host "Building claw (release)..." -ForegroundColor Cyan
    Push-Location $RustDir
    try {
        cargo build --release --bin claw
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    } finally {
        Pop-Location
    }
    Write-Host "Build complete." -ForegroundColor Green
}

if (-not (Test-Path $BinaryPath)) {
    throw "Binary not found at $BinaryPath - run without -NoBuild or pass -BinaryPath."
}

Write-Host "Using binary: $BinaryPath" -ForegroundColor Cyan

# 2. Ensure log directory exists
if (-not (Test-Path $LogDir)) {
    New-Item -ItemType Directory -Path $LogDir | Out-Null
}

# 3. Write launcher script that logs output
$LauncherPath = Join-Path $LogDir "daemon-launcher.ps1"
$LauncherContent = @"
`$log  = '$LogFile'
`$exe  = '$BinaryPath'
`$port = $Port
Add-Content `$log "[`$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] daemon starting on port `$port"
try {
    & `$exe daemon --port `$port 2>&1 | Tee-Object -Append -FilePath `$log
} catch {
    Add-Content `$log "[`$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] daemon crashed: `$_"
}
"@

Set-Content -Path $LauncherPath -Value $LauncherContent -Encoding ASCII
Write-Host "Launcher written to: $LauncherPath" -ForegroundColor Cyan

# 4. Register the scheduled task
$Action = New-ScheduledTaskAction `
    -Execute "powershell.exe" `
    -Argument "-NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$LauncherPath`""

$Trigger = New-ScheduledTaskTrigger -AtStartup

$Principal = New-ScheduledTaskPrincipal `
    -UserId $env:USERNAME `
    -LogonType Interactive `
    -RunLevel Highest

$Settings = New-ScheduledTaskSettingsSet `
    -ExecutionTimeLimit (New-TimeSpan -Hours 0) `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -StartWhenAvailable `
    -MultipleInstances IgnoreNew

Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue

Register-ScheduledTask `
    -TaskName $TaskName `
    -Description $TaskDesc `
    -Action $Action `
    -Trigger $Trigger `
    -Principal $Principal `
    -Settings $Settings | Out-Null

Write-Host ""
Write-Host "Task '$TaskName' registered." -ForegroundColor Green
Write-Host ""
Write-Host "  Start now  :  Start-ScheduledTask -TaskName '$TaskName'"
Write-Host "  Stop       :  Stop-ScheduledTask  -TaskName '$TaskName'"
Write-Host "  Remove     :  Unregister-ScheduledTask -TaskName '$TaskName'"
Write-Host "  Logs       :  Get-Content '$LogFile' -Wait"
Write-Host "  Health     :  Invoke-RestMethod http://127.0.0.1:$Port/health"
Write-Host ""

$Answer = Read-Host "Start the daemon now? [Y/n]"
if ($Answer -match "^[Yy]?$") {
    Start-ScheduledTask -TaskName $TaskName
    Write-Host "Waiting for daemon..." -ForegroundColor Cyan
    Start-Sleep -Seconds 2
    try {
        $h = Invoke-RestMethod "http://127.0.0.1:$Port/health" -ErrorAction Stop
        Write-Host "Health check OK: uptime=$($h.uptime_secs)s pid=$($h.pid)" -ForegroundColor Green
    } catch {
        Write-Host "Daemon may still be starting - check logs: $LogFile" -ForegroundColor Yellow
    }
}
