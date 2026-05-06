import { jsxs as _jsxs, jsx as _jsx } from "react/jsx-runtime";
import { forwardRef, useImperativeHandle, useState } from "react";
import { bridge, BridgeError } from "../bridge";
const PLACEHOLDER = {
    anthropic: "sk-ant-...",
    openai: "sk-proj-...",
};
export const AuthPanel = forwardRef(function AuthPanel({ provider, status, onStored }, ref) {
    const configured = status?.configured === true;
    const [open, setOpen] = useState(!configured);
    const [key, setKey] = useState("");
    const [feedback, setFeedback] = useState(null);
    const [busy, setBusy] = useState(false);
    useImperativeHandle(ref, () => ({ open: () => setOpen(true) }), []);
    const submit = async (e) => {
        e.preventDefault();
        const trimmed = key.trim();
        if (!trimmed)
            return;
        setBusy(true);
        setFeedback(null);
        try {
            await bridge("auth::set_token", {
                provider,
                credential: { type: "api_key", key: trimmed },
            });
            setFeedback("stored.");
            setKey("");
            onStored();
            setTimeout(() => setOpen(false), 800);
        }
        catch (e) {
            const msg = e instanceof BridgeError ? e.message : String(e);
            setFeedback(`error · ${msg}`);
        }
        finally {
            setBusy(false);
        }
    };
    // When the panel is collapsed and a key is already configured, render
    // nothing — the ControlsBar pill is the single source of truth at that
    // point. The panel only surfaces when the user clicks "set key" or when
    // we're missing one.
    if (!open && configured)
        return null;
    return (_jsxs("details", { className: "auth", open: open, onToggle: (e) => setOpen(e.currentTarget.open), children: [_jsxs("summary", { className: "auth-summary", children: [_jsxs("span", { className: "auth-summary-label", children: ["credential \u00B7 ", provider] }), _jsx("span", { className: "auth-summary-state", children: configured
                            ? `stored · ${status?.source ?? "unknown"}`
                            : "no key set" })] }), _jsxs("form", { className: "auth-form", onSubmit: submit, children: [_jsxs("label", { className: "auth-label", htmlFor: `key-${provider}`, children: [provider, " api key"] }), _jsx("input", { id: `key-${provider}`, className: "auth-input", type: "password", autoComplete: "off", spellCheck: false, placeholder: PLACEHOLDER[provider] ?? "key", value: key, onChange: (e) => setKey(e.target.value) }), _jsxs("p", { className: "auth-hint", children: ["stored on the bus via ", _jsxs("code", { children: ["auth::set_token ", `{provider:"${provider}"}`] }), ". Lives only in this engine\u2019s state; never persisted to disk by the harness. The harness consults env vars (", _jsx("code", { children: provider === "anthropic" ? "ANTHROPIC_API_KEY" : "OPENAI_API_KEY" }), ") as a fallback."] }), _jsx("button", { className: "btn-primary", disabled: busy || !key.trim(), children: busy ? "storing…" : `store ${provider} credential` }), feedback ? _jsx("p", { className: "auth-status", children: feedback }) : null] })] }));
});
