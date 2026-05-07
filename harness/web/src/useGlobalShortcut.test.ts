import { describe, it, expect } from "vitest";
import { matchesShortcut } from "./useGlobalShortcut";

const ev = (over: Partial<{ key: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean }> = {}) => ({
  key: "j",
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  ...over,
});

describe("matchesShortcut", () => {
  it("fires on Cmd-J when both meta and ctrl are requested", () => {
    expect(
      matchesShortcut({ key: "j", meta: true, ctrl: true }, ev({ metaKey: true })),
    ).toBe(true);
  });

  it("fires on Ctrl-J when both meta and ctrl are requested", () => {
    expect(
      matchesShortcut({ key: "j", meta: true, ctrl: true }, ev({ ctrlKey: true })),
    ).toBe(true);
  });

  it("does not fire on Cmd-K (different key)", () => {
    expect(
      matchesShortcut(
        { key: "j", meta: true, ctrl: true },
        ev({ key: "k", metaKey: true }),
      ),
    ).toBe(false);
  });

  it("does not fire when modifier is missing", () => {
    expect(
      matchesShortcut({ key: "j", meta: true, ctrl: true }, ev()),
    ).toBe(false);
  });

  it("does not fire on bare J (no modifier requested) when meta is down", () => {
    // Spec has no meta/ctrl: pressing the key with a modifier should NOT match.
    expect(matchesShortcut({ key: "j" }, ev({ metaKey: true }))).toBe(false);
  });

  it("matches case-insensitively", () => {
    expect(
      matchesShortcut(
        { key: "J", meta: true, ctrl: true },
        ev({ key: "j", metaKey: true }),
      ),
    ).toBe(true);
  });

  it("respects shift: false requires shift up", () => {
    expect(
      matchesShortcut(
        { key: "j", meta: true, ctrl: true },
        ev({ metaKey: true, shiftKey: true }),
      ),
    ).toBe(false);
  });

  it("respects shift: true requires shift down", () => {
    expect(
      matchesShortcut(
        { key: "j", meta: true, ctrl: true, shift: true },
        ev({ metaKey: true, shiftKey: true }),
      ),
    ).toBe(true);
  });

  it("meta-only spec ignores ctrl", () => {
    expect(
      matchesShortcut({ key: "j", meta: true }, ev({ ctrlKey: true })),
    ).toBe(false);
    expect(
      matchesShortcut({ key: "j", meta: true }, ev({ metaKey: true })),
    ).toBe(true);
  });
});
