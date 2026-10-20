<#
.SYNOPSIS
    Starts the widget in dev mode (npm run tauri dev) and writes the output to .\logs.

.DESCRIPTION
    The widget is not installed. It runs as long as this window stays open.
    To quit: right-click in the widget > Close, or Ctrl+C here.

    First start after a change: Vite starts immediately, then cargo compiles
    for another 1-2 minutes. The widget only appears after "Running target\debug\...".

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\start-widget.ps1
#>
[CmdletBinding()]
param(
    [string]$TargetDir = (Split-Path $PSScriptRoot -Parent)
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

if (-not (Test-Path (Join-Path $TargetDir 'package.json'))) {
    throw ('No repo in {0}. Specify the repo path with -TargetDir.' -f $TargetDir)
}

$LogDir = Join-Path $PSScriptRoot 'logs'
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$LogFile = Join-Path $LogDir ('tauri-dev-{0}.log' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))

$conf = ConvertFrom-Json ([System.IO.File]::ReadAllText((Join-Path $TargetDir 'src-tauri\tauri.conf.json')))
$productName = [string]$conf.productName

# tauri dev needs port 1420 exclusively (strictPort in vite.config.ts)
$busy = @(Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue)
if ($busy.Count -gt 0) {
    $owner = $busy[0].OwningProcess
    $ownerName = ''
    $op = Get-Process -Id $owner -ErrorAction SilentlyContinue
    if ($op) { $ownerName = $op.ProcessName }
    Write-Warning ('Port 1420 is in use (PID {0} {1}). Probably an old "tauri dev" is still running - stop it first.' -f $owner, $ownerName)
    exit 1
}

Write-Host ''
Write-Host ('Starting "{0}" from {1}' -f $productName, $TargetDir) -ForegroundColor Yellow
Write-Host 'What it looks like: small window, 340 px wide, no title bar, always on top,'
Write-Host ('usually top left on the primary screen. Taskbar entry: "{0}".' -f $productName)
Write-Host 'Move: drag the gray bar at the top of the widget. Menu: right-click in the widget.'
Write-Host ('Log: {0}' -f $LogFile)
Write-Host ''

$oldEnc = $null
try { $oldEnc = [Console]::OutputEncoding; [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false } catch { $oldEnc = $null }

$code = 0
Push-Location $TargetDir
$ErrorActionPreference = 'Continue'
try {
    & npm.cmd run tauri dev 2>&1 | ForEach-Object {
        $line = "$_"
        Write-Host $line
        Add-Content -Path $LogFile -Value $line -Encoding UTF8
    }
    $code = $LASTEXITCODE
} finally {
    Pop-Location
    if ($null -ne $oldEnc) { try { [Console]::OutputEncoding = $oldEnc } catch { } }
}

Write-Host ''
Write-Host ('tauri dev exited (exit code {0}). Log: {1}' -f $code, $LogFile)
exit $code
