import { describe, it, expect } from "vitest";
import {
  INITIAL_MENU_STATE,
  reduce,
  shouldOpenSlash,
  shouldOpenAt,
  extractQuery,
  type MenuItem,
} from "./useCommandMenu";

const item = (id: string, label = id): MenuItem => ({
  kind: "builtin",
  id,
  label,
});

describe("useCommandMenu reducer", () => {
  it("INITIAL_MENU_STATE is mode=idle with empty items", () => {
    expect(INITIAL_MENU_STATE.mode).toBe("idle");
    expect(INITIAL_MENU_STATE.items).toEqual([]);
    expect(INITIAL_MENU_STATE.selectedIndex).toBe(0);
    expect(INITIAL_MENU_STATE.historyIndex).toBeNull();
  });

  it("open-slash sets mode=slash, items, selectedIndex=0", () => {
    const items = [item("/new"), item("/cwd")];
    const s = reduce(INITIAL_MENU_STATE, { kind: "open-slash", items });
    expect(s.mode).toBe("slash");
    expect(s.items).toEqual(items);
    expect(s.selectedIndex).toBe(0);
  });

  it("open-at sets mode=at and resets state", () => {
    const items: MenuItem[] = [{ kind: "file", id: "/tmp/a", label: "a" }];
    const s = reduce(
      { ...INITIAL_MENU_STATE, selectedIndex: 5 },
      { kind: "open-at", items },
    );
    expect(s.mode).toBe("at");
    expect(s.selectedIndex).toBe(0);
  });

  it("move clamps to bounds (no wrap)", () => {
    const items = [item("a"), item("b"), item("c")];
    let s = reduce(INITIAL_MENU_STATE, { kind: "open-slash", items });
    s = reduce(s, { kind: "move", delta: -1 });
    expect(s.selectedIndex).toBe(0); // clamps at top
    s = reduce(s, { kind: "move", delta: 1 });
    s = reduce(s, { kind: "move", delta: 1 });
    s = reduce(s, { kind: "move", delta: 1 });
    expect(s.selectedIndex).toBe(2); // clamps at bottom
  });

  it("move on empty items keeps selectedIndex=0", () => {
    let s = reduce(INITIAL_MENU_STATE, { kind: "open-slash", items: [] });
    s = reduce(s, { kind: "move", delta: 1 });
    expect(s.selectedIndex).toBe(0);
  });

  it("filter updates query + items and resets selectedIndex to 0", () => {
    const start = [item("a"), item("b"), item("c")];
    let s = reduce(INITIAL_MENU_STATE, { kind: "open-slash", items: start });
    s = reduce(s, { kind: "move", delta: 1 });
    expect(s.selectedIndex).toBe(1);
    s = reduce(s, { kind: "filter", query: "x", items: [item("z")] });
    expect(s.query).toBe("x");
    expect(s.items).toEqual([item("z")]);
    expect(s.selectedIndex).toBe(0);
  });

  it("set-items preserves selectedIndex when possible, clamps when items shrink", () => {
    let s = reduce(INITIAL_MENU_STATE, {
      kind: "open-slash",
      items: [item("a"), item("b"), item("c")],
    });
    s = reduce(s, { kind: "move", delta: 1 });
    s = reduce(s, { kind: "move", delta: 1 });
    expect(s.selectedIndex).toBe(2);
    s = reduce(s, { kind: "set-items", items: [item("a")] });
    expect(s.selectedIndex).toBe(0);
  });

  it("open-history captures historyDraft and starts at index 0", () => {
    const s = reduce(INITIAL_MENU_STATE, {
      kind: "open-history",
      draft: "draft text",
    });
    expect(s.mode).toBe("history");
    expect(s.historyDraft).toBe("draft text");
    expect(s.historyIndex).toBe(0);
  });

  it("history-step clamps to history bounds", () => {
    let s = reduce(INITIAL_MENU_STATE, { kind: "open-history", draft: "" });
    // historyLen=3 → indices [0,1,2]
    s = reduce(s, { kind: "history-step", delta: 1, historyLen: 3 });
    expect(s.historyIndex).toBe(1);
    s = reduce(s, { kind: "history-step", delta: 1, historyLen: 3 });
    s = reduce(s, { kind: "history-step", delta: 1, historyLen: 3 });
    expect(s.historyIndex).toBe(2); // clamps
    s = reduce(s, { kind: "history-step", delta: -1, historyLen: 3 });
    expect(s.historyIndex).toBe(1);
  });

  it("history-step on empty history is a no-op", () => {
    let s = reduce(INITIAL_MENU_STATE, { kind: "open-history", draft: "x" });
    s = reduce(s, { kind: "history-step", delta: 1, historyLen: 0 });
    expect(s.historyIndex).toBe(0);
  });

  it("close resets to INITIAL_MENU_STATE", () => {
    let s = reduce(INITIAL_MENU_STATE, {
      kind: "open-slash",
      items: [item("a"), item("b")],
    });
    s = reduce(s, { kind: "move", delta: 1 });
    s = reduce(s, { kind: "close" });
    expect(s).toEqual(INITIAL_MENU_STATE);
  });

  it("unknown action returns state unchanged", () => {
    const fake = { kind: "absolutely_not_a_real_action" } as unknown as Parameters<
      typeof reduce
    >[1];
    expect(reduce(INITIAL_MENU_STATE, fake)).toBe(INITIAL_MENU_STATE);
  });
});

describe("trigger detection", () => {
  it("shouldOpenSlash: true on empty input + /", () => {
    expect(shouldOpenSlash("/", 1)).toBe(true);
  });

  it("shouldOpenSlash: true after newline", () => {
    expect(shouldOpenSlash("hi\n/", 4)).toBe(true);
  });

  it("shouldOpenSlash: false mid-line", () => {
    expect(shouldOpenSlash("hi /", 4)).toBe(false);
  });

  it("shouldOpenAt: true on empty + @", () => {
    expect(shouldOpenAt("@", 1)).toBe(true);
  });

  it("shouldOpenAt: true after space", () => {
    expect(shouldOpenAt("hi @", 4)).toBe(true);
  });

  it("shouldOpenAt: false when glued to a word", () => {
    expect(shouldOpenAt("hi@", 3)).toBe(false);
  });
});

describe("extractQuery", () => {
  it("returns the chars after the trigger", () => {
    expect(extractQuery("/cw", 3, "/")).toBe("cw");
  });

  it("returns empty string immediately after trigger", () => {
    expect(extractQuery("/", 1, "/")).toBe("");
  });

  it("returns null when caret moved before any trigger", () => {
    expect(extractQuery("hello", 5, "/")).toBeNull();
  });

  it("at-mention reads until the trigger, no whitespace allowed inside", () => {
    expect(extractQuery("hi @foo", 7, "@")).toBe("foo");
    expect(extractQuery("hi @foo bar", 11, "@")).toBeNull();
  });

  it("slash inside an at-mention does not register as slash mode", () => {
    // "@/tmp" — the slash sits inside a word, not at line-start
    expect(extractQuery("@/tmp", 5, "/")).toBeNull();
  });
});
