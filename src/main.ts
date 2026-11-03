import { invoke } from "@tauri-apps/api/core";
import { Menu } from "@tauri-apps/api/menu";
import { currentMonitor, getCurrentWindow, LogicalSize, PhysicalPosition } from "@tauri-apps/api/window";
import { createMascotElement, mascots, type MascotStatus } from "./mascot";
import { createCodexLogoElement } from "./codex-logo";
import {
  type OrderableSession,
  type SortKey,
  type SortSettings,
  type StatusSince,
  type VisualStatus,
  STATUS_PRIORITY,
  STALE_IDLE_MS,
  allocateStatusColumns,
  countByStatus,
  formatAge,
  filterSessions,
  isVisualStatus,
  lastPathSegment,
  nextSortKey,
  parseSortSettings,
  parseStatusFilter,
  serializeSortSettings,
  serializeStatusFilter,
  sortSessions,
  toEpochMs,
  toggleStatus,
  updateStatusSince,
} from "./sessionOrder";
import { initTray, setTrayMenu, updateStatusIndicators } from "./tray";
import { initAbout, setAboutOpen } from "./about";
import { type MenuActions, type MenuState, contextMenuEntries, trayMenuEntries } from "./contextMenu";
import { loadChoice, loadFlag, saveFlag } from "./settings";
import {
  bringToFront,
  getWindowLayer,
  hideWidget,
  initWindowLayer,
  isShownInTaskbar,
  onTrayClick,
  setWindowLayer,
  toggleTaskbarButton,
} from "./windowLayer";

const WINDOW_WIDTH = 340;
const appEl = document.getElementById("app") as HTMLDivElement;
const appWindow = getCurrentWindow();

type ThemePreference = "light" | "dark" | "system";
const THEME_STORAGE_KEY = "theme-preference";

let themePreference: ThemePreference = loadChoice(THEME_STORAGE_KEY, ["light", "dark", "system"], "system");

type SizeMode = "auto" | "manual";
const SIZE_MODE_STORAGE_KEY = "size-mode";

let sizeMode: SizeMode = loadChoice(SIZE_MODE_STORAGE_KEY, ["auto", "manual"], "auto");
const systemDarkQuery = window.matchMedia("(prefers-color-scheme: dark)");

function applyTheme(): void {
  const resolved = themePreference === "system" ? (systemDarkQuery.matches ? "dark" : "light") : themePreference;
  document.documentElement.setAttribute("data-theme", resolved);
}

applyTheme();
systemDarkQuery.addEventListener("change", () => {
  if (themePreference === "system") applyTheme();
});

// Card density: "normal" is the upstream two-/three-line card with the big
// mascot, "compact" is a single line per session with a small icon (see the
// body[data-density="compact"] rules in style.css). Applied before the first
// resize so auto size measures the right layout from the start.
type Density = "normal" | "compact";
const DENSITY_STORAGE_KEY = "density";

let density: Density = loadChoice(DENSITY_STORAGE_KEY, ["normal", "compact"], "normal");

function applyDensity(): void {
  document.body.dataset.density = density;
}

applyDensity();

// Transparent background: only the window panel goes, the cards stay solid
// (see body[data-transparent="on"] in style.css).
const TRANSPARENT_STORAGE_KEY = "transparent-background";
let transparentBackground = loadFlag(TRANSPARENT_STORAGE_KEY, false);

function applyTransparency(): void {
  document.body.dataset.transparent = transparentBackground ? "on" : "off";
}

applyTransparency();

// Natural height of the content. #app has min-height: 100%, so its
// scrollHeight never drops below the current window height — measuring that
// meant the window could grow but never shrink again (e.g. after a session
// ended or when switching to the compact view). Instead: the lowest bottom
// edge of #app's visible in-flow children (sessions, filter bar, orphan
// warning, usage bar, about panel) plus #app's bottom padding. While the
// window is capped to the screen height the session list scrolls; the part
// scrolled out of view counts too, so the window grows back once it fits.
function contentHeight(): number {
  let bottom = 0;
  for (const child of Array.from(appEl.children)) {
    if (!(child instanceof HTMLElement) || child.hidden) continue;
    bottom = Math.max(bottom, child.getBoundingClientRect().bottom);
  }
  if (bottom === 0) return appEl.scrollHeight;
  const paddingBottom = parseFloat(getComputedStyle(appEl).paddingBottom) || 0;
  const list = document.getElementById("session-list");
  const listOverflow = list && !list.hidden ? Math.max(0, list.scrollHeight - list.clientHeight) : 0;
  return Math.ceil(bottom + paddingBottom + listOverflow);
}

// Auto size never grows the window past the monitor's work area: with many
// sessions it would otherwise reach off the screen (taskbar side included).
async function maxWindowHeight(): Promise<number> {
  const monitor = await currentMonitor();
  if (!monitor) return Infinity;
  return Math.floor(monitor.workArea.size.height / monitor.scaleFactor);
}

// Window width is the user's choice in both size modes: wider windows show
// the session cards in several columns (see #session-list in style.css).
// Auto size only fits the height; manual size keeps both. Both are restored
// on the next start.
const MIN_WINDOW_WIDTH = 260; // keep in sync with minWidth in tauri.conf.json
const WINDOW_SIZE_STORAGE_KEY = "window-size";

interface StoredWindowSize {
  width: number;
  height: number;
}

function loadWindowSize(): StoredWindowSize | null {
  try {
    const raw = localStorage.getItem(WINDOW_SIZE_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<StoredWindowSize>;
    if (
      typeof parsed.width === "number" &&
      typeof parsed.height === "number" &&
      parsed.width >= MIN_WINDOW_WIDTH &&
      parsed.height >= 60
    ) {
      return { width: Math.round(parsed.width), height: Math.round(parsed.height) };
    }
  } catch {
    // malformed value - fall back to the defaults
  }
  return null;
}

const storedWindowSize = loadWindowSize();
let preferredWidth = storedWindowSize ? storedWindowSize.width : WINDOW_WIDTH;
let manualHeight = storedWindowSize ? storedWindowSize.height : 480;
let initialSizingDone = false;

function saveWindowSize(): void {
  localStorage.setItem(WINDOW_SIZE_STORAGE_KEY, JSON.stringify({ width: preferredWidth, height: manualHeight }));
}

// Auto-fits the window height to however many session cards are showing.
// Skipped in "manual" size mode, where the user has taken over sizing via
// the OS resize border (see applySizeMode) and we must not fight them.
// Only touches the window when the size really changes: this runs on every
// poll, and a SetWindowPos every 2 s ends Windows' "Show desktop" state, so
// the second Win+D showed the desktop again instead of restoring the windows.
async function resizeWindowToContent(): Promise<void> {
  if (sizeMode === "manual") return;
  try {
    const natural = contentHeight();
    const maxHeight = await maxWindowHeight();
    document.body.dataset.capped = natural > maxHeight ? "on" : "off";
    const wanted = new LogicalSize(preferredWidth, Math.min(natural, maxHeight));
    const current = (await appWindow.innerSize()).toLogical(await appWindow.scaleFactor());
    if (Math.abs(current.width - wanted.width) < 1 && Math.abs(current.height - wanted.height) < 1) return;
    await appWindow.setSize(wanted);
    await keepWindowOnScreen();
  } catch (err) {
    console.error("Failed to resize window to content", err);
  }
}

// If the window now reaches below the monitor's work area (e.g. more
// sessions in a widget near the taskbar), move it up so everything stays
// visible.
async function keepWindowOnScreen(): Promise<void> {
  const monitor = await currentMonitor();
  if (!monitor) return;
  const pos = await appWindow.outerPosition();
  const size = await appWindow.outerSize();
  const workTop = monitor.workArea.position.y;
  const workBottom = workTop + monitor.workArea.size.height;

  if (pos.y + size.height > workBottom) {
    const y = Math.max(workTop, workBottom - size.height);
    await appWindow.setPosition(new PhysicalPosition(pos.x, y));
  }
}

// tauri.conf.json ships with resizable: false and decorations: false, so
// there's no OS-drawn resize border to grab even once resizable is true —
// decorations:false removes the non-client area Windows would otherwise
// hit-test for resize cursors. The #resize-handles strips stand in for it via
// startResizeDragging(): all of them in manual mode, only the left/right ones
// in auto mode (height is ours there, width is the user's).
async function applySizeMode(): Promise<void> {
  document.body.dataset.sizeMode = sizeMode;
  try {
    await appWindow.setResizable(true);
  } catch (err) {
    console.error("Failed to apply size mode", err);
  }
  if (sizeMode === "auto") {
    await resizeWindowToContent();
  }
}

async function applyInitialWindowSize(): Promise<void> {
  if (sizeMode === "manual" && storedWindowSize) {
    try {
      await appWindow.setSize(new LogicalSize(storedWindowSize.width, storedWindowSize.height));
    } catch (err) {
      console.error("Failed to restore window size", err);
    }
  }
  await applySizeMode();
  initialSizingDone = true;
}

void applyInitialWindowSize();

// Remembers widths/heights the user drags to, and re-fits the height in auto
// size once the column count may have changed. Our own setSize calls keep the
// width at preferredWidth, so a width change here always comes from the user.
let refitTimer: number | undefined;
window.addEventListener("resize", () => {
  if (!initialSizingDone) return;
  let changed = false;
  const width = Math.round(window.innerWidth);
  if (Math.abs(width - preferredWidth) > 1) {
    preferredWidth = Math.max(MIN_WINDOW_WIDTH, width);
    changed = true;
  }
  if (sizeMode === "manual") {
    const height = Math.round(window.innerHeight);
    if (Math.abs(height - manualHeight) > 1) {
      manualHeight = height;
      changed = true;
    }
  }
  if (changed) saveWindowSize();
  // The number of status columns depends on the width (layoutStatusColumns),
  // so re-render; in auto size that also re-fits the height.
  window.clearTimeout(refitTimer);
  refitTimer = window.setTimeout(() => rerender(), 150);
});

for (const handle of document.querySelectorAll<HTMLDivElement>(".resize-handle")) {
  const direction = handle.dataset.direction as
    | "North"
    | "South"
    | "East"
    | "West"
    | "NorthEast"
    | "NorthWest"
    | "SouthEast"
    | "SouthWest";
  handle.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    appWindow.startResizeDragging(direction).catch((err) => {
      console.error("Failed to start resize dragging", err);
    });
  });
}


type SessionStatus = "working" | "needsInput" | "waiting";

interface SessionInfo {
  provider: "claude" | "codex";
  pid: number;
  sessionId: string;
  name: string;
  cwd: string;
  status: SessionStatus;
  activity: string | null;
  /** Raw statusUpdatedAt from the session file (Claude only); see sessionOrder.ts::toEpochMs. */
  statusUpdatedAt?: unknown;
  /** Claude Desktop's own id for the session, if any. */
  hostSessionId?: string | null;
  /** Shells still running below the session process (fork); 0 for Codex. */
  backgroundCommands?: number;
}

/** Idle with a command still running: replaces the activity text (fork). */
function backgroundText(session: SessionInfo): string | null {
  const count = session.backgroundCommands ?? 0;
  if (session.status !== "waiting" || count === 0) return null;
  return count === 1 ? "Background command running" : `${count} background commands running`;
}

interface OrphanProcess {
  pid: number;
  name: string;
  cpuPercent: number;
  diskBytes: number;
}

interface ResourceUsage {
  sessionCpuPercent: number;
  totalCpuPercent: number;
  sessionMemoryBytes: number;
  totalMemoryBytes: number;
  sessionDiskBytes: number;
  totalDiskBytes: number;
  orphanedProcesses: OrphanProcess[];
}

interface SessionsPayload {
  sessions: SessionInfo[];
  usage: ResourceUsage;
}

// "waitingAlert" is a frontend-only presentation variant: a plain "waiting"
// session whose last message read like a question gets a distinct color and
// mascot animation from an ordinary finished/idle "waiting" session, even
// though both share the same backend SessionStatus.
function visualStatus(session: SessionInfo): MascotStatus {
  if (session.status === "waiting" && session.activity === "Ended with a question") {
    return "waitingAlert";
  }
  return session.status;
}

const POLL_INTERVAL_MS = 2000;

const listEl = document.getElementById("session-list") as HTMLUListElement;

// The four labels match the filter chips: Waiting (for input) > Question >
// Working > Idle. Upstream called waitingAlert "Waiting", which clashed with
// "Waiting for input".
function statusLabel(status: MascotStatus): string {
  switch (status) {
    case "working":
      return "Working";
    case "needsInput":
      return "Waiting for input";
    case "waitingAlert":
      return "Question";
    case "waiting":
      return "Idle";
  }
}

const usageCpuEl = document.getElementById("usage-cpu") as HTMLSpanElement;
const usageMemEl = document.getElementById("usage-mem") as HTMLSpanElement;
const usageDiskEl = document.getElementById("usage-disk") as HTMLSpanElement;
const orphanWarningEl = document.getElementById("orphan-warning") as HTMLDivElement;
const orphanSummaryEl = document.getElementById("orphan-summary-btn") as HTMLButtonElement;
const orphanListEl = document.getElementById("orphan-list") as HTMLUListElement;

function formatPercent(value: number): string {
  return `${Math.round(value)}%`;
}

function formatBytes(bytes: number): string {
  const gb = bytes / 1024 ** 3;
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  return `${Math.round(bytes / 1024 ** 2)} MB`;
}

function formatBytesPerSec(bytesPerPoll: number): string {
  const bytesPerSec = bytesPerPoll / (POLL_INTERVAL_MS / 1000);
  const mbPerSec = bytesPerSec / 1024 ** 2;
  if (mbPerSec >= 1) return `${mbPerSec.toFixed(1)} MB/s`;
  return `${Math.round(bytesPerSec / 1024)} KB/s`;
}

function renderUsage(usage: ResourceUsage): void {
  usageCpuEl.textContent = `${formatPercent(usage.sessionCpuPercent)} / ${formatPercent(usage.totalCpuPercent)}`;
  const memPercent = usage.totalMemoryBytes > 0 ? (usage.sessionMemoryBytes / usage.totalMemoryBytes) * 100 : 0;
  usageMemEl.textContent = `${formatBytes(usage.sessionMemoryBytes)} (${formatPercent(memPercent)})`;
  const diskPercent = usage.totalDiskBytes > 0 ? (usage.sessionDiskBytes / usage.totalDiskBytes) * 100 : 0;
  usageDiskEl.textContent = `${formatBytesPerSec(usage.sessionDiskBytes)} (${formatPercent(diskPercent)})`;
  renderOrphanWarning(usage.orphanedProcesses);
}

// A PID must show up in this many consecutive polls before it's trusted
// enough to surface, and vanish for this many consecutive polls before it's
// cleared — otherwise a single noisy or missed sample makes the banner flap
// on and off (see usage.rs::find_runaway_orphans for the underlying
// per-poll detection this smooths over).
const ORPHAN_CONFIRM_POLLS = 3;
const ORPHAN_CLEAR_POLLS = 2;

interface TrackedOrphan {
  process: OrphanProcess;
  presentStreak: number;
  absentStreak: number;
  confirmed: boolean;
}

const trackedOrphans = new Map<number, TrackedOrphan>();

function updateOrphanTracking(orphans: OrphanProcess[]): OrphanProcess[] {
  const seenPids = new Set(orphans.map((o) => o.pid));

  for (const orphan of orphans) {
    const tracked = trackedOrphans.get(orphan.pid);
    if (tracked) {
      tracked.process = orphan;
      tracked.absentStreak = 0;
      tracked.presentStreak += 1;
      if (tracked.presentStreak >= ORPHAN_CONFIRM_POLLS) tracked.confirmed = true;
    } else {
      trackedOrphans.set(orphan.pid, {
        process: orphan,
        presentStreak: 1,
        absentStreak: 0,
        confirmed: ORPHAN_CONFIRM_POLLS <= 1,
      });
    }
  }

  for (const [pid, tracked] of trackedOrphans) {
    if (seenPids.has(pid)) continue;
    tracked.presentStreak = 0;
    tracked.absentStreak += 1;
    if (tracked.absentStreak >= ORPHAN_CLEAR_POLLS) {
      trackedOrphans.delete(pid);
    }
  }

  return [...trackedOrphans.values()].filter((t) => t.confirmed).map((t) => t.process);
}

// Flags processes left behind by a tool call whose parent shell has already
// exited (e.g. a backgrounded `find /` that timed out and was never killed)
// — see usage.rs::find_runaway_orphans for the detection rule.
let orphanListExpanded = false;

function renderOrphanWarning(rawOrphans: OrphanProcess[]): void {
  const orphans = updateOrphanTracking(rawOrphans);
  if (orphans.length === 0) {
    orphanWarningEl.hidden = true;
    orphanListExpanded = false;
    orphanListEl.hidden = true;
    orphanListEl.replaceChildren();
    return;
  }
  orphanWarningEl.hidden = false;
  const noun = orphans.length === 1 ? "orphaned process" : "orphaned processes";
  orphanSummaryEl.textContent = `⚠ ${orphans.length} ${noun} still running — click for details`;
  orphanSummaryEl.title = orphans
    .map((o) => `${o.name} (PID ${o.pid}, ${Math.round(o.cpuPercent)}% CPU)`)
    .join("\n");
  renderOrphanList(orphans);
}

function renderOrphanList(orphans: OrphanProcess[]): void {
  orphanListEl.hidden = !orphanListExpanded;
  orphanListEl.replaceChildren();
  for (const orphan of orphans) {
    const li = document.createElement("li");
    li.className = "orphan-row";

    const label = document.createElement("span");
    label.className = "orphan-row-label";
    label.textContent = `${orphan.name} (PID ${orphan.pid}) — ${Math.round(orphan.cpuPercent)}% CPU`;
    label.title = label.textContent;

    const killBtn = document.createElement("button");
    killBtn.type = "button";
    killBtn.className = "orphan-kill-btn";
    killBtn.textContent = "End process";
    killBtn.addEventListener("click", () => void killOrphan(orphan));

    li.append(label, killBtn);
    orphanListEl.appendChild(li);
  }
}

orphanSummaryEl.addEventListener("click", () => {
  orphanListExpanded = !orphanListExpanded;
  orphanListEl.hidden = !orphanListExpanded;
  void resizeWindowToContent();
});

// Only ever runs after the user explicitly confirms — the widget is
// otherwise strictly read-only (see README). Re-verified against the live
// pid+name on the Rust side too, in case the pid was recycled since the
// confirm dialog was built.
async function killOrphan(orphan: OrphanProcess): Promise<void> {
  const confirmed = window.confirm(
    `End ${orphan.name} (PID ${orphan.pid})?\n\n` +
      `This process was left behind by a session whose parent shell already exited. ` +
      `Ending it cannot be undone and may lose unsaved work it was doing.`,
  );
  if (!confirmed) return;

  try {
    await invoke("kill_orphan_process", { pid: orphan.pid, name: orphan.name });
    trackedOrphans.delete(orphan.pid);
  } catch (err) {
    console.error("Failed to end orphaned process", err);
    window.alert(`Couldn't end ${orphan.name} (PID ${orphan.pid}):\n${err}`);
  }
  await refresh();
}

interface SessionRow {
  li: HTMLLIElement;
  mascotEl: SVGSVGElement;
  text: HTMLDivElement;
  statusDot: HTMLSpanElement;
  statusText: HTMLSpanElement;
  /** Standard view: session title (falls back to the project folder). */
  title: HTMLSpanElement;
  /** Standard view: activity · project folder. */
  detail: HTMLSpanElement;
  /** Compact view, line 1 (next to the status): project folder. */
  folder: HTMLSpanElement;
  /** Compact view, line 2: session title · activity. */
  compactDetail: HTMLSpanElement;
  provider: SessionInfo["provider"];
  visual: MascotStatus;
  session: SessionInfo;
}

const sessionRows = new Map<string, SessionRow>();
let emptyStateEl: HTMLLIElement | null = null;

function buildSessionRow(session: SessionInfo, visual: MascotStatus): SessionRow {
  const li = document.createElement("li");
  li.className = "session-card";

  const mascotEl = session.provider === "codex"
    ? createCodexLogoElement(visual, "terminal", session.sessionId)
    : createMascotElement();
  if (session.provider === "claude") mascotEl.classList.add("mascot-svg");
  li.appendChild(mascotEl);

  const text = document.createElement("div");
  text.className = "session-text";

  // Standard view (order set in style.css): title, details, status.
  // Compact view (CSS swaps which spans show): folder + status, then title · activity.
  const folder = document.createElement("span");
  folder.className = "session-folder";
  text.appendChild(folder);

  const statusRow = document.createElement("span");
  statusRow.className = "session-status-row";
  const statusDot = document.createElement("span");
  const statusText = document.createElement("span");
  statusRow.append(statusDot, statusText);
  text.appendChild(statusRow);

  const title = document.createElement("span");
  title.className = "session-name";
  text.appendChild(title);

  const detail = document.createElement("span");
  detail.className = "session-detail";
  text.appendChild(detail);

  const compactDetail = document.createElement("span");
  compactDetail.className = "session-compact-detail";
  text.appendChild(compactDetail);

  li.appendChild(text);

  const row: SessionRow = {
    li,
    mascotEl,
    text,
    statusDot,
    statusText,
    title,
    detail,
    folder,
    compactDetail,
    provider: session.provider,
    visual,
    session,
  };

  li.addEventListener("click", () => void activateSession(row.session));

  return row;
}

// Click behaviour switches (context menu > "On session click").
// "Open in Claude Desktop" is on unless switched off; the sidebar selection
// via UI Automation is an opt-in fallback.
const OPEN_IN_DESKTOP_STORAGE_KEY = "click-open-in-desktop";
const SIDEBAR_SELECT_STORAGE_KEY = "click-sidebar-select";
const clickSettings = {
  openInDesktop: loadFlag(OPEN_IN_DESKTOP_STORAGE_KEY, true),
  sidebarSelect: loadFlag(SIDEBAR_SELECT_STORAGE_KEY, false),
};

// Claude Desktop sessions carry hostSessionId ("local_…") in their session
// file; terminal sessions don't.
// 1. Desktop session + "Open in Claude Desktop": open
//    claude://claude.ai/epitaxy/<hostSessionId> — Desktop comes forward and
//    switches to the session, from any view and even from the tray.
// 2. Focus the owning window (upstream behaviour, plus the highlight frame;
//    hint disambiguates between several windows sharing one process id).
//    Only if the link was not used or could not be opened and "Select in
//    sidebar" is on: select the session in Desktop's sidebar via UI
//    Automation (switching Chat/Cowork → Code first).
// Outcomes go to the console (Ctrl+Shift+I in `tauri dev`).
async function activateSession(session: SessionInfo): Promise<void> {
  if (session.pid <= 0) return; // unknown owner or demo row
  const hostSessionId = session.provider === "claude" && session.hostSessionId ? session.hostSessionId : null;

  let openedInDesktop = false;
  if (hostSessionId && clickSettings.openInDesktop) {
    try {
      await invoke("open_desktop_session", { hostSessionId });
      openedInDesktop = true;
      console.info(`open_desktop_session "${session.name}": claude://claude.ai/epitaxy/${hostSessionId}`);
    } catch (err) {
      console.error("open_desktop_session failed", err);
    }
  }

  const sidebarTitle = hostSessionId && !openedInDesktop && clickSettings.sidebarSelect ? session.name : null;
  try {
    const outcome = await invoke<string>("focus_session", {
      pid: session.pid,
      hint: lastPathSegment(session.cwd),
      sidebarTitle,
    });
    console.info(`focus_session "${session.name}": ${outcome}`);
  } catch (err) {
    console.error("focus_session failed", err);
  }
}

// Session cards are patched in place across polls instead of being torn
// down and rebuilt from scratch — the Claude mascot's "working"/
// "waitingAlert" states are SMIL-animated external SVGs whose timeline
// lives on the DOM element itself, so recreating the element every poll
// restarted the animation from frame 0 before it ever finished playing.
// `sessions` is already sorted and filtered (see applyOrdering);
// `totalCount` is the number before filtering, for the empty-state text.
function render(sessions: SessionInfo[], totalCount: number = sessions.length): void {
  if (sessions.length === 0) {
    for (const row of sessionRows.values()) row.li.remove();
    sessionRows.clear();
    if (!emptyStateEl) {
      emptyStateEl = document.createElement("li");
      emptyStateEl.className = "empty-state";
    }
    emptyStateEl.textContent = totalCount === 0 ? "No active sessions" : "No sessions match the filter";
    listEl.appendChild(emptyStateEl);
    layoutStatusColumns([]);
    mascots.prune(new Set());
    return;
  }

  if (emptyStateEl) {
    emptyStateEl.remove();
    emptyStateEl = null;
  }

  const wantedKeys = new Set<string>();

  for (const session of sessions) {
    const key = `${session.provider}:${session.sessionId}`;
    wantedKeys.add(key);
    const visual = visualStatus(session);
    let row = sessionRows.get(key);

    if (!row) {
      row = buildSessionRow(session, visual);
      sessionRows.set(key, row);
    } else {
      row.session = session;
      // Codex's mark bakes status into its markup at construction time
      // (signal glyph, aria-label); Claude's rig is one fixed SVG whose
      // visibility groups update reactively (see mascot.ts::update), so it
      // never needs rebuilding.
      if (row.provider === "codex" && row.visual !== visual) {
        const freshMascot = createCodexLogoElement(visual, "terminal", session.sessionId);
        row.li.replaceChild(freshMascot, row.mascotEl);
        row.mascotEl = freshMascot;
      }
      row.visual = visual;
    }

    // Fork: an idle session with a shell still running shows that instead of
    // its last activity; every status except Working shows how long it has
    // been in it, and an idle session untouched for an hour is dimmed.
    const background = visual === "waiting" ? backgroundText(session) : null;
    const activity = background ?? session.activity;
    const age = Date.now() - (statusSince.get(key)?.since ?? Date.now());
    const stale = visual === "waiting" && !background && age >= STALE_IDLE_MS;
    row.li.classList.toggle("session-stale", stale);
    row.li.classList.toggle("session-background", background !== null);

    // Long titles/details are cut off with an ellipsis; the tooltip has
    // everything, path first.
    row.li.title =
      `${session.cwd}\n${session.name}` +
      (activity ? `\n${activity}` : "");
    const folder = lastPathSegment(session.cwd);
    const titleText = session.name.trim() !== "" ? session.name : folder;
    const joinParts = (parts: string[]): string => parts.filter((part) => part.trim() !== "").join(" · ");
    // Standard view: title, then activity · folder (folder skipped if it already is the title).
    row.title.textContent = titleText;
    row.detail.textContent = joinParts([activity ?? "", titleText !== folder ? folder : ""]);
    // Compact view: folder next to the status, then title · activity.
    row.folder.textContent = folder;
    row.compactDetail.textContent = joinParts([titleText !== folder ? titleText : "", activity ?? ""]);
    row.statusDot.className = `status-dot status-dot-${visual}`;
    row.statusText.className = `session-status status-text-${visual}`;
    row.statusText.textContent = visual === "working" ? statusLabel(visual) : `${statusLabel(visual)} · ${formatAge(age)}`;

    if (row.provider === "claude") {
      mascots.set(key, row.mascotEl, visual);
    }

    listEl.appendChild(row.li); // appendChild on an existing child reorders it in place
  }

  for (const [key, row] of sessionRows) {
    if (!wantedKeys.has(key)) {
      row.li.remove();
      sessionRows.delete(key);
    }
  }

  layoutStatusColumns(sessions);
  mascots.prune(new Set(sessions.filter((s) => s.provider === "claude").map((s) => `${s.provider}:${s.sessionId}`)));
}

// Several columns fit: one column group per status instead of row-by-row flow.
// The statuses present (in the order the sort puts them, i.e. Waiting >
// Question > Working > Idle for the status sort) each get a group. If more
// columns fit than there are statuses, the spare columns go to the groups
// with the most cards, which then show their cards side by side, row by row
// (allocateStatusColumns). If there are more statuses than columns, the last
// column takes all the rest. Within a group the chosen sort order applies.
// With a single column, or only one status present, cards flow normally.
function layoutStatusColumns(sessions: SessionInfo[]): void {
  const style = getComputedStyle(listEl);
  const minCard = parseFloat(style.getPropertyValue("--card-min-width")) || 300;
  const gap = parseFloat(style.columnGap) || 0;
  const width = listEl.clientWidth - (parseFloat(style.paddingLeft) || 0) - (parseFloat(style.paddingRight) || 0);
  const fitColumns = Math.max(1, Math.floor((width + gap) / (minCard + gap)));

  const statuses: MascotStatus[] = [];
  const counts: number[] = [];
  for (const session of sessions) {
    const visual = visualStatus(session);
    const index = statuses.indexOf(visual);
    if (index < 0) {
      statuses.push(visual);
      counts.push(1);
    } else {
      counts[index] += 1;
    }
  }

  const byStatus = fitColumns >= 2 && statuses.length >= 2;
  // Spare columns: groups of several columns, cards placed explicitly.
  const spread = byStatus && fitColumns > statuses.length;
  const spans = spread ? allocateStatusColumns(counts, fitColumns) : statuses.map(() => 1);
  const starts: number[] = [];
  spans.reduce((start, span) => (starts.push(start), start + span), 1);
  const totalColumns = spread ? spans.reduce((sum, span) => sum + span, 0) : Math.min(fitColumns, statuses.length);

  listEl.classList.toggle("status-columns", byStatus);
  listEl.style.gridTemplateColumns = byStatus ? `repeat(${totalColumns}, minmax(0, 1fr))` : "";

  const placed = statuses.map(() => 0);
  for (const session of sessions) {
    const row = sessionRows.get(sessionKey(session));
    if (!row) continue;
    const group = statuses.indexOf(visualStatus(session));
    if (!byStatus) {
      row.li.style.gridColumn = "";
      row.li.style.gridRow = "";
    } else if (spread) {
      const i = placed[group]++;
      row.li.style.gridColumn = String(starts[group] + (i % spans[group]));
      row.li.style.gridRow = String(Math.floor(i / spans[group]) + 1);
    } else {
      row.li.style.gridColumn = String(Math.min(group, totalColumns - 1) + 1);
      row.li.style.gridRow = "";
    }
  }
}

// ---------------------------------------------------------------------------
// Filter and sort (pure logic in sessionOrder.ts)
// ---------------------------------------------------------------------------

const STATUS_FILTER_STORAGE_KEY = "status-filter";
const SORT_STORAGE_KEY = "sort";

let visibleStatuses: Set<VisualStatus> = parseStatusFilter(localStorage.getItem(STATUS_FILTER_STORAGE_KEY));
let sortSettings: SortSettings = parseSortSettings(localStorage.getItem(SORT_STORAGE_KEY));
let statusSince = new Map<string, StatusSince>();
let lastSessions: SessionInfo[] = [];

interface OrderedSession extends OrderableSession {
  session: SessionInfo;
}

function sessionKey(session: SessionInfo): string {
  return `${session.provider}:${session.sessionId}`;
}

function applyOrdering(sessions: SessionInfo[]): { visible: SessionInfo[]; counts: Record<VisualStatus, number> } {
  const now = Date.now();
  statusSince = updateStatusSince(
    statusSince,
    sessions.map((s) => ({ key: sessionKey(s), status: visualStatus(s), reportedSince: toEpochMs(s.statusUpdatedAt) })),
    now,
  );
  const entries: OrderedSession[] = sessions.map((s) => {
    const key = sessionKey(s);
    return {
      key,
      status: visualStatus(s),
      folder: lastPathSegment(s.cwd),
      title: s.name,
      since: statusSince.get(key)?.since ?? now,
      session: s,
    };
  });
  const visible = filterSessions(sortSessions(entries, sortSettings), visibleStatuses).map((e) => e.session);
  return { visible, counts: countByStatus(entries) };
}

const STATUS_CHIP_TITLES: Record<VisualStatus, string> = {
  needsInput: "Waiting for input",
  waitingAlert: "Question",
  working: "Working",
  waiting: "Idle",
};

const SORT_KEY_LABELS: Record<SortKey, { label: string; description: string }> = {
  status: { label: "Status", description: "Sorted by status priority, newest status change first" },
  title: { label: "Title", description: "Sorted by session title, then folder" },
  folder: { label: "Folder", description: "Sorted by project folder, then title" },
};

const filterChips = [...document.querySelectorAll<HTMLButtonElement>(".filter-chip")];
const sortKeyBtn = document.getElementById("sort-key-btn") as HTMLButtonElement;
const sortDirBtn = document.getElementById("sort-dir-btn") as HTMLButtonElement;

function renderFilterBar(counts: Record<VisualStatus, number>): void {
  for (const chip of filterChips) {
    const status = chip.dataset.status;
    if (!isVisualStatus(status)) continue;
    const active = visibleStatuses.has(status);
    chip.classList.toggle("active", active);
    chip.setAttribute("aria-pressed", String(active));
    const countEl = chip.querySelector(".chip-count");
    if (countEl) countEl.textContent = String(counts[status]);
    chip.title = `${STATUS_CHIP_TITLES[status]}: ${counts[status]} — click to ${active ? "hide" : "show"}`;
  }
  const key = sortSettings.key;
  const reversed = sortSettings.direction === "desc";
  sortKeyBtn.textContent = SORT_KEY_LABELS[key].label;
  sortKeyBtn.title = `${SORT_KEY_LABELS[key].description} — click to sort by ${SORT_KEY_LABELS[nextSortKey(key)].label.toLowerCase()}`;
  sortDirBtn.textContent = reversed ? "↑" : "↓";
  sortDirBtn.title =
    key === "status"
      ? reversed ? "Idle first — click for Waiting first" : "Waiting first — click for Idle first"
      : reversed ? "Z → A — click for A → Z" : "A → Z — click for Z → A";
}

// Re-applies filter/sort to the last poll's data right away instead of
// waiting up to POLL_INTERVAL_MS for the next refresh.
function rerender(): void {
  const { visible, counts } = applyOrdering(lastSessions);
  render(visible, lastSessions.length);
  renderFilterBar(counts);
  void updateStatusIndicators(counts);
  void resizeWindowToContent();
}

for (const chip of filterChips) {
  chip.addEventListener("click", () => {
    const status = chip.dataset.status;
    if (!isVisualStatus(status)) return;
    visibleStatuses = toggleStatus(visibleStatuses, status);
    localStorage.setItem(STATUS_FILTER_STORAGE_KEY, serializeStatusFilter(visibleStatuses));
    rerender();
  });
}

// Cycles Status → Title → Folder → Status.
sortKeyBtn.addEventListener("click", () => {
  sortSettings = { ...sortSettings, key: nextSortKey(sortSettings.key) };
  localStorage.setItem(SORT_STORAGE_KEY, serializeSortSettings(sortSettings));
  rerender();
});

sortDirBtn.addEventListener("click", () => {
  sortSettings = { ...sortSettings, direction: sortSettings.direction === "asc" ? "desc" : "asc" };
  localStorage.setItem(SORT_STORAGE_KEY, serializeSortSettings(sortSettings));
  rerender();
});

renderFilterBar(countByStatus([]));

// All four statuses exist as chips; guards against index.html and
// sessionOrder.ts drifting apart.
if (filterChips.length !== STATUS_PRIORITY.length) {
  console.warn("Filter chips and STATUS_PRIORITY are out of sync");
}

let refreshing = false;

async function refresh(): Promise<void> {
  if (refreshing) return;
  refreshing = true;
  try {
    const payload = await invoke<SessionsPayload>("get_sessions");
    lastSessions = payload.sessions;
    const { visible, counts } = applyOrdering(lastSessions);
    render(visible, lastSessions.length);
    renderFilterBar(counts);
    void updateStatusIndicators(counts);
    renderUsage(payload.usage);
    await resizeWindowToContent();
  } catch (err) {
    console.error("Failed to refresh sessions", err);
  } finally {
    refreshing = false;
  }
}

void refresh();
setInterval(() => void refresh(), POLL_INTERVAL_MS);

// ---------------------------------------------------------------------------
// Menus (fork): native right-click menu and tray menu, both from
// contextMenu.ts. The widget's own settings are changed here; window layer
// and taskbar button live in windowLayer.ts, About & help in about.ts.
// ---------------------------------------------------------------------------

// Autostart state lives on disk (Startup-folder shortcut), not in
// LocalStorage, so it is read from the backend.
let autostartEnabled = false;

invoke<boolean>("get_autostart")
  .then((enabled) => {
    autostartEnabled = enabled;
    void refreshTrayMenu();
  })
  .catch((err) => console.error("Failed to read autostart state", err));

function menuState(): MenuState {
  return {
    windowLayer: getWindowLayer(),
    showInTaskbar: isShownInTaskbar(),
    autostart: autostartEnabled,
    compact: density === "compact",
    transparent: transparentBackground,
    sizeMode,
    theme: themePreference,
    openInDesktop: clickSettings.openInDesktop,
    sidebarSelect: clickSettings.sidebarSelect,
  };
}

const menuActions: MenuActions = {
  setWindowLayer,
  toggleTaskbarButton,
  toggleAutostart() {
    invoke<boolean>("set_autostart", { enabled: !autostartEnabled })
      .then((enabled) => {
        autostartEnabled = enabled;
        void refreshTrayMenu();
      })
      .catch((err) => window.alert(`Start with Windows: ${err}`));
  },
  toggleCompact() {
    density = density === "compact" ? "normal" : "compact";
    localStorage.setItem(DENSITY_STORAGE_KEY, density);
    applyDensity();
    void resizeWindowToContent(); // re-fits the window to the new density in auto size
  },
  toggleTransparent() {
    transparentBackground = !transparentBackground;
    saveFlag(TRANSPARENT_STORAGE_KEY, transparentBackground);
    applyTransparency();
  },
  setSizeMode(mode) {
    sizeMode = mode;
    localStorage.setItem(SIZE_MODE_STORAGE_KEY, sizeMode);
    void applySizeMode();
  },
  setTheme(pref) {
    themePreference = pref;
    localStorage.setItem(THEME_STORAGE_KEY, themePreference);
    applyTheme();
  },
  toggleOpenInDesktop() {
    clickSettings.openInDesktop = !clickSettings.openInDesktop;
    saveFlag(OPEN_IN_DESKTOP_STORAGE_KEY, clickSettings.openInDesktop);
  },
  toggleSidebarSelect() {
    clickSettings.sidebarSelect = !clickSettings.sidebarSelect;
    saveFlag(SIDEBAR_SELECT_STORAGE_KEY, clickSettings.sidebarSelect);
  },
  refresh: () => void refresh(),
  openAbout: () => setAboutOpen(true),
  close: () => void invoke("close_app"),
  changed: () => void refreshTrayMenu(),
};

function trayEntries() {
  return trayMenuEntries(menuState(), menuActions, {
    bringToFront: () => void bringToFront(),
    hide: hideWidget,
  });
}

async function refreshTrayMenu(): Promise<void> {
  await setTrayMenu(trayEntries());
}

// Native popup menu: overlaps the desktop and other windows instead of being
// clipped to (or growing) the widget. The previous one is released when the
// next one is built.
let contextMenu: Menu | null = null;

document.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  void (async () => {
    try {
      const previous = contextMenu;
      contextMenu = await Menu.new({ items: contextMenuEntries(menuState(), menuActions) });
      await previous?.close();
      await contextMenu.popup(); // at the mouse position
    } catch (err) {
      console.error("Failed to open the context menu", err);
    }
  })();
});

// Startup of the fork's modules, after all state above exists.
initWindowLayer();
initAbout(() => void resizeWindowToContent());
// Tray icon + taskbar badge (tray.ts).
void initTray({
  onClick: () => void onTrayClick(),
  onDoubleClick: () => void bringToFront(),
  menu: trayEntries(),
});
