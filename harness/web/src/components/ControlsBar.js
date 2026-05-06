import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
export function ControlsBar({ providers, provider, onProvider, models, model, onModel, authStatus, onManageAuth, }) {
    const providerModels = models.filter((m) => m.provider === provider);
    const tone = authStatus?.configured === true
        ? "ok"
        : authStatus === null
            ? "unknown"
            : "missing";
    return (_jsxs("div", { className: "controls", children: [_jsxs("div", { className: "controls-group", children: [_jsx("span", { className: "controls-label", children: "provider" }), _jsx("div", { className: "seg", role: "radiogroup", "aria-label": "provider", children: providers.map((p) => (_jsx("button", { role: "radio", "aria-checked": p === provider, type: "button", className: "seg-btn", "data-active": p === provider, onClick: () => onProvider(p), children: p }, p))) })] }), _jsxs("div", { className: "controls-group", children: [_jsx("span", { className: "controls-label", children: "model" }), _jsx("select", { className: "model-select", value: model, onChange: (e) => onModel(e.target.value), disabled: providerModels.length === 0, children: providerModels.length === 0 ? (_jsxs("option", { value: "", children: ["no models for ", provider] })) : (providerModels.map((m) => (_jsx("option", { value: m.id, children: m.id }, m.id)))) })] }), _jsxs("button", { type: "button", className: "cred-pill", "data-tone": tone, onClick: onManageAuth, title: authStatus?.label
                    ? `${authStatus.label} (${authStatus.source})`
                    : "no credential set", children: [_jsx("span", { className: "cred-dot", "aria-hidden": true }), _jsx("span", { children: tone === "ok"
                            ? `credential · ${authStatus.source}`
                            : tone === "unknown"
                                ? "credential · checking…"
                                : "credential · set key" })] })] }));
}
