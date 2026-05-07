// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  __resetIiiClientForTests,
  __setIiiClientDepsForTests,
  disposeIiiClient,
  getIiiClient,
} from "./iii-client";

interface FakeSdk {
  trigger: ReturnType<typeof vi.fn>;
  registerFunction: ReturnType<typeof vi.fn>;
  addConnectionStateListener: ReturnType<typeof vi.fn>;
  shutdown: ReturnType<typeof vi.fn>;
}

function makeFakeSdk(): FakeSdk {
  return {
    trigger: vi.fn().mockResolvedValue({ ok: true }),
    registerFunction: vi.fn((id: string) => ({
      id,
      unregister: vi.fn(),
    })),
    addConnectionStateListener: vi.fn().mockReturnValue(() => undefined),
    shutdown: vi.fn().mockResolvedValue(undefined),
  };
}

describe("iii-client", () => {
  let sdk: FakeSdk;

  beforeEach(() => {
    sdk = makeFakeSdk();
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

  it("returns a singleton across calls", async () => {
    const a = await getIiiClient();
    const b = await getIiiClient();
    expect(a).toBe(b);
    expect(a.browserId).toBe("harness-test-id");
  });

  it("call() forwards to sdk.trigger with function_id + payload", async () => {
    const client = await getIiiClient();
    sdk.trigger.mockResolvedValueOnce({ value: 42 });
    const result = await client.call<{ value: number }>("models::list", {
      foo: "bar",
    });
    expect(result).toEqual({ value: 42 });
    expect(sdk.trigger).toHaveBeenCalledWith({
      function_id: "models::list",
      payload: { foo: "bar" },
    });
  });

  it("call() defaults payload to {} when omitted", async () => {
    const client = await getIiiClient();
    await client.call("ping");
    expect(sdk.trigger).toHaveBeenCalledWith({
      function_id: "ping",
      payload: {},
    });
  });

  it("on() registers handler under <functionId>::<browserId>", async () => {
    const client = await getIiiClient();
    const handler = vi.fn();
    client.on("ui::session::event", handler);
    expect(sdk.registerFunction).toHaveBeenCalledTimes(1);
    expect(sdk.registerFunction.mock.calls[0][0]).toBe(
      "ui::session::event::harness-test-id",
    );
    // Invoke the wrapped handler the SDK would call.
    const wrapped = sdk.registerFunction.mock.calls[0][1] as (
      data: unknown,
    ) => Promise<unknown>;
    const out = await wrapped({ session_id: "s1", text: "hi" });
    expect(handler).toHaveBeenCalledWith({ session_id: "s1", text: "hi" });
    // Wrapped handler returns null so the engine doesn't expect a payload.
    expect(out).toBeNull();
  });

  it("on() returns an unregister fn that calls ref.unregister", async () => {
    const ref = { id: "x", unregister: vi.fn() };
    sdk.registerFunction.mockReturnValueOnce(ref);
    const client = await getIiiClient();
    const off = client.on("ui::sessions::changed", () => {});
    off();
    expect(ref.unregister).toHaveBeenCalledTimes(1);
    // Idempotent — calling twice doesn't throw or double-unregister.
    off();
    expect(ref.unregister).toHaveBeenCalledTimes(1);
  });

  it("dispose() unregisters every active handler then calls sdk.shutdown", async () => {
    const ref1 = { id: "a", unregister: vi.fn() };
    const ref2 = { id: "b", unregister: vi.fn() };
    sdk.registerFunction.mockReturnValueOnce(ref1).mockReturnValueOnce(ref2);
    const client = await getIiiClient();
    client.on("ui::session::event", () => {});
    client.on("ui::sessions::changed", () => {});
    await disposeIiiClient();
    expect(ref1.unregister).toHaveBeenCalledTimes(1);
    expect(ref2.unregister).toHaveBeenCalledTimes(1);
    expect(sdk.shutdown).toHaveBeenCalledTimes(1);
  });

  it("addConnectionStateListener proxies to the sdk", async () => {
    const sub = vi.fn();
    const unsub = vi.fn();
    sdk.addConnectionStateListener.mockReturnValueOnce(unsub);
    const client = await getIiiClient();
    const off = client.addConnectionStateListener(sub);
    expect(sdk.addConnectionStateListener).toHaveBeenCalledWith(sub);
    off();
    expect(unsub).toHaveBeenCalled();
  });
});
