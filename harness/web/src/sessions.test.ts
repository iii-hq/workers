import { describe, it, expect } from "vitest";
import type { SessionRow } from "./types";
import { groupByDate, truncatePath } from "./sessions";

function row(id: string, updated_at_ms: number): SessionRow {
  return { session_id: id, state: "stopped", turn_count: 1, updated_at_ms };
}

describe("groupByDate", () => {
  // Anchor "now" at 2026-05-07 12:00 local time so the buckets are stable
  // regardless of when the test runs.
  const now = new Date(2026, 4, 7, 12, 0, 0).getTime();
  const startToday = new Date(2026, 4, 7, 0, 0, 0).getTime();
  const startYesterday = startToday - 24 * 60 * 60 * 1000;

  it("buckets today by start-of-day boundary", () => {
    const rows = [
      row("today-noon", now),
      row("today-midnight", startToday),
      row("yesterday-late", startToday - 1),
    ];
    const result = groupByDate(rows, now);
    expect(result.today.map((r) => r.session_id)).toEqual([
      "today-noon",
      "today-midnight",
    ]);
    expect(result.yesterday.map((r) => r.session_id)).toEqual(["yesterday-late"]);
    expect(result.earlier).toEqual([]);
  });

  it("buckets yesterday by previous start-of-day", () => {
    const rows = [
      row("yesterday-noon", startYesterday + 12 * 60 * 60 * 1000),
      row("yesterday-edge", startYesterday),
      row("two-days-ago", startYesterday - 1),
    ];
    const result = groupByDate(rows, now);
    expect(result.yesterday.map((r) => r.session_id)).toEqual([
      "yesterday-noon",
      "yesterday-edge",
    ]);
    expect(result.earlier.map((r) => r.session_id)).toEqual(["two-days-ago"]);
  });

  it("preserves input order within each bucket", () => {
    const rows = [row("a", now), row("b", now - 1), row("c", now - 2)];
    const result = groupByDate(rows, now);
    expect(result.today.map((r) => r.session_id)).toEqual(["a", "b", "c"]);
  });

  it("returns empty buckets for empty input", () => {
    expect(groupByDate([], now)).toEqual({ today: [], yesterday: [], earlier: [] });
  });
});

describe("truncatePath", () => {
  it("returns path unchanged when within limit", () => {
    expect(truncatePath("/short", 12)).toBe("/short");
    expect(truncatePath("/twelve/char", 12)).toBe("/twelve/char");
  });

  it("truncates from the left with `…/` prefix", () => {
    expect(truncatePath("/very/long/path/to/file", 12)).toBe("…/th/to/file");
  });

  it("preserves the tail (rightmost characters)", () => {
    const out = truncatePath("/users/yt/workspaces/personal/motia/workers", 20);
    expect(out.startsWith("…/")).toBe(true);
    expect(out.endsWith("workers")).toBe(true);
    expect(out.length).toBe(20);
  });

  it("degenerates safely when max is tiny", () => {
    expect(truncatePath("/abcdef", 2)).toBe("ef");
    expect(truncatePath("/abcdef", 1)).toBe("f");
  });
});
