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

  // Cascade: operator clicks "allow + always" on one card; the backend
  // resolves the originator AND every other pending row with the same
  // argv shape, emitting one approval_resolved event per cascaded row
  // (see approval-gate/src/register.rs::emit_approval_resolved). The
  // reducer must drain BOTH cards from pendingApprovals — otherwise the
  // operator sees stale cards lingering after the backend already
  // executed/denied the underlying calls.
  it("cascade: allow+always resolves originator and cascaded pending cards", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_requested",
      function_call_id: "t1",
      function_id: "shell::exec",
      args: { command: "echo", args: ["go"] },
      expires_at: 0,
    });
    s = applyEvent(s, {
      type: "approval_requested",
      function_call_id: "t2",
      function_id: "shell::exec",
      args: { command: "echo", args: ["go"] },
      expires_at: 0,
    });
    expect(s.pendingApprovals).toHaveLength(2);

    s = applyEvent(s, {
      type: "approval_resolved",
      function_call_id: "t1",
      decision: "allow",
    });
    expect(s.pendingApprovals).toHaveLength(1);
    expect(s.pendingApprovals[0].function_call_id).toBe("t2");

    s = applyEvent(s, {
      type: "approval_resolved",
      function_call_id: "t2",
      decision: "allow",
    });
    expect(s.pendingApprovals).toHaveLength(0);
  });

  it("messages without entry_id land in unkeyedMessages (backwards compat)", () => {
    const e = { type: "message_end", message: msg("legacy") } as AgentEvent;
    const s = applyEvent(INITIAL_STREAM_STATE, e);
    expect(s.unkeyedMessages.length).toBe(1);
    expect(s.messageMap.size).toBe(0);
  });

  it(
    "function results race-emitted with different timestamps dedupe by function_call_id",
    () => {
      // Repro: two turn::step handlers race past handle_finalize's
      // turn_end_emitted guard and each emit a MessageEnd for the same
      // pending_approval prefilled result. The timestamps differ
      // (chrono::Utc::now is called inside each handler). Without keying
      // unkeyedMessages on function_call_id, both events land in the
      // visible list and the user sees the same pending block twice.
      const pendingResult = (timestamp: number): AgentMessage => ({
        role: "function_result",
        content: [
          {
            type: "text",
            text: '{\n  "status": "pending_approval",\n  "call_id": "toolu_01A5w9bqCaPSrLL44HUMgy1G",\n  "function_id": "engine::functions::list",\n  "message": "Awaiting human approval. The result will be reported in a future turn."\n}',
          },
        ],
        function_call_id: "toolu_01A5w9bqCaPSrLL44HUMgy1G",
        function_id: "engine::functions::list",
        is_error: false,
        timestamp,
      });
      let s = applyEvent(INITIAL_STREAM_STATE, {
        type: "message_end",
        message: pendingResult(1_700_000_000_001),
      } as AgentEvent);
      s = applyEvent(s, {
        type: "message_end",
        message: pendingResult(1_700_000_000_042),
      } as AgentEvent);
      expect(s.unkeyedMessages.length).toBe(1);
    },
  );
});

// ──────────────────────────────────────────────────────────────────
// Finding #2 follow-up: approval_wake_failed events must land in
// `state.wakeFailures` so the UI can surface them. Without this the
// reducer's default branch drops the event and the operator sees
// "approval went through" while the conversation has actually stalled.
// ──────────────────────────────────────────────────────────────────

describe("reducer — approval_wake_failed", () => {
  it("records a wake-failure with its error text and a timestamp", () => {
    const before = Date.now();
    const s = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_wake_failed",
      error: "run::resume timed out",
    });
    expect(s.wakeFailures).toHaveLength(1);
    expect(s.wakeFailures[0].error).toBe("run::resume timed out");
    expect(s.wakeFailures[0].ts).toBeGreaterThanOrEqual(before);
  });

  it("dedups identical error strings so a retry storm doesn't flood the UI", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_wake_failed",
      error: "ws closed",
    });
    s = applyEvent(s, { type: "approval_wake_failed", error: "ws closed" });
    s = applyEvent(s, { type: "approval_wake_failed", error: "ws closed" });
    expect(s.wakeFailures).toHaveLength(1);
  });

  it("keeps distinct error strings separate", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_wake_failed",
      error: "ws closed",
    });
    s = applyEvent(s, {
      type: "approval_wake_failed",
      error: "engine restarted",
    });
    expect(s.wakeFailures.map((w) => w.error)).toEqual([
      "ws closed",
      "engine restarted",
    ]);
  });

  it("falls back to a default error string when the event omits one", () => {
    const s = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_wake_failed",
      error: "",
    });
    expect(s.wakeFailures).toHaveLength(1);
    expect(s.wakeFailures[0].error).toMatch(/run::resume failed/i);
  });

  it("preserves wakeFailures when an unrelated event (approval_resolved) arrives", () => {
    let s = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_wake_failed",
      error: "ws closed",
    });
    s = applyEvent(s, {
      type: "approval_resolved",
      function_call_id: "tc-1",
      decision: "allow",
    });
    expect(s.wakeFailures).toHaveLength(1);
  });
});
