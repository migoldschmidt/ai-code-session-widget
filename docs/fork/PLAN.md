# Session widget fork – implementation plan

Base: https://github.com/kasperlegarth/ai-session-widget (MIT, Tauri 2 + Rust + Vanilla TS)
Goal: Windows widget for Claude Code session status with filter, sorting, compact mode and prepared Desktop session selection.
Way of working: preparation locally in Cowork, later push to own GitHub repo. Upstream PRs only after that.

## Decisions made

| Topic | Decision |
|---|---|
| Fork vs. rebuild | Fork, keep upstream as a remote |
| Status order | Waiting > Question > Working > Idle |
| UI Automation for the Desktop sidebar | Build in, behind a setting, default off |
| Settings storage location | LocalStorage in the WebView |
| Compact mode | Yes |
| Resizable / scrolling | Already present in upstream, only verify |
| Upstream PRs | Later |

## Phase 0 – Foundation

1. Clone the repo, set the upstream remote (`git remote add upstream …`)
2. Check the toolchain: Rust (rustup, MSVC target), Visual Studio Build Tools with C++ workload, Node 20+, WebView2 Runtime
3. Build and start unmodified upstream: `npm install`, `npm run tauri dev`
4. Reference test: resizing, scrolling with many sessions, click on a session in the Desktop Code tab (does it focus `Claude.exe`?). Record the result.
5. Code overview: `src/main.ts` (rendering, state), `src/index.html`, `src/style.css`, `src-tauri/src/model.rs` (status logic), `src-tauri/src/focus.rs` (Win32/UIA focus), `tauri.conf.json`
6. Own product name and identifier in `tauri.conf.json`, `Cargo.toml`, `package.json`, so that fork and original can be installed side by side

## Phase 1 – Compact mode

7. Second density level: smaller status icons, single-line entries, reduced spacing
8. Toggle in the right-click context menu (next to theme and always-on-top)
9. Persist the state in LocalStorage, apply it on startup
10. Window auto-resize takes the selected density into account

## Phase 2 – Filter and sorting

11. Filter bar above the list: toggle chips for Waiting, Question, Working, Idle; multiple selection; count per status on the chip
12. Persist the filter selection in LocalStorage
13. Sorting by status: priority order Waiting > Question > Working > Idle
14. Secondary criterion for equal status: time in the current state, longest first
15. Sorting by name: project folder, then session title; toggle status/name; direction ascending/descending
16. Persist the sort setting in LocalStorage
17. Unit tests for filter and sort logic (pure functions in TS, testable without Tauri)

## Phase 3 – Click behavior

18. Verify the existing window focus on `Claude.exe` (result from item 4)
19. Deep link preparation: on click, additionally open `claude://code/{session-id}`, session ID from the JSONL file name. Behind setting "Try deep link", default off. The route exists in Claude Desktop but is disabled by a feature flag – see issues anthropics/claude-code #85042, #54614, #90290
20. UI Automation fallback: via Windows UIA (`Win32_UI_Accessibility` is already enabled in Cargo.toml) find the sidebar entry in Claude Desktop by session title and trigger it via `InvokePattern`/`SelectionItemPattern`. Behind setting "Select sidebar via UI Automation", default off. Retry logic, because Electron only builds the accessibility tree when a client connects. If the title is ambiguous: do not click, only focus
21. Settings for items 19 and 20 in the context menu, state in LocalStorage

## Phase 4 – Operation

22. Take over the GitHub Actions workflow for release builds from upstream and adapt it to the fork
23. Autostart option in the context menu (Startup folder shortcut)
24. Keep `cargo test` and `npx tsc --noEmit` green
25. Update the README: fork notice, feature list, settings overview

## Backlog (not part of this plan)

- Waiting time display per session ("waiting for 4 min"), modeled on ticlins-claude-monitor
- Audible notification on status change to Waiting/Question
- Upstream PRs for filter and sorting
- Enable the deep link as soon as Anthropic opens the feature flag

## Deliberately not done

- No network connections (no usage retrieval, no update check, no external assets)
- No access to Claude credentials
- No hooks in `~/.claude/settings.json`
- No write access to session or transcript files

## Notes for Cowork

- Each phase can be tested on its own with `npm run tauri dev`
- Rust changes only in Phase 3 (items 19 and 20); everything else is TS/CSS
- Before Phase 3, wait for the results of items 4 and 18 – they determine how much there is to do there at all
- Build the final `.msi` with `npm run tauri build`
