import { FormEvent, forwardRef, useImperativeHandle, useState } from "react";
import { bridge, BridgeError } from "../bridge";
import type { AuthStatus } from "../types";

export interface AuthPanelHandle {
  open: () => void;
}

interface Props {
  provider: string;
  status: AuthStatus | null;
  onStored: () => void;
}

const PLACEHOLDER: Record<string, string> = {
  anthropic: "sk-ant-...",
  openai: "sk-proj-...",
};

export const AuthPanel = forwardRef<AuthPanelHandle, Props>(function AuthPanel(
  { provider, status, onStored },
  ref,
) {
  const configured = status?.configured === true;
  const [open, setOpen] = useState(!configured);
  const [key, setKey] = useState("");
  const [feedback, setFeedback] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useImperativeHandle(ref, () => ({ open: () => setOpen(true) }), []);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const trimmed = key.trim();
    if (!trimmed) return;
    setBusy(true);
    setFeedback(null);
    try {
      await bridge<{ ok: boolean }>("auth::set_token", {
        provider,
        credential: { type: "api_key", key: trimmed },
      });
      setFeedback("stored.");
      setKey("");
      onStored();
      setTimeout(() => setOpen(false), 800);
    } catch (e) {
      const msg = e instanceof BridgeError ? e.message : String(e);
      setFeedback(`error · ${msg}`);
    } finally {
      setBusy(false);
    }
  };

  // When the panel is collapsed and a key is already configured, render
  // nothing — the ControlsBar pill is the single source of truth at that
  // point. The panel only surfaces when the user clicks "set key" or when
  // we're missing one.
  if (!open && configured) return null;

  return (
    <details
      className="auth"
      open={open}
      onToggle={(e) => setOpen(e.currentTarget.open)}
    >
      <summary className="auth-summary">
        <span className="auth-summary-label">credential · {provider}</span>
        <span className="auth-summary-state">
          {configured
            ? `stored · ${status?.source ?? "unknown"}`
            : "no key set"}
        </span>
      </summary>
      <form className="auth-form" onSubmit={submit}>
        <label className="auth-label" htmlFor={`key-${provider}`}>
          {provider} api key
        </label>
        <input
          id={`key-${provider}`}
          className="auth-input"
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder={PLACEHOLDER[provider] ?? "key"}
          value={key}
          onChange={(e) => setKey(e.target.value)}
        />
        <p className="auth-hint">
          stored on the bus via <code>auth::set_token {`{provider:"${provider}"}`}</code>.
          Lives only in this engine&rsquo;s state; never persisted to disk by
          the harness. The harness consults env vars
          (<code>{provider === "anthropic" ? "ANTHROPIC_API_KEY" : "OPENAI_API_KEY"}</code>)
          as a fallback.
        </p>
        <button className="btn-primary" disabled={busy || !key.trim()}>
          {busy ? "storing…" : `store ${provider} credential`}
        </button>
        {feedback ? <p className="auth-status">{feedback}</p> : null}
      </form>
    </details>
  );
});
