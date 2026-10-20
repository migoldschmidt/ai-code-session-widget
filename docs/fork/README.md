# Fork documentation – AI Code Session Widget

Fork of [kasperlegarth/ai-session-widget](https://github.com/kasperlegarth/ai-session-widget) (MIT), base `2113a5c` (v0.2.0). Goal: status overview of Claude Code sessions in the Desktop Code tab on Windows, with filter, sorting, compact view and a click that goes straight into the session.

| Document | Content |
|---|---|
| [PLAN.md](PLAN.md) | original implementation plan (Phases 0–4) |
| [DECISIONS.md](DECISIONS.md) | decisions, deviations from the plan, discarded approaches, findings on the session file and Claude Desktop |
| [TESTING.md](TESTING.md) | automated tests, manual test protocol, diagnostic scripts |

## Status

All plan items from Phases 0–4 are implemented, plus extensions that came out of testing (multiple columns by status, tray icon, taskbar badge, deep link via `epitaxy`). The feature list is in the [README](../../README.md#what-this-fork-adds).

## Development

```powershell
npm install
npm run tauri dev                                                     # or:
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\start-widget.ps1   # with log to scripts\logs
npx tsc --noEmit
npm test                                                              # TypeScript unit tests (Node >= 22.18)
cargo test --manifest-path src-tauri/Cargo.toml                       # Rust tests
```

Prerequisites: Rust (MSVC host), Visual Studio Build Tools with the C++ workload and Windows SDK, Node.js ≥ 22.18, WebView2 Runtime.

## Build and install

```powershell
npm run tauri build
```

Output under `src-tauri\target\release\bundle\`: `msi\AI Code Session Widget_<version>_x64_en-US.msi` and `nsis\AI Code Session Widget_<version>_x64-setup.exe`. The installers are not signed; SmartScreen asks for confirmation on first launch. Fork and original can be installed side by side (separate product name, identifier and MSI upgrade code).

"Start with Windows" (right-click menu) only works with the installed version.

## Updating Tauri

The Rust crate `tauri` and the npm packages `@tauri-apps/api`/`@tauri-apps/cli` must be on the same minor version, otherwise `tauri build` aborts ("Found version mismatched Tauri packages"). Both are therefore pinned to `~2.12` and are only bumped together:

```powershell
npm install @tauri-apps/api@~2.13.0 @tauri-apps/cli@~2.13.0
# src-tauri/Cargo.toml: tauri = { version = "~2.13", ... }
cargo update --manifest-path src-tauri/Cargo.toml -p tauri
```

`Cargo.lock` and `package-lock.json` are committed; CI builds with `--locked` and `npm ci` respectively and fails instead of silently updating.

## New version and release

1. Set the same version in three files: `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`; `package-lock.json` and `src-tauri/Cargo.lock` follow (via `npm install` and `cargo build` respectively).
2. Test and commit:

   ```powershell
   cargo test --manifest-path src-tauri/Cargo.toml
   npm test
   git commit -am "Version 0.3.3"
   git push
   ```

3. Create and push the tag:

   ```powershell
   git tag v0.3.3
   git push origin v0.3.3
   ```

4. The workflow `.github/workflows/release.yml` (Actions → "Release", approx. 10–20 min) runs the tests, builds the `.msi` and `-setup.exe` (tauri-action) and creates a draft under **Releases** using the GitHub CLI. If a draft already exists for the tag, only the installers are replaced.
5. The draft is only visible to you (`gh release create --draft` in `release.yml`). The text comes from `.github/release-notes.md`, the title is "AI Code Session Widget <tag>"; add version-specific notes to the draft, test the installers, then **Publish release** (if needed, tick **Set as a pre-release** first).

## Upstream

This repo is a GitHub fork of [kasperlegarth/ai-session-widget](https://github.com/kasperlegarth/ai-session-widget). The fork's start page shows how many commits it is behind the original; **Sync fork** brings them in. To be notified of new upstream releases: on the original repo, **Watch → Custom → Releases** (optionally also **Pull requests**).

Locally, the remote `upstream` points to the original repo.

Fetching new upstream changes:

```powershell
git fetch upstream
git log --oneline HEAD..upstream/main
git merge upstream/main
```

Candidates for upstream PRs: auto-size shrinks again, context menu is no longer cut off (see [DECISIONS.md](DECISIONS.md)).
