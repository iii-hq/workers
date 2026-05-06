import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useEffect, useState } from "react";
import { bridge } from "../bridge";
export function StatusPill() {
    const [status, setStatus] = useState("unknown");
    const [info, setInfo] = useState(null);
    useEffect(() => {
        let cancelled = false;
        const tick = async () => {
            try {
                const s = await bridge("harness::status");
                if (!cancelled) {
                    setStatus("ok");
                    setInfo(s);
                }
            }
            catch {
                if (!cancelled)
                    setStatus("down");
            }
        };
        tick();
        const id = setInterval(tick, 5000);
        return () => {
            cancelled = true;
            clearInterval(id);
        };
    }, []);
    return (_jsxs("div", { className: "status-pill", "data-status": status, children: [_jsx("span", { className: "status-dot", "aria-hidden": true }), _jsx("span", { className: "status-label", children: status === "ok" && info
                    ? `${info.name} ${info.version} · ${info.expected_workers.length} workers`
                    : status === "down"
                        ? "engine unreachable"
                        : "checking…" })] }));
}
