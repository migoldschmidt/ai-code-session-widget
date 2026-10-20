// Filter and sort logic for the session list. Pure functions only — no DOM,
// no Tauri, no imports — so it runs under `node --test` without a bundler
// (see tests/sessionOrder.test.ts). Keep the syntax erasable (no enums or
// namespaces) so Node's built-in type stripping can load this file.

/** Status as shown on a card. Same union as mascot.ts::MascotStatus. */
export type VisualStatus = "needsInput" | "waitingAlert" | "working" | "waiting";

/**
 * Priority order, highest first: Waiting (needs input: permission prompt or
 * AskUserQuestion) > Question (finished with a question) > Working > Idle.
 */
export const STATUS_PRIORITY: readonly VisualStatus[] = ["needsInput", "waitingAlert", "working", "waiting"];

export interface OrderableSession {
  /** Stable identity, `${provider}:${sessionId}`. */
  key: string;
  status: VisualStatus;
  /** Last path segment of the session's cwd (what the card shows). */
  folder: string;
  /** Session title (`name` in the session file). */
  title: string;
  /** Epoch ms since which the session has been in `status`. */
  since: number;
}

export type SortKey = "status" | "title" | "folder";
export type SortDirection = "asc" | "desc";

/** Order in which the sort button cycles through the keys. */
export const SORT_KEYS: readonly SortKey[] = ["status", "title", "folder"];

export function nextSortKey(key: SortKey): SortKey {
  const index = SORT_KEYS.indexOf(key);
  return SORT_KEYS[(index + 1) % SORT_KEYS.length];
}

export interface SortSettings {
  key: SortKey;
  direction: SortDirection;
}

export const DEFAULT_SORT: SortSettings = { key: "status", direction: "asc" };

export function isVisualStatus(value: unknown): value is VisualStatus {
  return typeof value === "string" && (STATUS_PRIORITY as readonly string[]).includes(value);
}

export function lastPathSegment(cwd: string): string {
  const normalized = cwd.replace(/\\/g, "/").replace(/\/$/, "");
  const parts = normalized.split("/");
  return parts[parts.length - 1] || cwd;
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

export function filterSessions<T extends OrderableSession>(sessions: readonly T[], visible: ReadonlySet<VisualStatus>): T[] {
  return sessions.filter((s) => visible.has(s.status));
}

export function countByStatus(sessions: readonly OrderableSession[]): Record<VisualStatus, number> {
  const counts: Record<VisualStatus, number> = { needsInput: 0, waitingAlert: 0, working: 0, waiting: 0 };
  for (const s of sessions) counts[s.status] += 1;
  return counts;
}

/**
 * Toggles one status chip. Turning off the last active chip would hide
 * everything, which reads like a bug — so that click shows all statuses again.
 */
export function toggleStatus(visible: ReadonlySet<VisualStatus>, status: VisualStatus): Set<VisualStatus> {
  const next = new Set(visible);
  if (next.has(status)) next.delete(status);
  else next.add(status);
  return next.size === 0 ? new Set(STATUS_PRIORITY) : next;
}

export function parseStatusFilter(raw: string | null): Set<VisualStatus> {
  if (raw !== null) {
    try {
      const parsed: unknown = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        const statuses = parsed.filter(isVisualStatus);
        if (statuses.length > 0) return new Set(statuses);
      }
    } catch {
      // malformed - fall through to the default
    }
  }
  return new Set(STATUS_PRIORITY);
}

export function serializeStatusFilter(visible: ReadonlySet<VisualStatus>): string {
  return JSON.stringify(STATUS_PRIORITY.filter((s) => visible.has(s)));
}

// ---------------------------------------------------------------------------
// Sort
// ---------------------------------------------------------------------------

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

function compareKeys(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Project folder, then session title, then the key as a stable tie-break (plan item 15). */
function compareByFolder(a: OrderableSession, b: OrderableSession): number {
  return collator.compare(a.folder, b.folder) || collator.compare(a.title, b.title) || compareKeys(a.key, b.key);
}

/** Session title (what the standard card shows on top), then folder, then key. */
function compareByTitle(a: OrderableSession, b: OrderableSession): number {
  return collator.compare(a.title, b.title) || collator.compare(a.folder, b.folder) || compareKeys(a.key, b.key);
}

/**
 * Status priority, then the most recent status change first (newest `since`
 * on top — the plan's "longest in state first" was swapped on request),
 * then by title.
 */
function compareByStatus(a: OrderableSession, b: OrderableSession): number {
  const byPriority = STATUS_PRIORITY.indexOf(a.status) - STATUS_PRIORITY.indexOf(b.status);
  if (byPriority !== 0) return byPriority;
  if (a.since !== b.since) return b.since - a.since;
  return compareByTitle(a, b);
}

const COMPARATORS: Record<SortKey, (a: OrderableSession, b: OrderableSession) => number> = {
  status: compareByStatus,
  title: compareByTitle,
  folder: compareByFolder,
};

/** Returns a sorted copy. "desc" reverses the whole order. */
export function sortSessions<T extends OrderableSession>(sessions: readonly T[], settings: SortSettings): T[] {
  const compare = COMPARATORS[settings.key];
  const sign = settings.direction === "desc" ? -1 : 1;
  return [...sessions].sort((a, b) => sign * compare(a, b));
}

export function parseSortSettings(raw: string | null): SortSettings {
  if (raw !== null) {
    try {
      const parsed = JSON.parse(raw) as { key?: unknown; direction?: unknown } | null;
      // "name" is what the first version stored for the folder sort.
      const key = parsed && parsed.key === "name" ? "folder" : parsed?.key;
      const direction = parsed?.direction;
      if ((SORT_KEYS as readonly unknown[]).includes(key) && (direction === "asc" || direction === "desc")) {
        return { key: key as SortKey, direction };
      }
    } catch {
      // malformed - fall through to the default
    }
  }
  return { ...DEFAULT_SORT };
}

export function serializeSortSettings(settings: SortSettings): string {
  return JSON.stringify({ key: settings.key, direction: settings.direction });
}

// ---------------------------------------------------------------------------
// Time in current state
// ---------------------------------------------------------------------------

/**
 * Best-effort conversion of Claude Code's `statusUpdatedAt` (format not
 * documented) to epoch ms: epoch milliseconds, epoch seconds, a numeric
 * string of either, or an ISO date string. Anything else gives null.
 */
export function toEpochMs(value: unknown): number | null {
  let num: number | null = null;
  if (typeof value === "number") {
    num = value;
  } else if (typeof value === "string" && value.trim() !== "") {
    const trimmed = value.trim();
    if (/^\d+(\.\d+)?$/.test(trimmed)) {
      num = Number(trimmed);
    } else {
      const parsed = Date.parse(trimmed);
      return Number.isFinite(parsed) ? parsed : null;
    }
  }
  if (num === null || !Number.isFinite(num) || num <= 0) return null;
  if (num >= 1e12) return Math.round(num); // milliseconds
  if (num >= 1e9) return Math.round(num * 1000); // seconds
  return null;
}

export interface StatusSince {
  status: VisualStatus;
  since: number;
}

export interface StatusObservation {
  key: string;
  status: VisualStatus;
  /** statusUpdatedAt from the session file, already converted, or null. */
  reportedSince: number | null;
}

/**
 * Tracks since when each session has shown its current status.
 * - same status as last poll: keep the old timestamp
 * - first sighting: trust the reported timestamp if plausible, else now
 * - status changed: the reported timestamp if it is newer than the previous
 *   one (the hook recorded the change), else now
 * Sessions that are gone are dropped.
 */
export function updateStatusSince(
  previous: ReadonlyMap<string, StatusSince>,
  observations: readonly StatusObservation[],
  now: number,
): Map<string, StatusSince> {
  const next = new Map<string, StatusSince>();
  for (const obs of observations) {
    const prev = previous.get(obs.key);
    const reported = obs.reportedSince !== null && obs.reportedSince <= now ? obs.reportedSince : null;
    if (prev && prev.status === obs.status) {
      next.set(obs.key, prev);
    } else if (!prev) {
      next.set(obs.key, { status: obs.status, since: reported ?? now });
    } else {
      const since = reported !== null && reported > prev.since ? reported : now;
      next.set(obs.key, { status: obs.status, since });
    }
  }
  return next;
}

/**
 * Status columns (fork): how many grid columns each status group gets when
 * more columns fit than there are statuses. `counts` = cards per status in
 * column order (left to right), `fitColumns` = columns that fit the width.
 * Every status gets one column; the spare columns go one by one to the group
 * that currently needs the most rows (ties: the more urgent, i.e. earlier,
 * group), but never more columns than the group has cards. With fewer
 * columns than statuses every status keeps one column (the caller merges the
 * rest into the last one).
 */
export function allocateStatusColumns(counts: readonly number[], fitColumns: number): number[] {
  const spans = counts.map(() => 1);
  let spare = Math.max(0, fitColumns - counts.length);
  while (spare > 0) {
    let best = -1;
    let bestRows = 0;
    counts.forEach((count, i) => {
      if (spans[i] >= count) return;
      const rows = Math.ceil(count / spans[i]);
      if (rows > bestRows) {
        best = i;
        bestRows = rows;
      }
    });
    if (best < 0) break;
    spans[best] += 1;
    spare -= 1;
  }
  return spans;
}

/** Idle longer than this counts as "probably forgotten": the card is dimmed (fork). */
export const STALE_IDLE_MS = 60 * 60 * 1000;

/**
 * Short "time in this status" for the status line (fork): "now" below a
 * minute, then minutes, hours, days ("12 min", "3 h", "2 d").
 */
export function formatAge(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / 60000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h`;
  return `${Math.floor(hours / 24)} d`;
}
