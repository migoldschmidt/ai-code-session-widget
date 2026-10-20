A small Windows widget that shows every running Claude Code session, including those in Claude Desktop's Code tab. At a glance you see which session is working, which one has finished and which one is waiting for a permission or an answer.

**Install:** download and run either the `.msi` or the `-setup.exe`. Requires Windows 11 (WebView2 is already part of it).

**Features**
- Status per session: Waiting, Question, Working, Idle, with time in status, status filter and sorting by status, title or folder.
- Click a session to open it directly in Claude Desktop.
- Idle sessions with a command still running in the background are marked as such; sessions idle for an hour or more are dimmed.
- Compact view and a multi-column layout by status.
- Tray icon and taskbar badge show how many sessions are waiting for you.
- Optional "Start with Windows".
- Only one widget runs at a time; starting it again brings the running one to the front.

**Privacy and security**
- Only reads Claude Code's local session files.
- No network access, no hooks, no credentials.
- Does not start any other programs.

**Note:** the installers are not signed. On first start Windows SmartScreen asks for confirmation ("More info" → "Run anyway").

Fork of [kasperlegarth/ai-session-widget](https://github.com/kasperlegarth/ai-session-widget) (MIT), details in the [README](https://github.com/{{REPOSITORY}}#readme).
