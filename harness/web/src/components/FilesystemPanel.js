import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { useCallback, useEffect, useMemo, useState } from "react";
import { bridge, BridgeError } from "../bridge";
function joinPath(base, name) {
    if (base === "/" || base === "")
        return `/${name}`.replace(/^\/+/, "/");
    return `${base.replace(/\/$/, "")}/${name}`;
}
function parentPath(p) {
    if (!p || p === "/")
        return "/";
    const trimmed = p.replace(/\/$/, "");
    const idx = trimmed.lastIndexOf("/");
    if (idx <= 0)
        return "/";
    return trimmed.slice(0, idx);
}
function fmtSize(n) {
    if (n == null)
        return "—";
    if (n < 1024)
        return `${n} B`;
    if (n < 1024 * 1024)
        return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
function entryKind(e) {
    if (e.kind === "dir")
        return "dir";
    if (typeof e.mode === "string" && e.mode.startsWith("d"))
        return "dir";
    return "file";
}
export function FilesystemPanel() {
    const [path, setPath] = useState("/");
    const [pathDraft, setPathDraft] = useState("/");
    const [entries, setEntries] = useState(null);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState(null);
    const [previewPath, setPreviewPath] = useState(null);
    const [previewText, setPreviewText] = useState("");
    const [previewMeta, setPreviewMeta] = useState(null);
    const [previewLoading, setPreviewLoading] = useState(false);
    const [previewError, setPreviewError] = useState(null);
    const listDir = useCallback(async (target) => {
        setLoading(true);
        setError(null);
        setEntries(null);
        try {
            const result = await bridge("shell::filesystem::ls", { path: target });
            if (result.details?.error) {
                throw new BridgeError(result.details.error, "shell::filesystem::ls", 0);
            }
            setEntries(result.details?.entries ?? []);
            setPath(target);
            setPathDraft(target);
        }
        catch (e) {
            const msg = e instanceof BridgeError ? e.message : String(e);
            setError(msg);
        }
        finally {
            setLoading(false);
        }
    }, []);
    const readFile = useCallback(async (target) => {
        setPreviewLoading(true);
        setPreviewError(null);
        setPreviewPath(target);
        setPreviewText("");
        setPreviewMeta(null);
        try {
            const result = await bridge("shell::filesystem::read", { path: target });
            if (result.details?.error) {
                throw new BridgeError(result.details.error, "shell::filesystem::read", 0);
            }
            const text = result.content?.[0]?.text ?? "";
            setPreviewText(text);
            setPreviewMeta(result.details ?? null);
        }
        catch (e) {
            const msg = e instanceof BridgeError ? e.message : String(e);
            setPreviewError(msg);
        }
        finally {
            setPreviewLoading(false);
        }
    }, []);
    useEffect(() => {
        void listDir("/");
    }, [listDir]);
    const sortedEntries = useMemo(() => {
        if (!entries)
            return [];
        return [...entries].sort((a, b) => {
            const ka = entryKind(a);
            const kb = entryKind(b);
            if (ka !== kb)
                return ka === "dir" ? -1 : 1;
            return a.name.localeCompare(b.name);
        });
    }, [entries]);
    return (_jsxs("section", { className: "fs-panel", children: [_jsxs("header", { className: "panel-head", children: [_jsx("span", { className: "panel-title", children: "filesystem" }), _jsx("span", { className: "panel-sub", children: "sandbox via shell::filesystem::*" })] }), _jsxs("form", { className: "fs-pathbar", onSubmit: (e) => {
                    e.preventDefault();
                    void listDir(pathDraft.trim() || "/");
                }, children: [_jsx("button", { type: "button", className: "btn-ghost", onClick: () => void listDir(parentPath(path)), disabled: loading || path === "/", "aria-label": "parent directory", children: "\u2191" }), _jsx("input", { className: "fs-pathinput", value: pathDraft, onChange: (e) => setPathDraft(e.target.value), spellCheck: false }), _jsx("button", { type: "submit", className: "btn-primary", disabled: loading, children: loading ? "loading…" : "open" })] }), error ? (_jsx("p", { className: "panel-error", role: "alert", children: error })) : null, _jsxs("div", { className: "fs-body", children: [_jsxs("ul", { className: "fs-list", children: [sortedEntries.length === 0 && !loading && !error ? (_jsx("li", { className: "fs-empty", children: "empty directory" })) : null, sortedEntries.map((e) => {
                                const kind = entryKind(e);
                                const full = joinPath(path, e.name);
                                const isPreview = previewPath === full;
                                return (_jsx("li", { children: _jsxs("button", { type: "button", className: "fs-row", "data-kind": kind, "data-active": isPreview, onClick: () => {
                                            if (kind === "dir")
                                                void listDir(full);
                                            else
                                                void readFile(full);
                                        }, children: [_jsx("span", { className: "fs-icon", "aria-hidden": true, children: kind === "dir" ? "▸" : "·" }), _jsx("span", { className: "fs-name", children: e.name }), _jsx("span", { className: "fs-meta", children: kind === "file" ? fmtSize(e.size) : "dir" })] }) }, full));
                            })] }), _jsx("div", { className: "fs-preview", children: previewPath == null ? (_jsx("p", { className: "fs-preview-empty", children: "click a file to preview \u00B7 max 256 KB inline" })) : (_jsxs(_Fragment, { children: [_jsxs("header", { className: "fs-preview-head", children: [_jsx("span", { className: "fs-preview-path", children: previewPath }), previewMeta?.truncated ? (_jsx("span", { className: "budget-tag", "data-tone": "warn", children: "truncated" })) : null, previewMeta?.bytes_read != null ? (_jsx("span", { className: "fs-preview-size", children: fmtSize(previewMeta.bytes_read) })) : null] }), previewLoading ? (_jsx("p", { className: "fs-preview-loading", children: "loading\u2026" })) : previewError ? (_jsx("p", { className: "panel-error", role: "alert", children: previewError })) : (_jsx("pre", { className: "fs-preview-body", children: previewText }))] })) })] })] }));
}
