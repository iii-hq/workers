import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
const fmt = new Intl.NumberFormat("en-US");
function lastAssistant(messages) {
    for (let i = messages.length - 1; i >= 0; i--) {
        const m = messages[i];
        if (m.role === "assistant")
            return m;
    }
    return null;
}
export function ContextMeter({ messages, contextWindow, model }) {
    const last = lastAssistant(messages);
    const used = last
        ? (last.usage?.input ?? 0) + (last.usage?.output ?? 0)
        : 0;
    const cap = contextWindow ?? 0;
    const ratio = cap > 0 ? Math.min(1, used / cap) : 0;
    const pct = cap > 0 ? ratio * 100 : 0;
    // tier the meter color: calm -> warn -> danger
    const tier = ratio < 0.8 ? "calm" : ratio < 0.95 ? "warn" : "danger";
    return (_jsxs("div", { className: "ctx-meter", "data-tier": tier, children: [_jsxs("div", { className: "ctx-meter-row", children: [_jsx("span", { className: "ctx-meter-label", children: "context" }), _jsxs("span", { className: "ctx-meter-numbers", children: [_jsx("span", { className: "ctx-meter-used", children: fmt.format(used) }), _jsx("span", { className: "ctx-meter-divider", children: "/" }), _jsx("span", { className: "ctx-meter-cap", children: cap > 0 ? fmt.format(cap) : "—" }), _jsx("span", { className: "ctx-meter-unit", children: "tokens" }), cap > 0 ? (_jsxs("span", { className: "ctx-meter-pct", children: [pct < 0.1 && pct > 0 ? "<0.1" : pct.toFixed(pct < 10 ? 2 : 1), "%"] })) : null] }), _jsx("span", { className: "ctx-meter-model", title: model, children: model })] }), _jsx("div", { className: "ctx-meter-bar", "aria-hidden": true, children: _jsx("div", { className: "ctx-meter-fill", style: { width: `${cap > 0 ? Math.max(pct, 0.5) : 0}%` } }) })] }));
}
