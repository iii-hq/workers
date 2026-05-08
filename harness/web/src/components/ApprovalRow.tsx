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

  const resolve = async (functionCallId: string, decision: "allow" | "deny") => {
    setBusyId(functionCallId);
    setErr(null);
    try {
      await bridge<{ ok: boolean }>("approval::resolve", {
        session_id: sessionId,
        function_call_id: functionCallId,
        tool_call_id: functionCallId,
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
      {pending.map((a) => {
        const callId = a.function_call_id ?? a.tool_call_id;
        const fnId = a.function_id ?? a.tool_name ?? "";
        if (!callId) return null;
        return (
        <div className="approval" key={callId}>
          <div className="approval-head">
            <span className="approval-eyebrow">approval needed</span>
            <span className="approval-title">{fnId}</span>
          </div>
          <pre className="approval-args">{JSON.stringify(a.args, null, 2)}</pre>
          <div className="approval-actions">
            <button
              type="button"
              className="approval-deny"
              disabled={busyId === callId}
              onClick={() => resolve(callId, "deny")}
            >
              deny
            </button>
            <button
              type="button"
              className="approval-allow"
              disabled={busyId === callId}
              onClick={() => resolve(callId, "allow")}
            >
              allow
            </button>
          </div>
        </div>
        );
      })}
      {err ? (
        <p className="approval-error" role="alert">
          {err}
        </p>
      ) : null}
    </div>
  );
}
