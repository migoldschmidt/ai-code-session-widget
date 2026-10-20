<#
.SYNOPSIS
    Phase 0, item 4/18: For each running Claude Code session, shows which window
    the widget would focus on click - without starting the widget.

.DESCRIPTION
    Read-only. Reads ~/.claude/sessions/*.json, the process list and the visible
    top-level windows and replicates the upstream logic from src-tauri/src/focus.rs:

      1. Starting at the session PID, walk up the parent chain (max. 10 steps) until a
         process owns a visible window (IsWindowVisible)  -> focus target.
      2. If the target has several visible windows, upstream uses UI Automation to pick
         the window whose tree contains the folder name (hint), otherwise the first one.
      3. From the target, continue upwards to a window with a real size -> frame target.

    Output is also written to .\logs\focus-diagnose-<time>.txt.
    The file contains project paths and session names - review before sharing.

    v2: no continue/break in loops (Windows PowerShell 5.1 threw
    "Argument types do not match" there); on errors, line and stack trace
    are printed and written to the log.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\diagnose-focus.ps1
#>
[CmdletBinding()]
param(
    [int]$MaxDepth = 10
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$LogDir = Join-Path $PSScriptRoot 'logs'
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$OutFile = Join-Path $LogDir ('focus-diagnose-{0}.txt' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$Utf8NoBom = New-Object System.Text.UTF8Encoding $false
$lines = New-Object System.Collections.Generic.List[string]

# Any unhandled error: message, line and stack trace to console and log.
trap {
    $err = $_
    $report = New-Object System.Collections.Generic.List[string]
    $report.Add('')
    $report.Add(('ERROR: {0}' -f $err.Exception.Message))
    $report.Add(('Type:  {0}' -f $err.Exception.GetType().FullName))
    if ($null -ne $err.InvocationInfo) { $report.Add([string]$err.InvocationInfo.PositionMessage) }
    $report.Add([string]$err.ScriptStackTrace)
    foreach ($r in $report) { Write-Host $r -ForegroundColor Red; $lines.Add($r) }
    [System.IO.File]::WriteAllLines($OutFile, $lines, $Utf8NoBom)
    Write-Host ('Log with error: {0}' -f $OutFile)
    exit 1
}

function Out-Line([string]$Text = '') {
    Write-Host $Text
    $lines.Add($Text)
}

function Get-Prop($Obj, [string]$Name) {
    $prop = $Obj.PSObject.Properties[$Name]
    $value = $null
    if ($prop) { $value = $prop.Value }
    return $value
}

if (-not ('SessionWidgetDiag.WinEnum' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace SessionWidgetDiag {
    public class WinInfo {
        public long Handle;
        public uint Pid;
        public string Title;
        public string ClassName;
        public int Width;
        public int Height;
        public bool Iconic;
    }

    public static class WinEnum {
        private delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);

#pragma warning disable 649
        [StructLayout(LayoutKind.Sequential)]
        private struct RECT { public int Left, Top, Right, Bottom; }
#pragma warning restore 649

        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
        [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] private static extern bool IsIconic(IntPtr hWnd);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr hWnd, StringBuilder sb, int max);
        [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hWnd, out RECT r);

        public static List<WinInfo> VisibleWindows() {
            List<WinInfo> list = new List<WinInfo>();
            EnumWindows(delegate (IntPtr h, IntPtr l) {
                if (!IsWindowVisible(h)) return true;
                uint pid;
                GetWindowThreadProcessId(h, out pid);
                StringBuilder t = new StringBuilder(512);
                GetWindowText(h, t, t.Capacity);
                StringBuilder c = new StringBuilder(256);
                GetClassName(h, c, c.Capacity);
                RECT r;
                GetWindowRect(h, out r);
                WinInfo w = new WinInfo();
                w.Handle = h.ToInt64();
                w.Pid = pid;
                w.Title = t.ToString();
                w.ClassName = c.ToString();
                w.Width = r.Right - r.Left;
                w.Height = r.Bottom - r.Top;
                w.Iconic = IsIconic(h);
                list.Add(w);
                return true;
            }, IntPtr.Zero);
            return list;
        }
    }
}
'@
}

# Collect processes and windows once
$procs = @{}
foreach ($cim in Get-CimInstance -ClassName Win32_Process) { $procs[[int]$cim.ProcessId] = $cim }

$winByPid = @{}
foreach ($win in [SessionWidgetDiag.WinEnum]::VisibleWindows()) {
    $key = [int]$win.Pid
    if (-not $winByPid.ContainsKey($key)) { $winByPid[$key] = New-Object System.Collections.Generic.List[object] }
    $winByPid[$key].Add($win)
}

# Emits the visible windows of a PID to the pipeline (nothing if there are none).
# Always wrap callers in @().
function Get-Windows([int]$ProcId) {
    if ($winByPid.ContainsKey($ProcId)) { $winByPid[$ProcId] }
}

function Format-Window($Info) {
    $title = '(no title)'
    if (-not [string]::IsNullOrEmpty($Info.Title)) { $title = '"' + $Info.Title + '"' }
    $min = ''
    if ($Info.Iconic) { $min = ' minimized' }
    return ('{0} [{1}] {2}x{3}{4}' -f $title, $Info.ClassName, $Info.Width, $Info.Height, $min)
}

function Format-Proc([int]$ProcId) {
    $text = '{0} (no longer exists)' -f $ProcId
    if ($procs.ContainsKey($ProcId)) {
        $proc = $procs[$ProcId]
        $exe = '(path not readable)'
        if ($proc.ExecutablePath) { $exe = [string]$proc.ExecutablePath }
        $text = '{0,-6} {1,-22} {2}' -f $ProcId, [string]$proc.Name, $exe
    }
    return $text
}

function Write-SessionReport([System.IO.FileInfo]$File) {
    Out-Line ''
    Out-Line ('--- {0}' -f $File.Name)

    $json = $null
    try { $json = ConvertFrom-Json ([System.IO.File]::ReadAllText($File.FullName)) } catch { $json = $null }
    if ($null -eq $json) {
        Out-Line '  not readable (no valid JSON), widget skips the file'
        return
    }

    $sessPidRaw = Get-Prop $json 'pid'
    if ($null -eq $sessPidRaw) {
        Out-Line '  no pid field'
        return
    }
    $sessPid = [int]$sessPidRaw
    $cwd = [string](Get-Prop $json 'cwd')
    $hint = (($cwd -replace '\\', '/') -replace '/$', '').Split('/')[-1]

    Out-Line ('  name:      {0}' -f (Get-Prop $json 'name'))
    Out-Line ('  status:    {0}' -f (Get-Prop $json 'status'))
    Out-Line ('  sessionId: {0}' -f (Get-Prop $json 'sessionId'))
    Out-Line ('  cwd:       {0}   (Hint: {1})' -f $cwd, $hint)
    Out-Line '  All fields:'
    foreach ($prop in @($json.PSObject.Properties)) {
        $val = ''
        if ($null -ne $prop.Value) { $val = [string](ConvertTo-Json -InputObject $prop.Value -Compress -Depth 3) }
        if ($val.Length -gt 120) { $val = $val.Substring(0, 117) + '...' }
        Out-Line ('    {0,-20} {1}' -f $prop.Name, $val)
    }

    if (-not $procs.ContainsKey($sessPid)) {
        Out-Line ('  PID {0} is not running -> widget hides the session.' -f $sessPid)
        return
    }

    # Parent chain like resolve_focus_pid (0..=MaxDepth), without break
    $chain = New-Object System.Collections.Generic.List[int]
    $cur = $sessPid
    $steps = 0
    $more = $true
    while ($more -and ($steps -le $MaxDepth) -and $procs.ContainsKey($cur)) {
        $chain.Add($cur)
        $parent = [int]$procs[$cur].ParentProcessId
        if (($parent -eq 0) -or ($parent -eq $cur)) { $more = $false } else { $cur = $parent }
        $steps = $steps + 1
    }

    Out-Line '  Process chain (session -> parents):'
    $targetIdx = -1
    for ($i = 0; $i -lt $chain.Count; $i++) {
        $cid = $chain[$i]
        $wins = @(Get-Windows $cid)
        Out-Line ('    {0}  Windows: {1}' -f (Format-Proc $cid), $wins.Count)
        foreach ($wi in $wins) { Out-Line ('           ' + (Format-Window $wi)) }
        if (($targetIdx -lt 0) -and ($wins.Count -gt 0)) { $targetIdx = $i }
    }

    if ($targetIdx -lt 0) {
        Out-Line '  => No process in the chain has a visible window: click does nothing.'
        return
    }

    $targetPid = $chain[$targetIdx]
    $targetWins = @(Get-Windows $targetPid)
    Out-Line ('  => Focus target: {0}' -f (Format-Proc $targetPid))
    if ($targetWins.Count -gt 1) {
        Out-Line ('     {0} visible windows -> upstream searches via UI Automation for "{1}", otherwise first window:' -f $targetWins.Count, $hint)
        Out-Line ('     first window = ' + (Format-Window $targetWins[0]))
    } else {
        Out-Line ('     Window: ' + (Format-Window $targetWins[0]))
    }

    $frameIdx = -1
    for ($i = $targetIdx; $i -lt $chain.Count; $i++) {
        if ($frameIdx -lt 0) {
            $real = @(Get-Windows ($chain[$i]) | Where-Object { ($_.Width -gt 0) -and ($_.Height -gt 0) })
            if ($real.Count -gt 0) { $frameIdx = $i }
        }
    }
    if ($frameIdx -ge 0) {
        Out-Line ('  => Frame target: {0}' -f (Format-Proc ($chain[$frameIdx])))
    } else {
        Out-Line '  => Frame target: none with a real size, no frame is drawn.'
    }

    if ([string]$procs[$targetPid].Name -ieq 'explorer.exe') {
        Out-Line '  !! Target is explorer.exe: click would focus taskbar/desktop instead of the app.'
    }
}

Out-Line ('Focus diagnosis {0}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'))
Out-Line ''

$claudeDir = Join-Path $env:USERPROFILE '.claude'
$sessionsDir = Join-Path $claudeDir 'sessions'
if ($env:CLAUDE_CONFIG_DIR) {
    Out-Line ('WARNING: CLAUDE_CONFIG_DIR = {0}. The widget still reads only {1}.' -f $env:CLAUDE_CONFIG_DIR, $claudeDir)
    Out-Line ''
}

# Overview: claude.exe processes with visible windows (desktop app vs. Claude Code CLI).
# Filter beforehand instead of skipping inside the loop.
Out-Line '=== claude.exe processes with visible windows ==='
$claudeWithWin = @(
    $procs.Values |
        Where-Object { ([string]$_.Name -ieq 'claude.exe') -and $winByPid.ContainsKey([int]$_.ProcessId) } |
        Sort-Object { [int]$_.ProcessId }
)
if ($claudeWithWin.Count -eq 0) {
    Out-Line '  none (Claude Desktop not open or closed to the tray)'
}
foreach ($cp in $claudeWithWin) {
    $cpId = [int]$cp.ProcessId
    Out-Line ('  ' + (Format-Proc $cpId))
    foreach ($wi in @(Get-Windows $cpId)) { Out-Line ('           Window: ' + (Format-Window $wi)) }
}
$claudeAll = @($procs.Values | Where-Object { [string]$_.Name -ieq 'claude.exe' }).Count
Out-Line ('  ({0} claude.exe processes in total)' -f $claudeAll)
Out-Line ''

Out-Line ('=== Sessions in {0} ===' -f $sessionsDir)
if (-not (Test-Path $sessionsDir)) {
    Out-Line '  Folder does not exist.'
} else {
    $files = @(Get-ChildItem -Path $sessionsDir -Filter '*.json' -File)
    if ($files.Count -eq 0) { Out-Line '  no session files.' }
    foreach ($f in $files) { Write-SessionReport $f }
}

[System.IO.File]::WriteAllLines($OutFile, $lines, $Utf8NoBom)
Write-Host ''
Write-Host ('Saved: {0}' -f $OutFile) -ForegroundColor Green
