// Generic global keybinder for window-level shortcuts (e.g. Cmd-J).
//
// Why not Cmd-K? Chrome's address bar focus already owns Cmd-K and many users
// have muscle memory for it. The Phase B design review picked Cmd-J for the
// function palette to avoid that collision. The handler here calls
// preventDefault + stopPropagation on the matched key combo so other listeners
// (and the browser default) don't also fire.
//
// `meta` and `ctrl` are both optional and combine OR-style when both are set:
// passing `{ key: "j", meta: true, ctrl: true }` matches Cmd-J on macOS AND
// Ctrl-J on Linux/Windows. That's the cross-platform default for app-wide
// shortcuts. If you want strictly Cmd-only or Ctrl-only, set just one.

import { useEffect } from "react";

export interface ShortcutSpec {
  key: string; // e.g. "j" — case-insensitive match against KeyboardEvent.key
  meta?: boolean; // Cmd on macOS
  ctrl?: boolean; // Ctrl on Linux/Win
  shift?: boolean;
}

/**
 * Pure predicate: does this keyboard event satisfy the shortcut spec?
 *
 * Extracted from the hook so it's exercisable in a node test environment
 * without a DOM. The hook just wires it to `window.addEventListener`.
 */
export function matchesShortcut(
  spec: ShortcutSpec,
  e: {
    key: string;
    metaKey: boolean;
    ctrlKey: boolean;
    shiftKey: boolean;
  },
): boolean {
  if (e.key.toLowerCase() !== spec.key.toLowerCase()) return false;

  // Modifier handling:
  // - When neither meta nor ctrl is requested, the event must have neither.
  // - When meta XOR ctrl is requested, that one must be pressed (the other is
  //   not checked — Cmd-Shift-J on a Cmd-only spec should still match).
  // - When both meta AND ctrl are requested, EITHER pressed counts (cross-
  //   platform mode).
  const wantMeta = !!spec.meta;
  const wantCtrl = !!spec.ctrl;
  if (!wantMeta && !wantCtrl) {
    if (e.metaKey || e.ctrlKey) return false;
  } else if (wantMeta && wantCtrl) {
    if (!e.metaKey && !e.ctrlKey) return false;
  } else if (wantMeta) {
    if (!e.metaKey) return false;
  } else if (wantCtrl) {
    if (!e.ctrlKey) return false;
  }

  // Shift is exact-match: spec.shift true requires shift down, spec.shift
  // unset/false requires shift up.
  if (!!spec.shift !== e.shiftKey) return false;
  return true;
}

export function useGlobalShortcut(
  spec: ShortcutSpec,
  handler: () => void,
): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!matchesShortcut(spec, e)) return;
      e.preventDefault();
      e.stopPropagation();
      handler();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [spec.key, spec.meta, spec.ctrl, spec.shift, handler]);
}
