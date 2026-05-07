import { describe, it, expect, vi, beforeEach } from "vitest";
import type { AgentMessage } from "./types";

vi.mock("./bridge", () => ({
  bridge: vi.fn(),
  BridgeError: class BridgeError extends Error {},
}));

import { bridge } from "./bridge";
import { loadMessagesWithEntryIds } from "./loadMessages";

const bridgeMock = bridge as unknown as ReturnType<typeof vi.fn>;

const sampleMsg = (text: string): AgentMessage => ({
  role: "user",
  content: [{ type: "text", text }],
  timestamp: 1,
});

describe("loadMessagesWithEntryIds", () => {
  beforeEach(() => {
    bridgeMock.mockReset();
  });

  it("returns session-tree results when non-empty (entry_ids included)", async () => {
    bridgeMock.mockResolvedValueOnce({
      messages: [
        { entry_id: "e1", message: sampleMsg("hi") },
        { entry_id: "e2", message: sampleMsg("there") },
      ],
    });
    const result = await loadMessagesWithEntryIds("s1");
    expect(result).toEqual([
      { entry_id: "e1", message: sampleMsg("hi") },
      { entry_id: "e2", message: sampleMsg("there") },
    ]);
    expect(bridgeMock).toHaveBeenCalledTimes(1);
    expect(bridgeMock).toHaveBeenCalledWith("session-tree::messages", { session_id: "s1" });
  });

  it("falls back to state::get when session-tree returns empty AND state has data", async () => {
    bridgeMock.mockResolvedValueOnce({ messages: [] });
    bridgeMock.mockResolvedValueOnce([sampleMsg("only-in-state")]);
    const result = await loadMessagesWithEntryIds("s2");
    expect(result).toEqual([{ entry_id: null, message: sampleMsg("only-in-state") }]);
    expect(bridgeMock).toHaveBeenNthCalledWith(2, "state::get", {
      scope: "agent",
      key: "session/s2/messages",
    });
  });

  it("falls back to state::get when session-tree throws", async () => {
    bridgeMock.mockRejectedValueOnce(new Error("session-tree unreachable"));
    bridgeMock.mockResolvedValueOnce([sampleMsg("recovered")]);
    const result = await loadMessagesWithEntryIds("s3");
    expect(result).toEqual([{ entry_id: null, message: sampleMsg("recovered") }]);
  });

  it("returns empty when both stores are empty", async () => {
    bridgeMock.mockResolvedValueOnce({ messages: [] });
    bridgeMock.mockResolvedValueOnce(null);
    const result = await loadMessagesWithEntryIds("s4");
    expect(result).toEqual([]);
  });
});
