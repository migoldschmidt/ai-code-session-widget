<#
.SYNOPSIS
    Lists the clickable elements in the Claude Desktop window (UI Automation),
    so that "Select in sidebar" and the automatic switch to "Code" use the
    correct names.

.DESCRIPTION
    Read-only - does not click anything. Output is also written to
    .\logs\claude-ui-<time>.txt. The file contains labels from the
    Claude UI (e.g. session and chat titles) - review before sharing.

    Best run twice: once with Claude Desktop in "Chat" (or
    "Cowork") and once in "Code". Claude Desktop must not be closed
    to the tray.

    Markers:
      <== Session title        name matches a running session
      <== Code toggle?         name is "Code" or "Claude Code"
      [selected]               element is currently selected (e.g. active tab)

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\dump-claude-ui.ps1
#>
[CmdletBinding()]
param(
    [int]$Attempts = 3
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$LogDir = Join-Path $PSScriptRoot 'logs'
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$OutFile = Join-Path $LogDir ('claude-ui-{0}.txt' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$Utf8NoBom = New-Object System.Text.UTF8Encoding $false
$lines = New-Object System.Collections.Generic.List[string]

function Out-Line([string]$Text = '') {
    Write-Host $Text
    $lines.Add($Text)
}

trap {
    $err = $_
    $report = New-Object System.Collections.Generic.List[string]
    $report.Add('')
    $report.Add(('ERROR: {0}' -f $err.Exception.Message))
    if ($null -ne $err.InvocationInfo) { $report.Add([string]$err.InvocationInfo.PositionMessage) }
    $report.Add([string]$err.ScriptStackTrace)
    foreach ($r in $report) { Write-Host $r -ForegroundColor Red; $lines.Add($r) }
    [System.IO.File]::WriteAllLines($OutFile, $lines, $Utf8NoBom)
    exit 1
}

function Get-NormalizedName([string]$Text) {
    return ((($Text -split '\s+') -join ' ').Trim()).ToLowerInvariant()
}

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

Out-Line ('Claude UI dump {0}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'))
Out-Line ''

# Session titles of the running sessions, used to mark matches
$wantedTitles = New-Object System.Collections.Generic.List[string]
$sessDir = Join-Path $env:USERPROFILE '.claude\sessions'
if (Test-Path $sessDir) {
    foreach ($f in @(Get-ChildItem -Path $sessDir -Filter '*.json' -File)) {
        try {
            $json = ConvertFrom-Json ([System.IO.File]::ReadAllText($f.FullName))
            $prop = $json.PSObject.Properties['name']
            if ($prop -and $prop.Value) { $wantedTitles.Add((Get-NormalizedName ([string]$prop.Value))) }
        } catch { }
    }
}
Out-Line ('Session titles from ~/.claude/sessions: {0}' -f $wantedTitles.Count)

# Claude Desktop = claude.exe with a main window (the Claude Code CLI has none)
$desktop = @(Get-Process -Name 'claude' -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero })
if ($desktop.Count -eq 0) {
    Out-Line 'No visible Claude Desktop window found. Open Claude Desktop (minimized is ok, tray is not).'
    [System.IO.File]::WriteAllLines($OutFile, $lines, $Utf8NoBom)
    exit 1
}
$proc = $desktop[0]
Out-Line ('Window: "{0}"  PID {1}' -f $proc.MainWindowTitle, $proc.Id)
Out-Line ''

$root = [System.Windows.Automation.AutomationElement]::FromHandle($proc.MainWindowHandle)
$scope = [System.Windows.Automation.TreeScope]::Descendants
$cond = [System.Windows.Automation.Condition]::TrueCondition

# Electron builds the accessibility tree only on demand - read several times, take the largest
$best = $null
$bestCount = -1
for ($i = 1; $i -le $Attempts; $i++) {
    $all = $root.FindAll($scope, $cond)
    Out-Line ('Pass {0}: {1} elements' -f $i, $all.Count)
    if ($all.Count -gt $bestCount) { $best = $all; $bestCount = $all.Count }
    if ($i -lt $Attempts) { Start-Sleep -Milliseconds 1000 }
}
Out-Line ''

$invokeId = [System.Windows.Automation.InvokePattern]::Pattern.Id
$selectId = [System.Windows.Automation.SelectionItemPattern]::Pattern.Id
$codeNames = @('code', 'claude code')
$titleHits = @{}
$codeHits = 0
$rows = New-Object System.Collections.Generic.List[string]

foreach ($el in $best) {
    try {
        $name = [string]$el.Current.Name
        if ($name.Trim() -ne '') {
            $patternIds = @($el.GetSupportedPatterns() | ForEach-Object { $_.Id })
            $hasInvoke = $patternIds -contains $invokeId
            $hasSelect = $patternIds -contains $selectId
            if ($hasInvoke -or $hasSelect) {
                $selected = ''
                if ($hasSelect) {
                    $sp = $el.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)
                    if ($sp.Current.IsSelected) { $selected = ' [selected]' }
                }
                $norm = Get-NormalizedName $name
                $mark = ''
                # When Code is inactive, Desktop appends a status badge, e.g. "Code, wartet auf deine Eingabe"
                # (German UI: "Code, waiting for your input") - so compare only the part before the comma
                $beforeBadge = Get-NormalizedName (($name -split ',')[0])
                if ($codeNames -contains $beforeBadge) { $mark = '  <== Code toggle?'; $codeHits++ }
                if ($wantedTitles.Contains($norm)) {
                    $mark = '  <== Session title'
                    if ($titleHits.ContainsKey($norm)) { $titleHits[$norm]++ } else { $titleHits[$norm] = 1 }
                }
                $kind = ([string]$el.Current.ControlType.ProgrammaticName) -replace '^ControlType\.', ''
                $pattern = 'Invoke'
                if ($hasInvoke -and $hasSelect) { $pattern = 'Invoke+Select' } elseif ($hasSelect) { $pattern = 'Select' }
                $short = $name
                if ($short.Length -gt 90) { $short = $short.Substring(0, 87) + '...' }
                $rows.Add(('  {0,-12} {1,-13} "{2}"{3}{4}' -f $kind, $pattern, $short, $selected, $mark))
            }
        }
    } catch { }
}

Out-Line ('=== Clickable elements with names ({0}) ===' -f $rows.Count)
foreach ($row in $rows) { Out-Line $row }
Out-Line ''
Out-Line '=== Summary ==='
Out-Line ('Code toggle candidates: {0}' -f $codeHits)
foreach ($t in $wantedTitles) {
    $n = 0
    if ($titleHits.ContainsKey($t)) { $n = $titleHits[$t] }
    $verdict = 'not found (Claude may not be showing the Code view)'
    if ($n -eq 1) { $verdict = 'unique - Select in sidebar should work' }
    if ($n -gt 1) { $verdict = 'ambiguous - widget does not click' }
    Out-Line ('  "{0}": {1}x, {2}' -f $t, $n, $verdict)
}

[System.IO.File]::WriteAllLines($OutFile, $lines, $Utf8NoBom)
Write-Host ''
Write-Host ('Saved: {0}' -f $OutFile) -ForegroundColor Green
