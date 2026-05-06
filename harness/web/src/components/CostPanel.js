import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { useCallback, useEffect, useState } from "react";
import { bridge, BridgeError } from "../bridge";
function fmtUsd(n) {
    return new Intl.NumberFormat("en-US", {
        style: "currency",
        currency: "USD",
        maximumFractionDigits: 2,
    }).format(n);
}
function fmtPct(spent, ceiling) {
    if (ceiling <= 0)
        return "—";
    return `${((spent / ceiling) * 100).toFixed(1)}%`;
}
function fmtDate(ms) {
    if (!ms)
        return "—";
    return new Date(ms).toLocaleString();
}
async function loadDetail(b) {
    const out = {
        budget: b,
        usage: null,
        forecast: null,
        error: null,
    };
    try {
        out.usage = await bridge("budget::usage", {
            budget_id: b.id,
            window: "all",
        });
    }
    catch (e) {
        out.error = e instanceof BridgeError ? e.message : String(e);
    }
    try {
        out.forecast = await bridge("budget::forecast", {
            budget_id: b.id,
        });
    }
    catch (e) {
        if (!out.error) {
            out.error = e instanceof BridgeError ? e.message : String(e);
        }
    }
    return out;
}
export function CostPanel() {
    const [details, setDetails] = useState([]);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState(null);
    const refresh = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const resp = await bridge("budget::list", {});
            const list = Array.isArray(resp)
                ? resp
                : Array.isArray(resp?.budgets)
                    ? resp.budgets
                    : [];
            const enriched = await Promise.all(list.map(loadDetail));
            setDetails(enriched);
        }
        catch (e) {
            const msg = e instanceof BridgeError ? e.message : String(e);
            setError(msg);
            setDetails([]);
        }
        finally {
            setLoading(false);
        }
    }, []);
    useEffect(() => {
        void refresh();
    }, [refresh]);
    return (_jsxs("section", { className: "cost-panel", children: [_jsxs("header", { className: "panel-head", children: [_jsx("span", { className: "panel-title", children: "cost" }), _jsx("span", { className: "panel-sub", children: "workspace + agent budgets via budget::*" }), _jsx("button", { type: "button", className: "btn-ghost", onClick: () => void refresh(), disabled: loading, children: loading ? "refreshing…" : "refresh" })] }), error ? (_jsx("p", { className: "panel-error", role: "alert", children: error })) : null, !loading && details.length === 0 && !error ? (_jsxs("p", { className: "panel-empty", children: ["no budgets configured. create one via", " ", _jsx("code", { children: "budget::create" }), " on the bus."] })) : null, _jsx("ul", { className: "budget-list", children: details.map((d) => {
                    const b = d.budget;
                    const pct = fmtPct(b.spent_usd, b.ceiling_usd);
                    const overBudget = b.spent_usd >= b.ceiling_usd;
                    return (_jsxs("li", { className: "budget-row", children: [_jsxs("header", { className: "budget-row-head", children: [_jsx("span", { className: "budget-id", children: b.name ?? b.id }), _jsx("span", { className: "budget-period", children: b.period }), b.enforced ? (_jsx("span", { className: "budget-tag", "data-tone": "warn", children: "enforced" })) : null, b.paused ? (_jsx("span", { className: "budget-tag", "data-tone": "muted", children: "paused" })) : null] }), _jsxs("dl", { className: "budget-grid", children: [_jsxs("div", { children: [_jsx("dt", { children: "spent" }), _jsxs("dd", { "data-over": overBudget ? "true" : "false", children: [fmtUsd(b.spent_usd), " ", _jsxs("span", { children: ["/ ", fmtUsd(b.ceiling_usd)] })] })] }), _jsxs("div", { children: [_jsx("dt", { children: "utilization" }), _jsx("dd", { children: pct })] }), _jsxs("div", { children: [_jsx("dt", { children: "resets" }), _jsx("dd", { children: fmtDate(b.period_resets_at) })] }), d.forecast ? (_jsxs(_Fragment, { children: [_jsxs("div", { children: [_jsx("dt", { children: "burn rate" }), _jsxs("dd", { children: [fmtUsd(d.forecast.rate_usd_per_day), " /day"] })] }), _jsxs("div", { children: [_jsx("dt", { children: "projected (30d)" }), _jsx("dd", { children: fmtUsd(d.forecast.projected_month_usd) })] }), _jsxs("div", { children: [_jsx("dt", { children: "days until breach" }), _jsx("dd", { children: d.forecast.days_until_breach == null
                                                            ? "—"
                                                            : d.forecast.days_until_breach.toFixed(1) })] })] })) : null] }), b.alerts.length > 0 ? (_jsx("ul", { className: "budget-alerts", children: b.alerts.map((a) => (_jsxs("li", { children: ["alert \u00B7 fires at ", a.threshold_pct, "% \u2192", " ", _jsx("code", { children: a.callback_function_id })] }, a.alert_id))) })) : null, d.error ? (_jsxs("p", { className: "panel-error", children: ["usage/forecast \u00B7 ", d.error] })) : null] }, b.id));
                }) })] }));
}
