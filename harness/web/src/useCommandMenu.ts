// Headless state machine for the Composer popover.
//
// Three modes share one popover:
//   - "slash"   — built-ins + skills, triggered by `/` at line-start
//   - "at"      — file browser under cwd, triggered by `@` at word-boundary
//   - "history" — walk back through prior user messages of the active session
//
// Selection lives in this reducer. Item *fetching* lives in the Composer
// (it owns IO). The reducer is pure; tests don't need IO.

import { useReducer } from "react";

export type MenuMode = "idle" | "slash" | "at" | "history";

export interface MenuItem {
  /** Coarse provenance — drives icons and how Enter is dispatched. */
  kind: "builtin" | "skill" | "file";
  /** Stable id within the mode (e.g. `/cwd`, `iii://skills/xyz`, absolute path). */
  id: string;
  /** Display text shown as the row's primary label. */
  label: string;
  /** Optional secondary text under the label. */
  description?: string;
  /** Mode-specific payload (skill uri, fs entry kind, etc.). */
  meta?: unknown;
}

export interface CommandMenuState {
  mode: MenuMode;
  /** Text after the trigger char. Empty for history mode. */
  query: string;
  /** Filtered + ranked items the popover should render. */
  items: MenuItem[];
  /** Index into `items`. Always 0..max(items.length-1, 0). */
  selectedIndex: number;
  /** History walk pointer. null = drafting (not in history mode), 0+ walks back. */
  historyIndex: number | null;
  /** Original draft text preserved when history walk starts. Restored on Esc. */
  historyDraft: string;
}

export const INITIAL_MENU_STATE: CommandMenuState = {
  mode: "idle",
  query: "",
  items: [],
  selectedIndex: 0,
  historyIndex: null,
  historyDraft: "",
};

export type Action =
  | { kind: "open-slash"; items: MenuItem[] }
  | { kind: "open-at"; items: MenuItem[] }
  | { kind: "open-history"; draft: string }
  | { kind: "set-items"; items: MenuItem[] }
  | { kind: "filter"; query: string; items: MenuItem[] }
  | { kind: "move"; delta: 1 | -1 }
  | { kind: "history-step"; delta: 1 | -1; historyLen: number }
  | { kind: "close" };

function clampIndex(items: MenuItem[], idx: number): number {
  if (items.length === 0) return 0;
  if (idx < 0) return 0;
  if (idx >= items.length) return items.length - 1;
  return idx;
}

export function reduce(state: CommandMenuState, action: Action): CommandMenuState {
  switch (action.kind) {
    case "open-slash":
      return {
        ...INITIAL_MENU_STATE,
        mode: "slash",
        items: action.items,
        selectedIndex: 0,
      };
    case "open-at":
      return {
        ...INITIAL_MENU_STATE,
        mode: "at",
        items: action.items,
        selectedIndex: 0,
      };
    case "open-history":
      return {
        ...INITIAL_MENU_STATE,
        mode: "history",
        historyDraft: action.draft,
        historyIndex: 0,
      };
    case "set-items":
      // Caller refreshed items without changing the query. Preserve selection
      // when possible; clamp if items shrunk.
      return {
        ...state,
        items: action.items,
        selectedIndex: clampIndex(action.items, state.selectedIndex),
      };
    case "filter":
      // Query changed; reset selection to the top because ranking shifted.
      return {
        ...state,
        query: action.query,
        items: action.items,
        selectedIndex: 0,
      };
    case "move": {
      const next = clampIndex(state.items, state.selectedIndex + action.delta);
      return next === state.selectedIndex ? state : { ...state, selectedIndex: next };
    }
    case "history-step": {
      // delta=+1 walks further back, delta=-1 walks toward the draft.
      // Caps at [0, historyLen-1]. The Composer turns historyIndex back into
      // the draft text when stepping past 0 with delta=-1 (handled there).
      const cur = state.historyIndex ?? 0;
      const proposed = cur + action.delta;
      if (action.historyLen === 0) return state;
      const clamped = Math.max(0, Math.min(action.historyLen - 1, proposed));
      if (clamped === cur) return state;
      return { ...state, historyIndex: clamped };
    }
    case "close":
      return INITIAL_MENU_STATE;
    default:
      return state;
  }
}

/** React hook wrapper around the reducer. */
export function useCommandMenu(): [CommandMenuState, React.Dispatch<Action>] {
  return useReducer(reduce, INITIAL_MENU_STATE);
}

// ─── Trigger helpers ────────────────────────────────────────────────────────
// Pure functions the Composer uses to decide whether a keystroke should open
// a mode. Kept here so they're easy to unit-test alongside the reducer.

/**
 * Slash triggers when `/` lands at the start of a line — i.e. the textarea is
 * empty OR the char immediately before the caret is `\n`. The trigger char
 * itself is at `caret-1` (caller has already inserted it).
 */
export function shouldOpenSlash(text: string, caret: number): boolean {
  if (caret <= 0) return false;
  if (text[caret - 1] !== "/") return false;
  if (caret === 1) return true;
  return text[caret - 2] === "\n";
}

/**
 * At-mention triggers when `@` follows whitespace, line-start, or empty input.
 * The trigger char sits at `caret-1`.
 */
export function shouldOpenAt(text: string, caret: number): boolean {
  if (caret <= 0) return false;
  if (text[caret - 1] !== "@") return false;
  if (caret === 1) return true;
  const prev = text[caret - 2];
  return prev === " " || prev === "\t" || prev === "\n";
}

/**
 * Extract the query the user has typed after the active trigger char. Returns
 * null if the caret has moved before the trigger (mode should close).
 */
export function extractQuery(
  text: string,
  caret: number,
  trigger: "/" | "@",
): string | null {
  // Walk back from the caret to find the most recent trigger char that still
  // qualifies (line-start for slash, word-boundary for at). We only care about
  // the contiguous run from that trigger to the caret with no whitespace.
  for (let i = caret - 1; i >= 0; i--) {
    const c = text[i];
    if (c === "\n") return null;
    if (trigger === "@" && (c === " " || c === "\t")) return null;
    if (c === trigger) {
      // Confirm boundary
      if (trigger === "/" && i !== 0 && text[i - 1] !== "\n") return null;
      if (trigger === "@" && i !== 0) {
        const prev = text[i - 1];
        if (prev !== " " && prev !== "\t" && prev !== "\n") return null;
      }
      return text.slice(i + 1, caret);
    }
  }
  return null;
}
