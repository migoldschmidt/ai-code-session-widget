// Unit tests for src/sessionOrder.ts. Run with `npm test` (node --test with
// Node's built-in TypeScript type stripping, Node >= 22.18).

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  type OrderableSession,
  type VisualStatus,
  STATUS_PRIORITY,
  STALE_IDLE_MS,
  allocateStatusColumns,
  formatAge,
  countByStatus,
  filterSessions,
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
} from "../src/sessionOrder.ts";

function s(key: string, status: VisualStatus, folder: string, title = "", since = 0): OrderableSession {
  return { key, status, folder, title, since };
}

const keys = (list: readonly OrderableSession[]): string[] => list.map((x) => x.key);

test("status sort follows Waiting > Question > Working > Idle", () => {
  const list = [
    s("idle", "waiting", "a"),
    s("work", "working", "a"),
    s("question", "waitingAlert", "a"),
    s("input", "needsInput", "a"),
  ];
  assert.deepEqual(keys(sortSessions(list, { key: "status", direction: "asc" })), ["input", "question", "work", "idle"]);
});

test("same status: most recent status change first", () => {
  const list = [s("old", "working", "a", "", 1000), s("new", "working", "b", "", 3000), s("mid", "working", "c", "", 2000)];
  assert.deepEqual(keys(sortSessions(list, { key: "status", direction: "asc" })), ["new", "mid", "old"]);
  // Priority still comes first: an old Waiting session beats a fresh Working one.
  const mixed = [s("fresh-work", "working", "a", "", 9000), s("old-input", "needsInput", "b", "", 1000)];
  assert.deepEqual(keys(sortSessions(mixed, { key: "status", direction: "asc" })), ["old-input", "fresh-work"]);
});

test("same status and time: falls back to title, then folder", () => {
  const list = [s("3", "working", "a", "Zeta", 5), s("2", "working", "beta", "Alpha", 5), s("1", "working", "Alpha", "alpha", 5)];
  assert.deepEqual(keys(sortSessions(list, { key: "status", direction: "asc" })), ["1", "2", "3"]);
});

test("desc reverses the status order", () => {
  const list = [s("input", "needsInput", "a"), s("idle", "waiting", "a"), s("work", "working", "a")];
  assert.deepEqual(keys(sortSessions(list, { key: "status", direction: "desc" })), ["idle", "work", "input"]);
});

test("folder sort: folder (case-insensitive, numeric), then title", () => {
  const list = [
    s("c", "working", "project10", "x"),
    s("b", "working", "Project2", "b-title"),
    s("a", "needsInput", "project2", "a-title"),
  ];
  assert.deepEqual(keys(sortSessions(list, { key: "folder", direction: "asc" })), ["a", "b", "c"]);
  assert.deepEqual(keys(sortSessions(list, { key: "folder", direction: "desc" })), ["c", "b", "a"]);
});

test("title sort: title (case-insensitive, numeric), then folder", () => {
  const list = [
    s("c", "waiting", "aaa", "Task 10"),
    s("b", "working", "zzz", "task 2"),
    s("a", "needsInput", "mmm", "Task 2"),
    s("d", "working", "bbb", "Rework login form"),
  ];
  assert.deepEqual(keys(sortSessions(list, { key: "title", direction: "asc" })), ["d", "a", "b", "c"]);
  assert.deepEqual(keys(sortSessions(list, { key: "title", direction: "desc" })), ["c", "b", "a", "d"]);
});

test("sort key button cycles status -> title -> folder -> status", () => {
  assert.equal(nextSortKey("status"), "title");
  assert.equal(nextSortKey("title"), "folder");
  assert.equal(nextSortKey("folder"), "status");
});

test("sortSessions does not mutate its input", () => {
  const list = [s("b", "waiting", "b"), s("a", "needsInput", "a")];
  sortSessions(list, { key: "status", direction: "asc" });
  assert.deepEqual(keys(list), ["b", "a"]);
});

test("filter keeps only visible statuses and counts ignore the filter", () => {
  const list = [s("1", "needsInput", "a"), s("2", "working", "a"), s("3", "working", "a"), s("4", "waiting", "a")];
  assert.deepEqual(keys(filterSessions(list, new Set<VisualStatus>(["working"]))), ["2", "3"]);
  assert.deepEqual(countByStatus(list), { needsInput: 1, waitingAlert: 0, working: 2, waiting: 1 });
});

test("toggleStatus turns chips on and off, never leaves all off", () => {
  const all = new Set(STATUS_PRIORITY);
  const withoutIdle = toggleStatus(all, "waiting");
  assert.equal(withoutIdle.has("waiting"), false);
  assert.equal(withoutIdle.size, 3);
  assert.equal(toggleStatus(withoutIdle, "waiting").has("waiting"), true);
  const onlyWorking = new Set<VisualStatus>(["working"]);
  assert.deepEqual([...toggleStatus(onlyWorking, "working")].sort(), [...STATUS_PRIORITY].sort());
});

test("status filter round-trips and falls back to all on bad input", () => {
  const visible = new Set<VisualStatus>(["waiting", "needsInput"]);
  const raw = serializeStatusFilter(visible);
  assert.equal(raw, '["needsInput","waiting"]');
  assert.deepEqual([...parseStatusFilter(raw)].sort(), ["needsInput", "waiting"]);
  for (const bad of [null, "", "nope", "{}", "[]", '["bogus"]']) {
    assert.equal(parseStatusFilter(bad).size, STATUS_PRIORITY.length, `input ${String(bad)}`);
  }
});

test("sort settings round-trip and fall back to status/asc", () => {
  for (const key of ["status", "title", "folder"] as const) {
    assert.deepEqual(parseSortSettings(serializeSortSettings({ key, direction: "desc" })), { key, direction: "desc" });
  }
  // The first version stored the folder sort as "name".
  assert.deepEqual(parseSortSettings('{"key":"name","direction":"asc"}'), { key: "folder", direction: "asc" });
  for (const bad of [null, "", "null", '{"key":"x","direction":"asc"}', "[1]"]) {
    assert.deepEqual(parseSortSettings(bad), { key: "status", direction: "asc" }, `input ${String(bad)}`);
  }
});

test("toEpochMs understands ms, seconds, numeric strings and ISO dates", () => {
  assert.equal(toEpochMs(1789636143000), 1789636143000);
  assert.equal(toEpochMs(1789636143), 1789636143000);
  assert.equal(toEpochMs("1789636143000"), 1789636143000);
  assert.equal(toEpochMs("2026-10-18T16:54:06.000Z"), Date.UTC(2026, 9, 18, 16, 54, 6));
  for (const bad of [null, undefined, "", "soon", 0, -5, 42, Number.NaN, {}, []]) {
    assert.equal(toEpochMs(bad), null, `input ${String(bad)}`);
  }
});

test("updateStatusSince keeps, seeds and resets timestamps", () => {
  const now = 10_000;
  // First sighting: trust a plausible reported time, else now; future times are ignored.
  let state = updateStatusSince(
    new Map(),
    [
      { key: "a", status: "working", reportedSince: 4000 },
      { key: "b", status: "waiting", reportedSince: null },
      { key: "c", status: "waiting", reportedSince: 99_999 },
    ],
    now,
  );
  assert.deepEqual(state.get("a"), { status: "working", since: 4000 });
  assert.deepEqual(state.get("b"), { status: "waiting", since: now });
  assert.deepEqual(state.get("c"), { status: "waiting", since: now });

  // Unchanged status keeps its timestamp; changed status takes a newer reported
  // time, otherwise now; vanished sessions are dropped.
  state = updateStatusSince(
    state,
    [
      { key: "a", status: "working", reportedSince: 9000 },
      { key: "b", status: "needsInput", reportedSince: 12_000 },
    ],
    20_000,
  );
  assert.deepEqual(state.get("a"), { status: "working", since: 4000 });
  assert.deepEqual(state.get("b"), { status: "needsInput", since: 12_000 });
  assert.equal(state.has("c"), false);

  state = updateStatusSince(state, [{ key: "a", status: "waiting", reportedSince: 1000 }], 30_000);
  assert.deepEqual(state.get("a"), { status: "waiting", since: 30_000 });
});

test("lastPathSegment handles Windows and POSIX paths", () => {
  assert.equal(lastPathSegment("C:\\Dev-AI\\DelMan\\Ableich-DelMan-SSTs"), "Ableich-DelMan-SSTs");
  assert.equal(lastPathSegment("C:\\Projects\\foo\\"), "foo");
  assert.equal(lastPathSegment("/home/me/proj"), "proj");
});

test("allocateStatusColumns: spare columns go to the group with the most cards", () => {
  // 4 columns fit, Working has 5 cards, Idle 1: Working gets 3 side by side.
  assert.deepEqual(allocateStatusColumns([5, 1], 4), [3, 1]);
  // Spread evenly when both groups are large.
  assert.deepEqual(allocateStatusColumns([4, 4], 4), [2, 2]);
  // Ties go to the more urgent (left) group.
  assert.deepEqual(allocateStatusColumns([3, 3], 3), [2, 1]);
});

test("allocateStatusColumns: never more columns than cards, never below one", () => {
  // Only 1 + 2 cards: 3 of the 6 fitting columns are used, cards get wider.
  assert.deepEqual(allocateStatusColumns([1, 2], 6), [1, 2]);
  // Fewer columns than statuses: one each (caller merges the overflow).
  assert.deepEqual(allocateStatusColumns([3, 2, 1], 2), [1, 1, 1]);
  assert.deepEqual(allocateStatusColumns([], 4), []);
});

test("formatAge: minutes, hours, days", () => {
  assert.equal(formatAge(0), "now");
  assert.equal(formatAge(59_000), "now");
  assert.equal(formatAge(60_000), "1 min");
  assert.equal(formatAge(59 * 60_000), "59 min");
  assert.equal(formatAge(60 * 60_000), "1 h");
  assert.equal(formatAge(25 * 60 * 60_000), "1 d");
  assert.equal(formatAge(-5_000), "now");
  assert.equal(STALE_IDLE_MS, 60 * 60_000);
});
