# Install git hooks that ping the coder index endpoint after every commit or
# merge. The hook is a portable POSIX sh script so Git Bash on Windows, WSL,
# and Unix systems all execute it the same way. Hook content is ASCII-only
# per CLAUDE.md (no em-dashes, no smart quotes).
#
# Usage (from repo root):
#   .\scripts\install-coder-git-hook.ps1
#   .\scripts\install-coder-git-hook.ps1 -DaemonUrl http://127.0.0.1:7878

param(
    [string]$DaemonUrl = "http://127.0.0.1:7878"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$HooksDir = Join-Path $RepoRoot ".git\hooks"

if (-not (Test-Path $HooksDir)) {
    throw "No .git/hooks directory found at $HooksDir. Run this from a git repository root."
}

$SettingsPath = Join-Path $env:USERPROFILE ".claw\settings.json"
if (-not (Test-Path $SettingsPath)) {
    Write-Host "Warning: $SettingsPath not found." -ForegroundColor Yellow
    Write-Host "The hook will still install, but it needs a daemonKey entry to succeed at runtime." -ForegroundColor Yellow
}

# POSIX-sh hook body. Parses `daemonKey` out of ~/.claw/settings.json with
# grep+sed (jq not guaranteed on user machines) and POSTs a best-effort
# rebuild ping in the background so the hook never blocks the commit.
$HookBody = @"
#!/bin/sh
# Auto-installed by scripts/install-coder-git-hook.ps1. Fires coder file
# index rebuild after commit / merge. Non-blocking and best-effort.
SETTINGS="`$HOME/.claw/settings.json"
if [ ! -f "`$SETTINGS" ]; then
    exit 0
fi
KEY=`$(grep daemonKey "`$SETTINGS" | sed 's/.*"daemonKey"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
if [ -z "`$KEY" ]; then
    exit 0
fi
curl -s -m 2 -X POST -H "Authorization: Bearer `$KEY" "$DaemonUrl/code/index/rebuild" >/dev/null 2>&1 &
exit 0
"@

# Normalize to LF line endings - Git Bash and POSIX shells choke on CRLF
# shebang lines.
$HookBody = $HookBody -replace "`r`n", "`n"

foreach ($HookName in @("post-commit", "post-merge")) {
    $HookPath = Join-Path $HooksDir $HookName
    [System.IO.File]::WriteAllText($HookPath, $HookBody, (New-Object System.Text.UTF8Encoding $false))

    # git on POSIX checks the exec bit. On Windows NTFS it's a no-op, but
    # running chmod via Git Bash is cheap insurance for users on WSL/macOS.
    $GitBash = (Get-Command bash.exe -ErrorAction SilentlyContinue)
    if ($null -ne $GitBash) {
        & bash.exe -c "chmod +x '$($HookPath -replace '\\','/')'" 2>$null
    }

    Write-Host "Installed $HookName hook at $HookPath" -ForegroundColor Green
}

Write-Host ""
Write-Host "Hook installed. After each commit or merge, the daemon will reindex the repo." -ForegroundColor Cyan
Write-Host "Daemon URL: $DaemonUrl" -ForegroundColor DarkGray
Write-Host "Reads daemonKey from: $SettingsPath" -ForegroundColor DarkGray
