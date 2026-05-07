// Pure helpers for the bus function palette (Cmd-J).
//
// Kept pure so the FunctionPalette component reduces to event wiring +
// rendering. Filter ranking mirrors menuItems.ts::filterCommands so the two
// in-app fuzzies behave the same. Recent-list persistence is intentionally
// localStorage-only — Phase B doesn't sync invocation history across
// browsers, and the function palette is a power-user tool.

export interface FunctionEntry {
  function_id: string;
  description?: string;
  // Pass-through bag for anything else engine::functions::list returns
  // (request_format, metadata, etc.) so callers can render extra fields
  // without changes here.
  [extra: string]: unknown;
}

export interface RecentInvocation {
  function_id: string;
  payload: unknown;
  ts: number;
  ok: boolean; // true = bridge call succeeded, false = errored
}

const SENSITIVE_PATTERNS: readonly RegExp[] = [
  /^policy::/,
  /^auth::set_/,
  /^shell::filesystem::write$/,
  /^shell::filesystem::mkdir$/,
  /^shell::filesystem::rm$/,
];

/**
 * True iff calling `fnId` would mutate state or escalate authority. The
 * palette's Send button is replaced by a two-step Enter-to-confirm prompt
 * for these. List is intentionally narrow — we lean toward over-confirm
 * later, not bypass-now.
 */
export function isSensitive(fnId: string): boolean {
  return SENSITIVE_PATTERNS.some((re) => re.test(fnId));
}

/**
 * Extract the worker prefix from a `worker::action` style function id.
 * Returns `"(unknown)"` when the id has no `::` separator.
 */
export function workerFromId(fnId: string): string {
  const idx = fnId.indexOf("::");
  return idx > 0 ? fnId.slice(0, idx) : "(unknown)";
}

// ---- filter ranking (mirrors menuItems.ts) ----

const SCORE_PREFIX = 100;
const SCORE_SUBSTRING_ID = 60;
const SCORE_SUBSTRING_DESC = 40;
const SCORE_FUZZY_ID = 10;
const SCORE_FUZZY_DESC = 5;
const SCORE_THRESHOLD = 5;

function fuzzyMatches(haystack: string, needle: string): boolean {
  if (needle.length === 0) return true;
  let h = 0;
  let n = 0;
  while (h < haystack.length && n < needle.length) {
    if (haystack[h] === needle[n]) n += 1;
    h += 1;
  }
  return n === needle.length;
}

function scoreEntry(entry: FunctionEntry, q: string): number {
  const id = entry.function_id.toLowerCase();
  const worker = workerFromId(entry.function_id).toLowerCase();
  const desc = (entry.description ?? "").toLowerCase();

  if (id.startsWith(q) || worker.startsWith(q)) return SCORE_PREFIX;
  if (id.includes(q) || worker.includes(q)) return SCORE_SUBSTRING_ID;
  if (desc.includes(q)) return SCORE_SUBSTRING_DESC;
  if (fuzzyMatches(id, q)) return SCORE_FUZZY_ID;
  if (fuzzyMatches(desc, q)) return SCORE_FUZZY_DESC;
  return 0;
}

/**
 * Filter + rank palette entries by query. Empty query returns items in
 * original order. Sort is stable: original index breaks score ties so the
 * UI doesn't reshuffle when scores are equal.
 */
export function filterPalette(
  items: FunctionEntry[],
  query: string,
): FunctionEntry[] {
  if (query.length === 0) return items.slice();
  const q = query.toLowerCase();
  const scored = items
    .map((item, idx) => ({ item, score: scoreEntry(item, q), idx }))
    .filter((x) => x.score >= SCORE_THRESHOLD);
  scored.sort((a, b) => {
    if (b.score !== a.score) return b.score - a.score;
    return a.idx - b.idx;
  });
  return scored.map((x) => x.item);
}

// ---- recent invocations ----

const RECENT_KEY = "harness.palette.recent";
const RECENT_CAP = 20;

function isRecentInvocation(v: unknown): v is RecentInvocation {
  if (typeof v !== "object" || v === null) return false;
  const r = v as Record<string, unknown>;
  return (
    typeof r.function_id === "string" &&
    typeof r.ts === "number" &&
    typeof r.ok === "boolean"
  );
}

/**
 * Read the recent-invocation list from localStorage. Returns `[]` for any
 * shape we can't parse — keep failures invisible since this is recovery
 * data, not a source of truth.
 */
export function loadRecent(): RecentInvocation[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isRecentInvocation);
  } catch {
    return [];
  }
}

/**
 * Insert `entry` at the head of the recent list, dedupe by function_id (keep
 * the newer one), cap at RECENT_CAP. Returns the new list. Persists to
 * localStorage if available; failures are swallowed (storage full / disabled).
 */
export function pushRecent(entry: RecentInvocation): RecentInvocation[] {
  const current = loadRecent();
  const filtered = current.filter((r) => r.function_id !== entry.function_id);
  const next = [entry, ...filtered].slice(0, RECENT_CAP);
  if (typeof localStorage !== "undefined") {
    try {
      localStorage.setItem(RECENT_KEY, JSON.stringify(next));
    } catch {
      // Storage full / disabled — drop silently.
    }
  }
  return next;
}

/**
 * Build a `curl` invocation that hits the harness bridge with the same
 * function_id + payload. Quotes single quotes inside the JSON body using the
 * standard `'\''` shell escape so the command can be pasted into a real
 * shell.
 */
export function curlForBridge(fnId: string, payload: unknown): string {
  const body = JSON.stringify({ function_id: fnId, payload }, null, 2);
  const escaped = body.replace(/'/g, "'\\''");
  return `curl -X POST http://127.0.0.1:3111/bridge/trigger \\
  -H 'content-type: application/json' \\
  -d '${escaped}'`;
}
