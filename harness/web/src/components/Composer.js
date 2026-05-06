import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useState } from "react";
export function Composer({ disabled, onSend }) {
    const [text, setText] = useState("");
    const [busy, setBusy] = useState(false);
    const submit = async (e) => {
        e.preventDefault();
        const trimmed = text.trim();
        if (!trimmed || busy)
            return;
        setBusy(true);
        try {
            await onSend(trimmed);
            setText("");
        }
        finally {
            setBusy(false);
        }
    };
    return (_jsxs("form", { className: "composer", onSubmit: submit, children: [_jsx("textarea", { className: "composer-input", placeholder: disabled
                    ? "set an api key to send messages"
                    : "say something. shift+enter for newline.", value: text, onChange: (e) => setText(e.target.value), onKeyDown: (e) => {
                    if (e.key === "Enter" && !e.shiftKey) {
                        e.preventDefault();
                        void submit(e);
                    }
                }, disabled: disabled || busy, rows: 3 }), _jsx("button", { type: "submit", className: "composer-send", disabled: disabled || busy || !text.trim(), children: busy ? "sending…" : "send → run::start_and_wait" })] }));
}
