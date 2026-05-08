import { describe, it, expect } from "vitest";
import { applyEvent } from "./reducer";
import { INITIAL_STREAM_STATE, type AgentEvent, type AgentMessage } from "./types";

const msg = (text: string): AgentMessage => ({
  role: "user",
  content: [{ type: "text", text }],
  timestamp: 1,
});

describe("reducer (entry-id keyed)", () => {
  it("idempotent: same message_end event applied twice produces same state", () => {
    const event: AgentEvent = { type: "message_end", message: msg("hi"), entry_id: "e1" };
    const s1 = applyEvent(INITIAL_STREAM_STATE, event);
    const s2 = applyEvent(s1, event);
    expect(s2.messageMap.size).toBe(1);
    expect(s2.messageOrder).toEqual(["e1"]);
  });

  it("upsert: message_start then message_end with same entry_id merges to one entry", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, { type: "message_start", message: msg(""), entry_id: "e1" });
    s = applyEvent(s, { type: "message_end", message: msg("final"), entry_id: "e1" });
    expect(s.messageMap.size).toBe(1);
    expect(s.messageMap.get("e1")?.content).toEqual([{ type: "text", text: "final" }]);
  });

  it("late event for an already-snapshotted entry (entry_id < lastEntryId, never seen) is dropped", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, { type: "message_end", message: msg("first"), entry_id: "e2" });
    s = applyEvent(s, { type: "message_end", message: msg("late"), entry_id: "e1" });
    expect(s.messageMap.size).toBe(1);
    expect(s.messageMap.has("e1")).toBe(false);
  });

  it("out-of-order: message_end then message_start with same entry_id still produces correct final state", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, { type: "message_end", message: msg("done"), entry_id: "e1" });
    s = applyEvent(s, { type: "message_start", message: msg(""), entry_id: "e1" });
    expect(s.messageMap.get("e1")?.content).toEqual([{ type: "text", text: "done" }]);
  });

  it("agent_end self-heals — upserts any messages not seen via deltas", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, { type: "message_end", message: msg("a"), entry_id: "e1" });
    s = applyEvent(s, {
      type: "agent_end",
      messages: [
        { entry_id: "e1", message: msg("a") },
        { entry_id: "e2", message: msg("b") },
      ],
    });
    expect(s.messageMap.size).toBe(2);
    expect(s.messageMap.get("e2")?.content).toEqual([{ type: "text", text: "b" }]);
    expect(s.status).toBe("ended");
  });

  it("unknown event type does not throw, returns state unchanged", () => {
    const fake = { type: "absolutely_not_a_real_event" } as unknown as AgentEvent;
    expect(() => applyEvent(INITIAL_STREAM_STATE, fake)).not.toThrow();
    expect(applyEvent(INITIAL_STREAM_STATE, fake)).toBe(INITIAL_STREAM_STATE);
  });

  it("lastEntryId tracks max entry_id across upserts", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, { type: "message_end", message: msg("a"), entry_id: "e3" });
    s = applyEvent(s, { type: "message_end", message: msg("b"), entry_id: "e7" });
    s = applyEvent(s, { type: "message_end", message: msg("c"), entry_id: "e5" });
    expect(s.lastEntryId).toBe("e7");
  });

  it("approval_requested + approval_resolved manage pendingApprovals", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_requested",
      function_call_id: "t1",
      function_id: "shell::fs::write",
      args: {},
      expires_at: 0,
    });
    expect(s.pendingApprovals.length).toBe(1);
    expect(s.pendingApprovals[0].function_call_id).toBe("t1");
    s = applyEvent(s, {
      type: "approval_resolved",
      function_call_id: "t1",
      decision: "allow",
    });
    expect(s.pendingApprovals.length).toBe(0);
  });

  it("messages without entry_id land in unkeyedMessages (backwards compat)", () => {
    const e = { type: "message_end", message: msg("legacy") } as AgentEvent;
    const s = applyEvent(INITIAL_STREAM_STATE, e);
    expect(s.unkeyedMessages.length).toBe(1);
    expect(s.messageMap.size).toBe(0);
  });
});
