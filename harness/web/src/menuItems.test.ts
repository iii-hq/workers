import { describe, it, expect } from "vitest";
import {
  BUILT_IN_COMMANDS,
  filterCommands,
  skillsListToMenuItems,
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

describe("skillsListToMenuItems", () => {
  it("returns [] for null", () => {
    expect(skillsListToMenuItems(null)).toEqual([]);
  });

  it("returns [] for undefined", () => {
    expect(skillsListToMenuItems(undefined)).toEqual([]);
  });

  it("returns [] for empty array", () => {
    expect(skillsListToMenuItems([])).toEqual([]);
  });

  it("projects rows into /id menu items with secondary 'title — description'", () => {
    const out = skillsListToMenuItems([
      { id: "tdd", title: "Write tests first", description: "Red, green, refactor." },
      { id: "refactor", title: "Refactor", description: "Clean up dead code" },
    ]);
    expect(out.length).toBe(2);
    expect(out[0].kind).toBe("skill");
    expect(out[0].id).toBe("/tdd");
    expect(out[0].label).toBe("/tdd");
    expect(out[0].description).toBe("Write tests first — Red, green, refactor.");
    expect((out[0].meta as { id: string; uri: string })).toEqual({
      id: "tdd",
      uri: "iii://tdd",
    });
  });

  it("falls back to id when title is missing or blank", () => {
    const out = skillsListToMenuItems([
      { id: "shell" },
      { id: "harness", title: "   " },
    ]);
    expect(out[0].description).toBe("shell");
    expect(out[1].description).toBe("harness");
  });

  it("omits the trailing em-dash when description is empty", () => {
    const out = skillsListToMenuItems([{ id: "x", title: "X" }]);
    expect(out[0].description).toBe("X");
  });

  it("preserves nested ids in label and uri", () => {
    const out = skillsListToMenuItems([
      { id: "resend/email/send", title: "send", description: "Send an email" },
    ]);
    expect(out[0].id).toBe("/resend/email/send");
    expect((out[0].meta as { uri: string }).uri).toBe("iii://resend/email/send");
  });

  it("drops rows whose id is missing or blank", () => {
    const out = skillsListToMenuItems([
      { id: "ok", title: "ok" },
      { id: "", title: "blank" },
      { id: "   ", title: "spaces" },
      // @ts-expect-error — explicitly testing the runtime guard
      { title: "no id" },
    ]);
    expect(out.length).toBe(1);
    expect(out[0].id).toBe("/ok");
  });
});
