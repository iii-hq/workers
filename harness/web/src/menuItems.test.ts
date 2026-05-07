import { describe, it, expect } from "vitest";
import {
  BUILT_IN_COMMANDS,
  filterCommands,
  skillsIndexToMenuItems,
} from "./menuItems";
import type { MenuItem } from "./useCommandMenu";

const item = (id: string, label = id): MenuItem => ({
  kind: "builtin",
  id,
  label,
});

describe("filterCommands", () => {
  it("empty query returns all items in original order", () => {
    const result = filterCommands(BUILT_IN_COMMANDS, "");
    expect(result.map((x) => x.id)).toEqual(BUILT_IN_COMMANDS.map((x) => x.id));
  });

  it("ranks exact-prefix matches first", () => {
    const items = [item("/clear"), item("/cwd"), item("/help")];
    const result = filterCommands(items, "/c");
    // Both /clear and /cwd are prefix matches; original order preserved by stable sort.
    expect(result[0].id).toBe("/clear");
    expect(result[1].id).toBe("/cwd");
    expect(result.find((x) => x.id === "/help")).toBeUndefined();
  });

  it("substring beats fuzzy", () => {
    const substring = item("/xfoobar"); // contains "foo"
    const fuzzy = item("/fxoxo"); // matches f-o-o as fuzzy but no "foo" substring
    const items = [fuzzy, substring];
    const result = filterCommands(items, "foo");
    expect(result[0].id).toBe("/xfoobar");
  });

  it("below-threshold matches drop out (no fuzzy on label fallback for nonsense)", () => {
    const items = [item("/foo")];
    const result = filterCommands(items, "xyz");
    expect(result).toEqual([]);
  });

  it("fuzzy on id matches in-order chars", () => {
    const items = [item("/provider"), item("/help")];
    const result = filterCommands(items, "pvr");
    expect(result.map((x) => x.id)).toEqual(["/provider"]);
  });
});

describe("skillsIndexToMenuItems", () => {
  it("returns [] for null", () => {
    expect(skillsIndexToMenuItems(null)).toEqual([]);
  });

  it("returns [] for empty string", () => {
    expect(skillsIndexToMenuItems("")).toEqual([]);
  });

  it("parses well-formed lines with em-dash", () => {
    const md = [
      "# skills",
      "",
      "- [tdd](iii://skills/tdd) — Write tests first",
      "- [refactor](iii://skills/refactor) — Clean up dead code",
      "",
    ].join("\n");
    const out = skillsIndexToMenuItems(md);
    expect(out.length).toBe(2);
    expect(out[0].kind).toBe("skill");
    expect(out[0].id).toBe("/tdd");
    expect(out[0].label).toBe("/tdd");
    expect(out[0].description).toContain("Write tests first");
    expect((out[0].meta as { uri: string }).uri).toBe("iii://skills/tdd");
  });

  it("parses lines with plain hyphen separator", () => {
    const md = "- [foo](iii://skills/foo) - description here";
    const out = skillsIndexToMenuItems(md);
    expect(out.length).toBe(1);
    expect(out[0].id).toBe("/foo");
  });

  it("skips non-skill lines silently", () => {
    const md = [
      "Some intro paragraph",
      "- not a skill link",
      "- [valid](iii://skills/valid) — yes",
      "- [external](https://example.com) — no",
    ].join("\n");
    const out = skillsIndexToMenuItems(md);
    expect(out.length).toBe(1);
    expect(out[0].id).toBe("/valid");
  });
});
