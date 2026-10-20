# Decisions and findings

As of 18 Oct 2026. Verified against Claude Desktop 2.42304 (Store version, German UI), Claude Code 2.1.299, Windows 11.

## Principles

- **Passive instead of hooks.** The widget only reads `~/.claude/sessions/<pid>.json` and the end of the transcript. No hooks, no changes to `settings.json`, no credentials, no network.
- **Actions that write or trigger something only on click:** focus a window, open a Desktop session via link (can be turned off), select the sidebar entry via UI Automation (option, off), terminate orphaned processes (with confirmation), create the autostart shortcut (option).
- **No child processes.** Since 0.3.1 the widget does not start any other programs: the autostart shortcut and the deep link go through Windows Shell APIs in its own process (see [Antivirus](#antivirus)).
- **Fork identity:** product name `AI Code Session Widget`, identifier `io.github.migoldschmidt.ai-code-session-widget`, fixed MSI upgrade code `ce6562a5-104e-4f94-a74c-812b5a4b64c9`, publisher `Michael Goldschmidt`. Separate WebView2 data folder under `%LOCALAPPDATA%\<Identifier>`, and therefore separate LocalStorage; if the identifier changes, settings start from scratch. The upgrade code stays the same so that an MSI install replaces an older fork version. Executable since 0.3.1: `AI Code Session Widget.exe` (`mainBinaryName`; the Cargo package remains `session-widget`).
- **App icon and logo:** own logo (face between curly braces, blue gradient), sources in `assets/logo/`: `logo.svg` (transparent), `app-icon.svg`/`app-icon.png` (on a dark, rounded background) and the generator script `build_logo.py`. `src-tauri/icons/*` are generated from `app-icon.png` (exe, desktop and Start menu shortcut, installer, "Installed apps"); regenerate with `npm run tauri icon assets/logo/app-icon.png`. `src/public/logo.png` is the transparent logo for tray and taskbar.
- **Single instance:** A second launch (desktop icon, Start menu, autostart) brings the running widget to the front, also from the tray, and exits before opening a window. Official plugin `tauri-plugin-single-instance` (named mutex and hidden message window, local only). Only active in release builds, so that `tauri dev` runs alongside the installed widget.
- **License:** MIT, original © Kasper Legarth, fork changes © Michael Goldschmidt. The logo is part of the fork changes and is also MIT-licensed; the Codex mark has its own MIT license (LobeHub).
- **Installer without downloading WebView2:** `webviewInstallMode: skip`. Tauri's default would have embedded a hidden PowerShell in the MSI that downloads WebView2 from the internet if it is missing. Windows 11 ships with WebView2; without WebView2 the app does not start.

## What the session file provides

`~/.claude/sessions/<pid>.json` of a Desktop Code session (values shortened):

| Field | Example | Use in the fork |
|---|---|---|
| `name` | `Rework login form` | Title on the card; matches the sidebar title in Claude Desktop exactly |
| `status` | `idle` / `busy` / `waiting` | Upstream status logic |
| `waitingFor` | `permission prompt` | when a permission prompt is open, together with `status: "waiting"` |
| `statusUpdatedAt` | `1792351181525` (ms) | Initial value for "time in state" |
| `hostSessionId` | `local_0a1b2c3d-…` | Desktop's own ID → deep link; marks Desktop sessions |
| `entrypoint` | `claude-desktop` | (not used, alternative to `hostSessionId`) |
| `procStart` | Windows FILETIME, e.g. `"134368033234460409"` | Protection against reused PIDs: a session only counts as active if the process under the PID was started at the same time (±5 s) |
| other | `startedAt`, `version`, `kind`, `peerProtocol`, `peerFeatures`, `pidDomain`, `messagingSocketPath`, `nameSince`, `updatedAt` | – |

Claude Desktop starts the CLI (`%APPDATA%\Claude\claude-code\<version>\…\claude.exe`) directly as a child process of `Claude.exe`. Desktop sessions therefore appear just like terminal sessions. Cowork sessions run in a VM and do not show up.

**Permission prompts** are detected reliably: during the prompt Claude Code writes `status: "waiting"` and `waitingFor: "permission prompt"`, and the widget shows "Waiting for input · Needs permission: Bash". An additional hook is not needed.

## Status and sorting

| Backend | Display | Color | Filter chip |
|---|---|---|---|
| NeedsInput (permission or AskUserQuestion) | Waiting for input | red | Waiting |
| Waiting, last reply ends with "?" | Question (upstream: "Waiting") | blue | Question |
| Working | Working | green | Working |
| Waiting | Idle | gray | Idle |

- Priority: Waiting > Question > Working > Idle. "Asked a question" (AskUserQuestion) counts as Waiting because it blocks.
- **Deviation from the plan:** Within a status, the **most recent** status change is at the top (plan: longest in state first), changed on request.
- "Time in state" is measured by the frontend; when a session is first seen, `statusUpdatedAt` serves as the initial value. After a widget restart without `statusUpdatedAt`, the time starts over.
- The sort button cycles Status → Title → Folder. Plan item 15 ("folder, then title") is "Folder"; "Title" was added because the title is now the main line of the card.
- **Background commands:** Idle sessions with a shell still running under their process (bash, sh, zsh, pwsh, powershell; not `cmd.exe`, because Claude Code starts `npx` MCP servers via `cmd /c`) show "Background command running" and a green ring around the gray dot. Status, filter and sorting remain Idle (`background.rs`). Assumption: Claude Code does not keep a persistent shell open per session; otherwise an Idle session without a background command would show the hint as well (test 19).
- **Time in status:** Waiting, Question and Idle show the duration in the state ("Idle · 12 min"), Working does not. Idle for 60 min or more (`STALE_IDLE_MS`) is dimmed; hovering removes the dimming.
- With multiple columns, each present status gets a column; if there are not enough columns, the last one collects the rest. If more columns fit than there are statuses, the status with the most cards per column gets the remaining columns (never more than it has cards) and shows its cards side by side, row by row (`allocateStatusColumns`).

## Clicking a session

1. **Desktop session:** open `claude://claude.ai/epitaxy/<hostSessionId>`. Claude Desktop jumps directly into the session, also from the Chat/Cowork view and from the tray. **Undocumented**, verified manually (18 Oct 2026, again on 19 Oct 2026 with 0.3.1). Officially documented are only `claude://claude.ai/new`, `…/chat/{id}`, `…/project/{id}`, `claude://code/new` and `claude://cowork/new`.
2. Then focus the window of the parent process and outline it in orange for 3 s (upstream).
3. Only if the link is off or fails and "Select in sidebar (fallback)" is on: select the sidebar entry via UI Automation.

Discarded:
- `claude://code/<id>`: The route exists but is blocked by a feature flag (anthropics/claude-code #85042, #54614, #90290). Tested and removed.
- Desktop session store (`%APPDATA%\Claude\claude-code-sessions\…`) as a second data source: not needed, `name` and `hostSessionId` are already in the session file. One more internal data store would only have added attack surface and dependency.

### UI Automation (fallback)

From `scripts/dump-claude-ui.ps1`:
- The mode switch consists of two RadioButtons: "Chat und Cowork" (Chat and Cowork) and "Code". While Code is not active, Desktop appends a status: "Code, wartet auf deine Eingabe" (Code, waiting for your input). Therefore only the part before the comma is matched.
- If the Code RadioButton is not selected, the widget switches first and then searches.
- Sidebar entries in Code mode are buttons with exactly the session title. Additional text ("<Titel> Gestartet von …", i.e. "<title> Started by …") counts as a partial match if no exact match exists. "<Titel>, Sitzung umbenennen" ("<title>, Rename session", header row) never counts.
- Ambiguous → click nothing. Retry, because Electron only builds the accessibility tree on request.

### Tray fix

If Claude Desktop is closed to the tray, it has no visible window; upstream then kept climbing up to `explorer.exe` and focused an arbitrary Explorer or helper window. Now the search stops at the shell and instead re-shows the largest hidden window with a title of the nearest ancestor.

## Display

- **Upstream bug fixed:** Auto-size measured `#app.scrollHeight`, which because of `min-height: 100%` never falls below the window height – the window could only grow. Now the bottom edge of the lowest visible block counts.
- **Context menu:** In Auto mode the window grows while the menu is open; if it would extend below the taskbar, it moves up and back afterwards. In Manual mode the menu scrolls.
- Order in the window: title "AI Code Session Widget" in the drag bar (instead of the grip, in both views), filter bar, sessions, system status (orphaned processes, CPU/RAM/disk).
- Card, standard: title, detail line (activity · folder), status. Compact: folder + status, below that title · activity.

## Taskbar

- **Tray icon:** mascot in the pose of the most urgent status (rendered from the widget, zoom 1.2), status dot at the bottom left, tooltip with all counts. A single click hides/shows the widget, a double click brings it up via the tray icon (it stays there at the taskbar until it is dragged).
- **Taskbar button:** mascot instead of the app icon, overlay badge with the number of sessions that are waiting (Waiting + Question).
- **Without sessions:** tray and taskbar button show the logo (`src/public/logo.png`) without dot and badge.
- Optional custom tray image under `src/public/tray-mascot.png` (git-ignored, because third-party images often have no clear license). Takes precedence in the tray, also over the logo.
- Required for this: Tauri feature `tray-icon`, CSP `img-src 'self' blob:` (the mascot SVG is rendered via a blob URL), window permissions show/hide/focus/position/icon/overlay.

## Antivirus

Version 0.3.0: After right-click → "Start with Windows", a behavior-based antivirus removed the installed app. The cause was very likely the implementation at the time: an unsigned, unknown program starts a hidden `powershell.exe -ExecutionPolicy Bypass` and uses it to create a shortcut in the Startup folder – the typical pattern malware uses to establish persistence.

Since 0.3.1:
- Autostart writes the `.lnk` in its own process via the shell object `IShellLinkW` + `IPersistFile`, like Explorer does. Location and name stay the same.
- Deep link via `ShellExecuteW` instead of `explorer.exe <url>` (launching via `explorer.exe` counts as indirect execution, also a malware pattern).
- As a result, the widget no longer starts any child processes.

What remains and may still be assessed by behavior detection: entry in the Startup folder (only on click), bringing windows of other processes to the foreground (`AttachThreadInput`, `SetForegroundWindow`), frame window over other applications' windows, UI Automation (finding the matching window by title; sidebar selection as an option), process list every 2 s, Restart Manager for Codex. All of this is upstream behavior except autostart and sidebar selection.

In a corporate environment: do not disable or bypass the antivirus yourself. Get an exception (hash or path) through IT; long term, sign the installers and the exe with a company certificate. Unsigned builds get a new hash with every new build.

## Tooling notes

- Windows PowerShell 5.1 threw "Argument types do not match" in a `foreach` loop with `continue` and many iterations. The scripts therefore avoid `continue`/`break` in large loops.
- `node --test` only runs TypeScript without a flag from Node 22.18 on; CI therefore runs on Node 22.

## Backlog

- Sound on status change to Waiting/Question.
- Upstream PRs: auto-size fix, context menu fix.
- Watch feature request anthropics/claude-code #89531 (official done/waiting status in Desktop).
- Check the `epitaxy` deep link after Desktop updates; if it goes away, turn on "Select in sidebar (fallback)".
