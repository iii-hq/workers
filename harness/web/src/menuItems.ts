// Slash-menu items: built-in commands + skills parsed from the iii://skills
// markdown index. Plus a fuzzy filter the popover applies as the user types.
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

// Each line of the rendered iii://skills index looks like:
//   - [name](iii://skills/<id>) — <description>
// or with a hyphen-minus instead of em-dash. We keep the regex permissive
// but anchor it on the `iii://skills/` URI so non-skill lines are skipped.
const SKILL_LINE = /^-\s+\[([^\]]+)\]\((iii:\/\/skills\/[^)]+)\)\s*[—\-]\s*(.+)$/;

/**
 * Parse the markdown body returned by `skill::fetch iii://skills` into
 * MenuItems. Lines that don't match the expected shape are skipped silently.
 * Returns [] if the index hasn't loaded yet.
 */
export function skillsIndexToMenuItems(index: string | null): MenuItem[] {
  if (index == null) return [];
  const out: MenuItem[] = [];
  for (const raw of index.split("\n")) {
    const line = raw.trim();
    if (!line.startsWith("-")) continue;
    const m = SKILL_LINE.exec(line);
    if (!m) continue;
    const [, name, uri, description] = m;
    // Derive the id portion of the URI (everything after `iii://skills/`).
    const idPart = uri.slice("iii://skills/".length);
    out.push({
      kind: "skill",
      id: `/${idPart}`,
      label: `/${idPart}`,
      description: `${name} — ${description}`,
      meta: { uri },
    });
  }
  return out;
}
