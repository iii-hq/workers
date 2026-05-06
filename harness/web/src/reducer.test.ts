import { describe, expect, it } from "vitest";
import { applyEvent } from "./reducer";
import { INITIAL_STREAM_STATE, type AgentEvent, type AgentMessage } from "./types";

const userMsg = (text: string): AgentMessage => ({
  role: "user",
  content: [{ type: "text", text }],
  timestamp: 0,
});

describe("applyEvent", () => {
  it("agent_start sets status to running", () => {
    const next = applyEvent(INITIAL_STREAM_STATE, { type: "agent_start" });
    expect(next.status).toBe("running");
  });

  it("agent_end sets status to ended", () => {
    const next = applyEvent(
      { ...INITIAL_STREAM_STATE, status: "running" },
      { type: "agent_end", messages: [] },
    );
    expect(next.status).toBe("ended");
  });

  it("message_end appends the message", () => {
    const m = userMsg("hi");
    const next = applyEvent(INITIAL_STREAM_STATE, { type: "message_end", message: m });
    expect(next.messages).toHaveLength(1);
    expect(next.messages[0]).toEqual(m);
  });

  it("duplicate message_end with same role+timestamp does not append twice", () => {
    const m = userMsg("hi");
    const s1 = applyEvent(INITIAL_STREAM_STATE, { type: "message_end", message: m });
    const s2 = applyEvent(s1, { type: "message_end", message: m });
    expect(s2.messages).toHaveLength(1);
  });

  it("approval_requested adds a pending entry", () => {
    const next = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_requested",
      tool_call_id: "tc-1",
      tool_name: "shell::filesystem::write",
      args: { path: "/tmp/x" },
      expires_at: 0,
    });
    expect(next.pendingApprovals).toHaveLength(1);
    expect(next.pendingApprovals[0].tool_call_id).toBe("tc-1");
  });

  it("approval_resolved clears the matching pending entry", () => {
    const seeded = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_requested",
      tool_call_id: "tc-1",
      tool_name: "x",
      args: {},
      expires_at: 0,
    });
    const next = applyEvent(seeded, {
      type: "approval_resolved",
      tool_call_id: "tc-1",
      decision: "allow",
    });
    expect(next.pendingApprovals).toHaveLength(0);
  });

  it("approval_resolved before its requested is a no-op (replay)", () => {
    const next = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_resolved",
      tool_call_id: "tc-1",
      decision: "deny",
    });
    expect(next.pendingApprovals).toHaveLength(0);
  });

  it("unknown event variants pass through unchanged", () => {
    const unknown = { type: "totally_made_up" } as unknown as AgentEvent;
    const next = applyEvent(INITIAL_STREAM_STATE, unknown);
    expect(next).toBe(INITIAL_STREAM_STATE);
  });
});
