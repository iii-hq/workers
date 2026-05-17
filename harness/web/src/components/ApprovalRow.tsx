import { useEffect, useState } from "react";
import { bridge, BridgeError } from "../bridge";
import type { PendingApproval, WakeFailure } from "../types";

interface Props {
  sessionId: string;
  pending: PendingApproval[];
  /**
   * Wake-failure alerts surfaced by approval-gate when `run::resume`
   * fails after an approval was resolved. Finding #2 follow-up — without
   * showing these, the operator sees the card disappear and assumes
   * everything went through while the conversation has actually stalled.
   */
  wakeFailures?: WakeFailure[];
  /** Optional dismissal callback so the consumer can clear alerts. */
  onDismissWakeFailure?: (failure: WakeFailure) => void;
}

type ResolveDecision = "allow" | "deny";
type DenialPayload =
  | { kind: "user_rejected" }
  | { kind: "user_corrected"; detail: { feedback: string } };

/**
 * Subscribe to a 1s tick and report `expiresAt - now` (ms). Returns the
 * raw remaining number so the parent can render its own format. Negative
 * once expired; the parent disables actions on `remaining <= 0`.
 */
function useCountdown(expiresAt: number | undefined): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!expiresAt) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [expiresAt]);
  if (!expiresAt) return Number.POSITIVE_INFINITY;
  return expiresAt - now;
}

function formatRemaining(ms: number): string {
  if (!Number.isFinite(ms)) return "";
  if (ms <= 0) return "expired";
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${m}:${String(r).padStart(2, "0")}`;
}

/**
 * Map an `ok:false` reply from `approval::resolve` to an operator-readable
 * line. The gate's typed errors mean distinct things — generic copy here
 * would hide the multi-window race that `already_resolved` exists to signal.
 */
function messageForResolveError(error: string | undefined, decision: ResolveDecision): string {
  switch (error) {
    case "timed_out":
      return "this approval expired before you could resolve it";
    case "already_resolved":
      return "another window already resolved this approval";
    case "in_flight":
      return "this approval is mid-execution and can no longer be changed";
    case "not_found":
      return "approval record not found (already drained?)";
    case "missing_id":
    case "bad_decision":
    case "bad_denial":
      return `bad ${decision} payload: ${error}`;
    case "state_write_failed":
      return "state bus rejected the write — try again";
    default:
      return error ? `resolve failed: ${error}` : "resolve failed";
  }
}

function countdownTone(ms: number): "ok" | "warn" | "danger" | "expired" {
  if (!Number.isFinite(ms)) return "ok";
  if (ms <= 0) return "expired";
  if (ms < 10_000) return "danger";
  if (ms < 30_000) return "warn";
  return "ok";
}

interface ApprovalCardProps {
  sessionId: string;
  approval: PendingApproval;
  callId: string;
  fnId: string;
  busyId: string | null;
  onResolve: (functionCallId: string, decision: ResolveDecision, opts?: { always?: boolean; denial?: DenialPayload }) => void;
}

/**
 * One pending-approval card. Lives in its own component so the
 * countdown hook is rendered once per row (hooks can't sit inside
 * a .map callback without violating the rules-of-hooks contract).
 */
function ApprovalCard({ sessionId: _sessionId, approval, callId, fnId, busyId, onResolve }: ApprovalCardProps) {
  const remaining = useCountdown(approval.expires_at);
  const expired = remaining <= 0;
  const tone = countdownTone(remaining);
  const [feedback, setFeedback] = useState("");
  const titleId = `approval-title-${callId}`;
  const busy = busyId === callId;

  const denyClick = () => {
    const trimmed = feedback.trim();
    if (trimmed.length > 0) {
      onResolve(callId, "deny", {
        denial: { kind: "user_corrected", detail: { feedback: trimmed } },
      });
    } else {
      onResolve(callId, "deny");
    }
  };

  return (
    <section
      className="approval"
      data-expired={expired || undefined}
      data-busy={busy || undefined}
      role="region"
      aria-labelledby={titleId}
    >
      <header className="approval-head">
        <span className="approval-eyebrow">approval needed</span>
        <span className="approval-title" id={titleId}>{fnId}</span>
        {approval.expires_at ? (
          <span className="approval-countdown" data-tone={tone} aria-live="off">
            {formatRemaining(remaining)}
          </span>
        ) : null}
      </header>
      <pre className="approval-args">{JSON.stringify(approval.args, null, 2)}</pre>
      <details className="approval-feedback">
        <summary>
          <span className="approval-feedback-label">add a correction</span>
          <span className="approval-feedback-hint">sent to the model on deny</span>
        </summary>
        <textarea
          className="approval-feedback-input"
          value={feedback}
          onChange={(e) => setFeedback(e.target.value)}
          placeholder="e.g. wrong directory, use /tmp/y"
          rows={3}
        />
      </details>
      <div className="approval-actions">
        <button
          type="button"
          className="approval-deny"
          disabled={busy || expired}
          onClick={denyClick}
        >
          deny
        </button>
        <button
          type="button"
          className="approval-allow-always"
          disabled={busy || expired}
          title="Allow this and auto-approve other pending calls in this session that match the same pattern"
          onClick={() => onResolve(callId, "allow", { always: true })}
        >
          allow + always
        </button>
        <button
          type="button"
          className="approval-allow"
          disabled={busy || expired}
          onClick={() => onResolve(callId, "allow")}
        >
          allow
        </button>
      </div>
      {expired ? <p className="approval-expired-note">this approval has expired</p> : null}
    </section>
  );
}

export function ApprovalRow({
  sessionId,
  pending,
  wakeFailures = [],
  onDismissWakeFailure,
}: Props) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  // Even when no approvals are pending we must keep this region mounted
  // long enough to surface wake-failure alerts — those arrive AFTER
  // approval_resolved has removed every card from `pending`.
  if (pending.length === 0 && wakeFailures.length === 0) return null;

  const resolve = async (
    functionCallId: string,
    decision: ResolveDecision,
    opts: { always?: boolean; denial?: DenialPayload } = {}
  ) => {
    setBusyId(functionCallId);
    setErr(null);
    const payload: Record<string, unknown> = {
      session_id: sessionId,
      function_call_id: functionCallId,
      tool_call_id: functionCallId,  // legacy alias
      decision,
    };
    if (opts.always) payload.always = true;
    if (opts.denial) payload.denial = opts.denial;
    try {
      const resp = await bridge<{ ok: boolean; cascaded?: number; error?: string }>(
        "approval::resolve",
        payload,
      );
      // Finding #8: the bridge call succeeds at transport level even
      // when the gate returned ok:false (timed_out / already_resolved /
      // in_flight / not_found). Without surfacing that the operator
      // sees the card vanish and assumes the decision went through.
      if (!resp?.ok) {
        setErr(messageForResolveError(resp?.error, decision));
      }
    } catch (e) {
      setErr(e instanceof BridgeError ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div className="approvals" role="region" aria-label="pending approvals" aria-live="polite">
      {wakeFailures.map((w) => (
        <div
          key={`${w.ts}-${w.error}`}
          className="approval-wake-failed"
          role="alert"
        >
          <strong>Approval went through, but the conversation didn’t resume.</strong>
          <span className="approval-wake-failed-detail"> {w.error}</span>
          {onDismissWakeFailure ? (
            <button
              type="button"
              className="approval-wake-failed-dismiss"
              onClick={() => onDismissWakeFailure(w)}
              aria-label="dismiss wake-failure alert"
            >
              ×
            </button>
          ) : null}
        </div>
      ))}
      {pending.map((a) => {
        const callId = a.function_call_id ?? a.tool_call_id;
        const fnId = a.function_id ?? a.tool_name ?? "";
        if (!callId) return null;
        return (
          <ApprovalCard
            key={callId}
            sessionId={sessionId}
            approval={a}
            callId={callId}
            fnId={fnId}
            busyId={busyId}
            onResolve={resolve}
          />
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
