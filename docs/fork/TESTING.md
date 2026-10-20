# Tests

## Automated

| Command | Scope | As of 19 Oct 2026 |
|---|---|---|
| `cargo test --manifest-path src-tauri/Cargo.toml` | Rust: upstream tests plus fork (session fields, deep link check, autostart path and natively written shortcut in the temp folder, title matching, shell stop, `procStart` and PID reuse) | 0.3.0: 67 passed, 1 ignored (Codex diagnostics); 0.3.1: green |
| `npm test` | `src/sessionOrder.ts`: filter, sorting, persistence, time in state | 15 passed |
| `npx tsc --noEmit` | Frontend type check | green |

All three run in `.github/workflows/ci.yml`.

## Manual

Start the widget: `.\scripts\start-widget.ps1` (log to `scripts\logs`).

As of 19 Oct 2026: version 0.3.1 installed as MSI and tested on Windows 11 Enterprise, all tests passed.

| No. | Test | Expected |
|---|---|---|
| 1 | Start | From the top: filter bar, sessions, system status. The window adapts to the height and also shrinks again |
| 2 | Right-click → Compact view | small icon; line 1 folder + status, line 2 title · activity |
| 3 | Drag the right edge wider | from approx. 630 px (compact 460 px) columns by status; the last column collects the rest |
| 3b | Two statuses, one with many cards (e.g. 5× Working, 1× Idle), widget very wide (4 columns) | Working occupies 3 columns with cards side by side, Idle 1 column |
| 4 | Restart the widget | Width, compact mode, filter and sorting are kept |
| 5 | Click the "Idle" chip | Idle sessions disappear, chip is faded, counter stays |
| 6 | Click the sort button several times, toggle the arrow | Status → Title → Folder, direction reverses |
| 7 | Right-click with 1 session, widget close to the taskbar | Menu fully visible, widget moves up and back afterwards |
| 8 | Close Claude Desktop to the tray, click a session | Desktop comes back and shows the session |
| 9 | Switch Desktop to Chat, click a Desktop session | Desktop jumps directly into the session (deep link) |
| 9b | "Open in Claude Desktop" off, "Select in sidebar (fallback)" on, test 9 | Desktop switches to Code and selects the session via UI Automation |
| 10 | Trigger a permission prompt in a session | Card red "Waiting for input · Needs permission: …", tray dot red, red badge on the taskbar button |
| 11 | Single-click the tray icon, then again | Widget disappears, reappears at its previous position |
| 12 | Double-click the tray icon | Widget appears directly above the icon and stays open |
| 13 | Installed version: right-click → Start with Windows, sign out/sign in | Widget starts with Windows; antivirus reports nothing (0.3.0 with PowerShell was removed by behavior detection) |
| 14 | Process Explorer or similar: click a session, toggle Start with Windows | Widget does not start a child process (no `powershell.exe`, no `explorer.exe`) |
| 15 | Start without a running session | Drag bar shows "AI Code Session Widget"; tray and taskbar button show the logo, no dot, no badge |
| 16 | Start a session | Tray and taskbar switch to the mascot with status |
| 17 | Installed version: desktop, Start menu, "Installed apps", Task Manager → Details | Logo as icon, name "AI Code Session Widget", process `AI Code Session Widget.exe` |
| 18 | Installed version is running (also hidden in the tray): double-click the desktop icon again | No second widget, no second process; the running widget comes to the front |
| 19 | Session without a running command, finished (Idle) | Status "Idle · N min", **no** background command hint |
| 20 | Have Claude start a dev server or similar in the background, session becomes Idle | "Background command running", green ring around the gray dot; after stopping the server, normal Idle again |
| 21 | Leave a session Idle for over 60 min | Card dimmed, normal on hover |

## Diagnostic scripts

| Script | Purpose |
|---|---|
| `scripts/diagnose-focus.ps1` | per session file all fields, process chain with windows, focus and frame target (replicates `focus.rs`) |
| `scripts/dump-claude-ui.ps1` | clickable UI Automation elements in the Claude Desktop window, marks session titles and the Code switch |
| `scripts/start-widget.ps1` | `npm run tauri dev` with log |

Run each with `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\<name>.ps1`. The logs under `scripts\logs` are git-ignored because they contain local paths and session titles.
