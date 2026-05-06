import { useState } from "react";
import { bridge, BridgeError } from "../bridge";
import type { PendingApproval } from "../types";

interface Props {
  sessionId: string;
  pending: PendingApproval[];
}

export function ApprovalRow({ sessionId, pending }: Props) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  if (pending.length === 0) return null;

  const resolve = async (toolCallId: string, decision: "allow" | "deny") => {
    setBusyId(toolCallId);
    setErr(null);
    try {
      await bridge<{ ok: boolean }>("approval::resolve", {
        session_id: sessionId,
        tool_call_id: toolCallId,
        decision,
      });
    } catch (e) {
      setErr(e instanceof BridgeError ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div className="approvals">
      {pending.map((a) => (
        <div className="approval" key={a.tool_call_id}>
          <div className="approval-head">
            <span className="approval-eyebrow">approval needed</span>
            <span className="approval-title">{a.tool_name}</span>
          </div>
          <pre className="approval-args">{JSON.stringify(a.args, null, 2)}</pre>
          <div className="approval-actions">
            <button
              type="button"
              className="approval-deny"
              disabled={busyId === a.tool_call_id}
              onClick={() => resolve(a.tool_call_id, "deny")}
            >
              deny
            </button>
            <button
              type="button"
              className="approval-allow"
              disabled={busyId === a.tool_call_id}
              onClick={() => resolve(a.tool_call_id, "allow")}
            >
              allow
            </button>
          </div>
        </div>
      ))}
      {err ? (
        <p className="approval-error" role="alert">
          {err}
        </p>
      ) : null}
    </div>
  );
}
