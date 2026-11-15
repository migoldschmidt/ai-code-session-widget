# Tests

## Automated

| Command | Scope | As of 19 Oct 2026 |
|---|---|---|
| `cargo test --manifest-path src-tauri/Cargo.toml` | Rust: upstream tests plus fork (session fields, deep link check, autostart path and natively written shortcut in the temp folder, title matching, shell stop, `procStart` and PID reuse) | 0.3.0: 67 passed, 1 ignored (Codex diagnostics); 0.4.0: 77 passed, 1 ignored (incl. window layer parsing) |
| `npm test` | `src/sessionOrder.ts`: filter, sorting, persistence, time in state; `src/contextMenu.ts` and `src/settings.ts`: menu entries, check marks, saved settings | 0.4.0: 26 passed |
| `npx tsc --noEmit` | Frontend type check | green |

All three run in `.github/workflows/ci.yml`. `.github/workflows/gitleaks.yml` scans the history for secrets on every push (see [README.md](README.md#secret-scan)).

## Manual

Start the widget: `.\scripts\start-widget.ps1` (log to `scripts\logs`).

As of 19 Oct 2026: version 0.3.1 installed as MSI and tested on Windows 11 Enterprise, all tests passed. 0.4.0 (1 Nov 2026) adds tests 22–35: all passed on Windows 11 Enterprise (release build), 23 and 24 also with `scripts\test-win-d.ps1`.

| No. | Test | Expected |
|---|---|---|
| 1 | Start | From the top: filter bar, sessions, system status. The window adapts to the height and also shrinks again |
| 2 | Right-click → View → Compact view | small icon; line 1 folder + status, line 2 title · activity |
| 3 | Drag the right edge wider | from approx. 630 px (compact 460 px) columns by status; the last column collects the rest |
| 3b | Two statuses, one with many cards (e.g. 5× Working, 1× Idle), widget very wide (4 columns) | Working occupies 3 columns with cards side by side, Idle 1 column |
| 4 | Restart the widget | Width, compact mode, filter and sorting are kept |
| 5 | Click the "Idle" chip | Idle sessions disappear, chip is faded, counter stays |
| 6 | Click the sort button several times, toggle the arrow | Status → Title → Folder, direction reverses |
| 7 | Right-click with 1 session, widget close to the taskbar | Native menu opens over the desktop and taskbar, fully visible; the widget does not change size or position |
| 8 | Close Claude Desktop to the tray, click a session | Desktop comes back and shows the session |
| 9 | Switch Desktop to Chat, click a Desktop session | Desktop jumps directly into the session (deep link) |
| 9b | "Open in Claude Desktop" off, "Select in sidebar (fallback)" on, test 9 | Desktop switches to Code and selects the session via UI Automation |
| 10 | Trigger a permission prompt in a session | Card red "Waiting for input · Needs permission: …", tray dot red, red badge on the taskbar button |
| 11 | Single-click the tray icon, with Always on top and with Normal (widget behind other windows, hidden, minimized) | Widget comes to the front at once (no delay) every time, never hides |
| 11b | Pinned to desktop, windows over it: single-click the tray icon, wait | Widget comes to the front, drops back onto the desktop after 5 s (or earlier when another window is clicked) |
| 12 | Double-click the tray icon (also while pinned to the desktop, under open windows) | Widget comes to the front where it is; pinned, it goes back onto the desktop after clicking another window |
| 13 | Installed version: right-click → Window → Start with Windows, sign out/sign in | Widget starts with Windows; antivirus reports nothing (0.3.0 with PowerShell was removed by behavior detection) |
| 14 | Process Explorer or similar: click a session, toggle Start with Windows | Widget does not start a child process (no `powershell.exe`, no `explorer.exe`) |
| 15 | Start without a running session | Filter bar starts with logo + "AI Sessions" (tooltip: full name), widget can be dragged by it; tray and taskbar button show the logo, no dot, no badge |
| 16 | Start a session | Tray and taskbar switch to the mascot with status |
| 17 | Installed version: desktop, Start menu, "Installed apps", Task Manager → Details | Logo as icon, name "AI Code Session Widget", process `AI Code Session Widget.exe` |
| 18 | Installed version is running (also hidden in the tray): double-click the desktop icon again | No second widget, no second process; the running widget comes to the front |
| 19 | Session without a running command, finished (Idle) | Status "Idle · N min", **no** background command hint |
| 20 | Have Claude start a dev server or similar in the background, session becomes Idle | Status "Working", activity "Background command running"; after stopping the server, normal Idle again |
| 21 | Leave a session Idle for over 60 min | Card dimmed, normal on hover |
| 22 | Right-click → Window → Pin to desktop, open other windows over it, click the widget, then another window | Widget stays behind the other windows (after a click on it until another window is activated); no taskbar button, tray icon remains; clicking a session still focuses it |
| 23 | Pinned to desktop: Win+D, wait a few seconds (polls run), Win+D again — or `scripts\test-win-d.ps1` | Widget stays visible on the desktop and is clickable; the second Win+D brings all windows back and the widget is below them again |
| 24 | Always on top (not pinned): Win+D, wait, Win+D again — or `scripts\test-win-d.ps1` | Widget is minimized with the other windows; all come back on the second Win+D, the widget still always on top |
| 25 | Always on top and Pin to desktop off (Normal): Win+D, Win+D again | Widget goes away with the other windows and comes back with them |
| 26 | Pinned to desktop: restart the widget, then turn Pin to desktop off | Starts pinned again; afterwards back to always on top with taskbar button |
| 27 | Right-click → Window → Show in taskbar off, restart, Pin to desktop on and off, then Show in taskbar on | Button gone (tray icon stays), also after restart and after leaving the desktop mode; menu item greyed out while pinned; comes back with the badge |
| 28 | Drag the widget by the title, a free spot of the filter bar and the CPU/RAM/disk bar | Widget moves each time |
| 29 | Pinned to desktop, windows over it: drag the widget by the title, let go, wait | Widget comes above the windows as soon as the mouse is pressed and stays there while moving, also when holding still; drops back onto the desktop about half a second after letting go |
| 30 | Right-click → View → Transparent background, restart | Window panel gone, title and cards readable on the wallpaper; setting kept after restart |
| 31 | Hide the widget, right-click the tray icon, change a setting (e.g. Compact view), right-click the widget | Tray menu shows Bring to front, Hide widget and the full menu; the change applies; both menus show the same check marks |
| 32 | Pinned to desktop, windows open over it: tray icon → Bring to front, then click another window | Widget comes up above all windows, drops back onto the desktop after the click (at the latest after 60 s) |
| 33 | Tray icon → Hide widget, with and without Show in taskbar; then single-click the tray icon | With a taskbar button the widget is minimized (button stays), without it is hidden; the click brings it back each time |
| 34 | Right-click → About & help, then Open repository on GitHub, then Back/Esc | Short help with version; browser opens the repository; session view returns |
| 35 | Light theme (View → Light theme or Windows in light mode) | Cards light grey (#f7f6f2) on a slightly darker ground, no white surfaces |

## Diagnostic scripts

| Script | Purpose |
|---|---|
| `scripts/diagnose-focus.ps1` | per session file all fields, process chain with windows, focus and frame target (replicates `focus.rs`) |
| `scripts/dump-claude-ui.ps1` | clickable UI Automation elements in the Claude Desktop window, marks session titles and the Code switch |
| `scripts/start-widget.ps1` | `npm run tauri dev` with log |
| `scripts/test-win-d.ps1` | presses Win+D twice and reports minimized windows, the desktop's and the widget's z-order (tests 23, 24); **not read-only**, your windows disappear for a few seconds |

Run each with `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\<name>.ps1`. The logs under `scripts\logs` are git-ignored because they contain local paths and session titles.
