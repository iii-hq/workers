import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { bridge } from "../bridge";
export async function fetchSessions() {
    // state::list returns values without keys. The turn_state value is the
    // only one in scope=agent that carries both session_id and state, so we
    // filter for it.
    const values = await bridge("state::list", {
        scope: "agent",
        prefix: "session/",
    });
    if (!Array.isArray(values))
        return [];
    const rows = [];
    for (const v of values) {
        if (v &&
            typeof v === "object" &&
            "session_id" in v &&
            "state" in v &&
            typeof v.session_id === "string") {
            const o = v;
            rows.push({
                session_id: String(o.session_id),
                state: String(o.state ?? "unknown"),
                turn_count: Number(o.turn_count ?? 0),
                updated_at_ms: Number(o.updated_at_ms ?? 0),
            });
        }
    }
    rows.sort((a, b) => b.updated_at_ms - a.updated_at_ms);
    return rows;
}
export function SessionList({ sessions, active, onPick, onNew }) {
    return (_jsxs("aside", { className: "rail", children: [_jsxs("header", { className: "rail-head", children: [_jsx("span", { className: "rail-title", children: "sessions" }), _jsx("button", { className: "btn-ghost", onClick: onNew, type: "button", children: "+ new" })] }), sessions.length === 0 ? (_jsx("p", { className: "rail-empty", children: "no turns yet. send your first message on the right." })) : (_jsx("ul", { className: "session-list", children: sessions.map((s) => (_jsx("li", { children: _jsxs("button", { type: "button", className: "session-row", "data-active": s.session_id === active, onClick: () => onPick(s.session_id), children: [_jsx("span", { className: "session-id", children: s.session_id }), _jsxs("span", { className: "session-meta", children: [_jsx("span", { "data-state": s.state, children: s.state }), _jsx("span", { children: "\u00B7" }), _jsxs("span", { children: [s.turn_count, " turn", s.turn_count === 1 ? "" : "s"] })] })] }) }, s.session_id))) }))] }));
}
