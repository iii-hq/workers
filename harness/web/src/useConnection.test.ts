// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import type { IIIConnectionState } from "iii-browser-sdk";

import {
  __resetIiiClientForTests,
  __setIiiClientDepsForTests,
  disposeIiiClient,
} from "./iii-client";
import { useConnection } from "./useConnection";

interface FakeSdk {
  trigger: ReturnType<typeof vi.fn>;
  registerFunction: ReturnType<typeof vi.fn>;
  addConnectionStateListener: ReturnType<typeof vi.fn>;
  shutdown: ReturnType<typeof vi.fn>;
}

describe("useConnection", () => {
  let sdk: FakeSdk;
  let connectionListeners: Array<(s: IIIConnectionState) => void>;
  let unsubMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    connectionListeners = [];
    unsubMock = vi.fn();
    sdk = {
      trigger: vi.fn(),
      registerFunction: vi.fn(),
      addConnectionStateListener: vi.fn((handler) => {
        connectionListeners.push(handler);
        // Mimic the sdk's behaviour: fire immediately with current state.
        handler("connecting");
        return unsubMock;
      }),
      shutdown: vi.fn().mockResolvedValue(undefined),
    };

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

  it("starts disconnected then transitions when the sdk reports connecting/connected", async () => {
    const { result } = renderHook(() => useConnection());

    // Pre-bootstrap, hook returns the initial "disconnected" sentinel.
    expect(result.current.status).toBe("disconnected");

    // Let the bootstrap microtasks resolve and the immediate-fire handler run.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(result.current.status).toBe("connecting");
    const t1 = result.current.since;
    expect(t1).toBeGreaterThan(0);

    await act(async () => {
      connectionListeners[0]("connected");
    });
    expect(result.current.status).toBe("connected");
    expect(result.current.since).toBeGreaterThanOrEqual(t1);
  });

  it("unsubscribes on unmount", async () => {
    const { unmount } = renderHook(() => useConnection());
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(unsubMock).not.toHaveBeenCalled();
    unmount();
    expect(unsubMock).toHaveBeenCalledTimes(1);
  });
});
