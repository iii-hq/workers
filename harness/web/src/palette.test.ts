import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  type FunctionEntry,
  type RecentInvocation,
  curlForBridge,
  filterPalette,
  isSensitive,
  loadRecent,
  pushRecent,
  workerFromId,
} from "./palette";

// ---- minimal localStorage shim for the node test env ----
//
// vitest runs with `environment: "node"` (see vitest.config.ts), so there's
// no global `localStorage`. The palette helpers gracefully degrade when it's
// missing, which we test explicitly below — but we also need a working shim
// for the round-trip tests on pushRecent / loadRecent.
class MemoryStorage {
  private store = new Map<string, string>();
  getItem(k: string): string | null {
    return this.store.has(k) ? (this.store.get(k) as string) : null;
  }
  setItem(k: string, v: string): void {
    this.store.set(k, v);
  }
  removeItem(k: string): void {
    this.store.delete(k);
  }
  clear(): void {
    this.store.clear();
  }
}

const installStorage = () => {
  (globalThis as unknown as { localStorage: MemoryStorage }).localStorage =
    new MemoryStorage();
};
const uninstallStorage = () => {
  delete (globalThis as unknown as { localStorage?: unknown }).localStorage;
};

describe("isSensitive", () => {
  it.each([
    ["policy::deny", true],
    ["policy::list", true],
    ["auth::set_token", true],
    ["auth::set_anything", true],
    ["shell::filesystem::write", true],
    ["shell::filesystem::mkdir", true],
    ["shell::filesystem::rm", true],
    ["shell::filesystem::ls", false],
    ["shell::filesystem::read", false],
    ["state::get", false],
    ["harness::status", false],
    ["auth::status", false],
    ["models::list", false],
  ])("%s -> %s", (id, expected) => {
    expect(isSensitive(id)).toBe(expected);
  });
});

describe("workerFromId", () => {
  it("returns prefix before ::", () => {
    expect(workerFromId("session-tree::list")).toBe("session-tree");
    expect(workerFromId("shell::filesystem::ls")).toBe("shell");
  });
  it("returns (unknown) when no separator", () => {
    expect(workerFromId("flat")).toBe("(unknown)");
    expect(workerFromId("")).toBe("(unknown)");
  });
});

describe("filterPalette", () => {
  const fn = (function_id: string, description = ""): FunctionEntry => ({
    function_id,
    description,
  });

  it("empty query returns items in original order", () => {
    const items = [fn("a::x"), fn("b::y")];
    expect(filterPalette(items, "").map((i) => i.function_id)).toEqual([
      "a::x",
      "b::y",
    ]);
  });

  it("ranks substring on id ahead of substring on description", () => {
    const idMatch = fn("harness::status", "completely unrelated text");
    const descMatch = fn("session-tree::list", "harness related description");
    const result = filterPalette([descMatch, idMatch], "harness");
    expect(result[0].function_id).toBe("harness::status");
    expect(result[1].function_id).toBe("session-tree::list");
  });

  it("ranks prefix above substring", () => {
    const prefix = fn("foo::bar");
    const substring = fn("xx::xfooz");
    const result = filterPalette([substring, prefix], "foo");
    expect(result[0].function_id).toBe("foo::bar");
  });

  it("drops below-threshold entries", () => {
    const items = [fn("alpha::beta", "gamma")];
    expect(filterPalette(items, "zzz")).toEqual([]);
  });

  it("matches on worker prefix", () => {
    const items = [fn("session-tree::list"), fn("harness::status")];
    const result = filterPalette(items, "session");
    expect(result[0].function_id).toBe("session-tree::list");
  });
});

describe("loadRecent", () => {
  afterEach(() => {
    uninstallStorage();
  });

  it("returns [] when localStorage is undefined", () => {
    uninstallStorage();
    expect(loadRecent()).toEqual([]);
  });

  it("returns [] when key is missing", () => {
    installStorage();
    expect(loadRecent()).toEqual([]);
  });

  it("returns [] when stored value is invalid JSON", () => {
    installStorage();
    localStorage.setItem("harness.palette.recent", "{not json");
    expect(loadRecent()).toEqual([]);
  });

  it("returns [] when stored value is not an array", () => {
    installStorage();
    localStorage.setItem("harness.palette.recent", JSON.stringify({}));
    expect(loadRecent()).toEqual([]);
  });

  it("filters out malformed entries", () => {
    installStorage();
    const mixed = [
      { function_id: "a::b", payload: {}, ts: 1, ok: true },
      { function_id: 42 }, // wrong type
      null,
      { function_id: "c::d", payload: null, ts: 2, ok: false },
    ];
    localStorage.setItem("harness.palette.recent", JSON.stringify(mixed));
    const out = loadRecent();
    expect(out.length).toBe(2);
    expect(out[0].function_id).toBe("a::b");
    expect(out[1].function_id).toBe("c::d");
  });
});

describe("pushRecent", () => {
  beforeEach(() => {
    installStorage();
  });
  afterEach(() => {
    uninstallStorage();
  });

  const inv = (id: string, ts: number): RecentInvocation => ({
    function_id: id,
    payload: {},
    ts,
    ok: true,
  });

  it("dedupes by function_id, keeping the new entry at the head", () => {
    pushRecent(inv("a::x", 1));
    pushRecent(inv("b::y", 2));
    const after = pushRecent(inv("a::x", 3));
    expect(after.map((r) => r.function_id)).toEqual(["a::x", "b::y"]);
    expect(after[0].ts).toBe(3);
  });

  it("caps at 20 entries", () => {
    for (let i = 0; i < 25; i++) {
      pushRecent(inv(`fn${i}::x`, i));
    }
    const final = loadRecent();
    expect(final.length).toBe(20);
    // Newest first
    expect(final[0].function_id).toBe("fn24::x");
    expect(final[19].function_id).toBe("fn5::x");
  });

  it("survives setItem throwing (quota exceeded)", () => {
    const throwing = {
      getItem: () => null,
      setItem: () => {
        throw new Error("QuotaExceededError");
      },
      removeItem: () => undefined,
      clear: () => undefined,
    };
    (globalThis as unknown as { localStorage: typeof throwing }).localStorage =
      throwing;
    // Must not throw
    expect(() => pushRecent(inv("a::b", 1))).not.toThrow();
  });
});

describe("curlForBridge", () => {
  it("builds a POST against /bridge/trigger with the function_id + payload", () => {
    const out = curlForBridge("models::list", {});
    expect(out).toContain("curl -X POST http://127.0.0.1:3111/bridge/trigger");
    expect(out).toContain("'content-type: application/json'");
    expect(out).toContain('"function_id": "models::list"');
    expect(out).toContain('"payload": {}');
  });

  it("escapes single quotes in payload values", () => {
    const out = curlForBridge("shell::run", { cmd: "echo 'hi'" });
    // The single quote inside the JSON body must be escaped using the
    // standard '\'' shell-quoting trick so the command can be pasted into a
    // real shell.
    expect(out).toContain("'\\''hi'\\''");
  });
});
