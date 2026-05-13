// Slash-menu items: built-in commands + skills surfaced from the
// `directory::skills::list` rows.  Plus a fuzzy filter the popover
// applies as the user types.
//
// The filter ranking is intentionally simple — the slash menu has at most a
// few dozen entries, so we sort in-memory on every keystroke.

import type { MenuItem } from "./useCommandMenu";

export const BUILT_IN_COMMANDS: MenuItem[] = [
  { kind: "builtin", id: "/new", label: "/new", description: "Start a new session" },
  { kind: "builtin", id: "/clear", label: "/clear", description: "Clear current draft" },
  {
    kind: "builtin",
    id: "/cwd",
    label: "/cwd",
    description: "Set working directory: /cwd <path>",
  },
  {
    kind: "builtin",
    id: "/model",
    label: "/model",
    description: "Switch model: /model <id>",
  },
  {
    kind: "builtin",
    id: "/provider",
    label: "/provider",
    description: "Switch provider: /provider <name>",
  },
  { kind: "builtin", id: "/help", label: "/help", description: "Show shortcuts" },
  {
    kind: "builtin",
    id: "/repair",
    label: "/repair",
    description: "Repair session-tree drift via session-tree::reconcile",
  },
  {
    kind: "builtin",
    id: "/fork",
    label: "/fork",
    description: "Fork session at last message",
  },
  {
    kind: "builtin",
    id: "/export md",
    label: "/export md",
    description: "Export current session as markdown",
  },
  {
    kind: "builtin",
    id: "/export json",
    label: "/export json",
    description: "Export current session as JSON",
  },
];

const SCORE_PREFIX = 100;
const SCORE_SUBSTRING = 50;
const SCORE_FUZZY_ID = 10;
const SCORE_FUZZY_LABEL = 5;
const SCORE_THRESHOLD = 5;

/**
 * In-order character match: every char of needle appears (in order, not
 * necessarily contiguous) in haystack. Case-insensitive.
 */
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

function scoreItem(item: MenuItem, query: string): number {
  const id = item.id.toLowerCase();
  const label = item.label.toLowerCase();
  const q = query.toLowerCase();

  if (id.startsWith(q)) return SCORE_PREFIX;
  if (id.includes(q)) return SCORE_SUBSTRING;
  if (fuzzyMatches(id, q)) return SCORE_FUZZY_ID;
  if (fuzzyMatches(label, q)) return SCORE_FUZZY_LABEL;
  return 0;
}

/**
 * Filter + rank items by query. Empty query returns items in original order
 * (no ranking). Items scoring below threshold are dropped.
 */
export function filterCommands(items: MenuItem[], query: string): MenuItem[] {
  if (query.length === 0) return items.slice();
  const scored = items
    .map((item, idx) => ({ item, score: scoreItem(item, query), idx }))
    .filter((x) => x.score >= SCORE_THRESHOLD);
  // Stable sort: higher score first, original index breaks ties.
  scored.sort((a, b) => {
    if (b.score !== a.score) return b.score - a.score;
    return a.idx - b.idx;
  });
  return scored.map((x) => x.item);
}

/**
 * One row from `directory::skills::list`. The worker enriches each row
 * with `title` + `description` so a picker doesn't need a follow-up
 * `directory::skills::get` per entry.
 */
export interface SkillRow {
  id: string;
  title?: string;
  description?: string;
}

/**
 * Project `directory::skills::list` rows into MenuItems for the slash
 * popover. Entries without a non-empty `id` are dropped silently;
 * everything else becomes a `/skill-id` mention with `<title> — <description>`
 * (or just `<title>` when description is empty) as the secondary line.
 *
 * Returns [] when `rows` is null/undefined or empty.
 */
export function skillsListToMenuItems(rows: SkillRow[] | null | undefined): MenuItem[] {
  if (rows == null) return [];
  const out: MenuItem[] = [];
  for (const row of rows) {
    const id = row.id?.trim();
    if (!id) continue;
    const title = row.title?.trim() || id;
    const description = row.description?.trim() ?? "";
    const secondary = description ? `${title} — ${description}` : title;
    out.push({
      kind: "skill",
      id: `/${id}`,
      label: `/${id}`,
      description: secondary,
      meta: { id, uri: `iii://${id}` },
    });
  }
  return out;
}
