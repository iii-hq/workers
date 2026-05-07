// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";

import {
  __resetIiiClientForTests,
  __setIiiClientDepsForTests,
  disposeIiiClient,
} from "./iii-client";
import { useStatus } from "./useStatus";

interface FakeSdk {
  trigger: ReturnType<typeof vi.fn>;
  registerFunction: ReturnType<typeof vi.fn>;
  addConnectionStateListener: ReturnType<typeof vi.fn>;
  shutdown: ReturnType<typeof vi.fn>;
}

type Handlers = Record<string, (payload: unknown) => Promise<unknown> | void>;

function makeSdk(handlers: Handlers): FakeSdk {
  return {
    trigger: vi.fn().mockResolvedValue({ ok: true }),
    registerFunction: vi.fn(
      (id: string, fn: (payload: unknown) => Promise<unknown> | void) => {
        handlers[id] = fn;
        return {
          id,
          unregister: vi.fn(() => {
            delete handlers[id];
          }),
        };
      },
    ),
    addConnectionStateListener: vi.fn().mockReturnValue(() => undefined),
    shutdown: vi.fn().mockResolvedValue(undefined),
  };
}

describe("useStatus", () => {
  let handlers: Handlers;
  let sdk: FakeSdk;

  beforeEach(() => {
    handlers = {};
    sdk = makeSdk(handlers);
    __setIiiClientDepsForTests({
      fetchBridgeInfo: vi.fn().mockResolvedValue({
        ws_path: "/iii/ws",
        protocol: "ws" as const,
        engine_url: "ws://127.0.0.1:49134",
      }),
      makeBrowserId: () => "harness-test-id",
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      registerWorker: () => sdk as any,
      composeWsUrl: () => "ws://127.0.0.1:49134",
    });
  });

  afterEach(async () => {
    await disposeIiiClient();
    __resetIiiClientForTests();
  });

  async function flush() {
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  async function fire(topic: string, payload: unknown) {
    const fn = handlers[`${topic}::harness-test-id`];
    expect(fn, `handler for ${topic} should be registered`).toBeDefined();
    await act(async () => {
      await fn(payload);
    });
  }

  it("subscribes to ui::* topics and calls ui::subscribe with browser_id", async () => {
    renderHook(() => useStatus());
    await flush();

    const ids = sdk.registerFunction.mock.calls.map((c) => c[0]);
    expect(ids).toEqual(
      expect.arrayContaining([
        "ui::approval::requested::harness-test-id",
        "ui::approval::resolved::harness-test-id",
        "ui::cost::tick::harness-test-id",
        "ui::workers::changed::harness-test-id",
      ]),
    );
    expect(sdk.trigger).toHaveBeenCalledWith({
      function_id: "ui::subscribe",
      payload: { browser_id: "harness-test-id", session_id: null },
    });
  });

  it("adds and removes pending approvals across requested/resolved", async () => {
    const { result } = renderHook(() => useStatus());
    await flush();

    await fire("ui::approval::requested", {
      tool_call_id: "tc-1",
      tool_name: "shell::filesystem::write",
      args: { path: "/tmp/x" },
      expires_at: 9999,
      session_id: "s1",
    });
    expect(result.current.pendingApprovals).toHaveLength(1);
    expect(result.current.pendingApprovals[0].tool_call_id).toBe("tc-1");

    await fire("ui::approval::resolved", {
      tool_call_id: "tc-1",
      decision: "allow",
    });
    expect(result.current.pendingApprovals).toHaveLength(0);
  });

  it("dedupes a duplicate approval requested push", async () => {
    const { result } = renderHook(() => useStatus());
    await flush();

    await fire("ui::approval::requested", {
      tool_call_id: "tc-1",
      tool_name: "x",
    });
    await fire("ui::approval::requested", {
      tool_call_id: "tc-1",
      tool_name: "x",
    });
    expect(result.current.pendingApprovals).toHaveLength(1);
  });

  it("replaces cost snapshot on each tick", async () => {
    const { result } = renderHook(() => useStatus());
    await flush();

    await fire("ui::cost::tick", {
      usd_today: 1.5,
      by_provider: { anthropic: 1.0, openai: 0.5 },
    });
    expect(result.current.cost.usd_today).toBe(1.5);

    await fire("ui::cost::tick", {
      usd_today: 2.0,
      by_provider: {},
    });
    expect(result.current.cost.usd_today).toBe(2.0);
  });

  it("ignores malformed cost ticks", async () => {
    const { result } = renderHook(() => useStatus());
    await flush();

    await fire("ui::cost::tick", { not: "a cost tick" });
    expect(result.current.cost.usd_today).toBe(0);
  });

  it("replaces workers snapshot on each push", async () => {
    const { result } = renderHook(() => useStatus());
    await flush();

    await fire("ui::workers::changed", {
      up: 5,
      down: 1,
      total: 6,
      workers: [{ name: "harness", status: "up" }],
    });
    expect(result.current.workers.up).toBe(5);
    expect(result.current.workers.workers).toHaveLength(1);
  });

  it("buffers events and caps at 200 entries", async () => {
    const { result } = renderHook(() => useStatus());
    await flush();

    for (let i = 0; i < 250; i++) {
      await fire("ui::cost::tick", {
        usd_today: i,
        by_provider: {},
      });
    }
    expect(result.current.events.length).toBe(200);
    // Newest is preserved at the tail.
    const last = result.current.events[result.current.events.length - 1];
    expect((last.payload as { usd_today: number }).usd_today).toBe(249);
  });

  it("clearEvents empties the rolling buffer", async () => {
    const { result } = renderHook(() => useStatus());
    await flush();

    await fire("ui::cost::tick", { usd_today: 1, by_provider: {} });
    expect(result.current.events).toHaveLength(1);
    await act(async () => {
      result.current.clearEvents();
    });
    expect(result.current.events).toHaveLength(0);
  });

  it("flips hydrated=true after the first push of any kind", async () => {
    const { result } = renderHook(() => useStatus());
    await flush();
    expect(result.current.hydrated).toBe(false);
    await fire("ui::workers::changed", {
      up: 1,
      down: 0,
      total: 1,
      workers: [],
    });
    expect(result.current.hydrated).toBe(true);
  });

  it("unsubscribes on unmount", async () => {
    const { unmount } = renderHook(() => useStatus());
    await flush();
    sdk.trigger.mockClear();
    unmount();
    // After unmount, the cleanup fires a follow-up ui::unsubscribe call.
    await flush();
    expect(sdk.trigger).toHaveBeenCalledWith({
      function_id: "ui::unsubscribe",
      payload: { browser_id: "harness-test-id", session_id: null },
    });
  });
});
