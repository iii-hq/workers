import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
function blockText(m) {
    return m.content
        .filter((b) => b.type === "text")
        .map((b) => b.text)
        .join("\n");
}
function roleLabel(m) {
    if (m.role === "user")
        return "you";
    if (m.role === "assistant")
        return m.model ? `${m.model}` : "assistant";
    return m.tool_name ? `tool · ${m.tool_name}` : "tool";
}
export function SessionView({ sessionId, messages, loading }) {
    if (!sessionId) {
        return (_jsxs("section", { className: "view view-empty", children: [_jsx("h2", { className: "view-empty-h", children: "no session selected" }), _jsx("p", { className: "view-empty-p", children: "start a new turn or pick one from the rail." })] }));
    }
    return (_jsxs("section", { className: "view", children: [_jsxs("header", { className: "view-head", children: [_jsx("span", { className: "view-eyebrow", children: "session" }), _jsx("h2", { className: "view-title", children: sessionId })] }), _jsxs("ol", { className: "messages", children: [messages.map((m, i) => {
                        const text = blockText(m);
                        if (!text)
                            return null;
                        return (_jsxs("li", { className: "msg", "data-role": m.role, children: [_jsx("span", { className: "msg-role", children: roleLabel(m) }), _jsx("p", { className: "msg-text", children: text }), m.role === "assistant" && m.usage ? (_jsxs("p", { className: "msg-usage", children: [m.usage.input ?? 0, "\u2193 \u00B7 ", m.usage.output ?? 0, "\u2191 tokens", m.stop_reason ? ` · stop: ${m.stop_reason}` : null] })) : null] }, i));
                    }), loading ? (_jsxs("li", { className: "msg msg-loading", children: [_jsx("span", { className: "msg-role", children: "\u2026" }), _jsx("p", { className: "msg-text", children: "running turn\u2026" })] })) : null] })] }));
}
