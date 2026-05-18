/**
 * `router::abort` side-effects builder. Mirrors
 * `provider-router/src/register.rs::abort_side_effects` (PR #150).
 *
 * Returns the ordered list of iii triggers an abort should fire: first
 * set the per-session `abort_signal` flag so any in-flight handlers can
 * notice the cancellation, then sweep pending approvals so the next
 * session start (or list_pending call) sees a clean slate.
 *
 * No registration is wired in harness-node yet — the live `router::abort`
 * handler lives in the Rust workspace. This module exposes the pure
 * helper so when the harness-node abort handler arrives it can call
 * `abortSideEffects(sid).forEach(fire)` for parity with Rust.
 */

const STATE_SCOPE = 'agent';

export type AbortSideEffect = {
  function_id: string;
  payload: Record<string, unknown>;
};

export function abortSideEffects(session_id: string): AbortSideEffect[] {
  return [
    {
      function_id: 'state::set',
      payload: {
        scope: STATE_SCOPE,
        key: `session/${session_id}/abort_signal`,
        value: true,
      },
    },
    {
      function_id: 'approval::sweep_session',
      payload: { session_id },
    },
  ];
}
