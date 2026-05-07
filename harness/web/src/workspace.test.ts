import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("./bridge", () => ({
  bridge: vi.fn(),
  BridgeError: class BridgeError extends Error {},
}));

import { bridge } from "./bridge";
import { loadWorkspace, saveWorkspace, type Workspace } from "./workspace";

const bridgeMock = bridge as unknown as ReturnType<typeof vi.fn>;

describe("workspace", () => {
  beforeEach(() => {
    bridgeMock.mockReset();
  });

  it("loadWorkspace returns null when state has no value", async () => {
    bridgeMock.mockResolvedValueOnce(null);
    expect(await loadWorkspace("s1")).toBeNull();
    expect(bridgeMock).toHaveBeenCalledWith("state::get", {
      scope: "agent",
      key: "session/s1/workspace",
    });
  });

  it("loadWorkspace returns the stored Workspace shape when present", async () => {
    const ws: Workspace = { cwd: "/tmp/foo", set_at: 1234 };
    bridgeMock.mockResolvedValueOnce(ws);
    expect(await loadWorkspace("s1")).toEqual(ws);
  });

  it("loadWorkspace returns null on bridge error (advisory, never throws)", async () => {
    bridgeMock.mockRejectedValueOnce(new Error("bridge down"));
    expect(await loadWorkspace("s1")).toBeNull();
  });

  it("loadWorkspace returns null when shape is malformed", async () => {
    bridgeMock.mockResolvedValueOnce({ random: "junk" });
    expect(await loadWorkspace("s1")).toBeNull();
  });

  it("saveWorkspace writes state::set with set_at timestamp", async () => {
    bridgeMock.mockResolvedValueOnce(undefined);
    const before = Date.now();
    await saveWorkspace("s1", "/tmp/foo");
    const after = Date.now();
    expect(bridgeMock).toHaveBeenCalledWith("state::set", {
      scope: "agent",
      key: "session/s1/workspace",
      value: expect.objectContaining({
        cwd: "/tmp/foo",
        set_at: expect.any(Number),
      }),
    });
    const call = bridgeMock.mock.calls[0][1] as { value: Workspace };
    expect(call.value.set_at).toBeGreaterThanOrEqual(before);
    expect(call.value.set_at).toBeLessThanOrEqual(after);
  });
});
