import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { bridge, BridgeError } from "./bridge";
import { useSkillsIndex } from "./useSkillsIndex";
import { AuthPanel } from "./components/AuthPanel";
import { Composer } from "./components/Composer";
import { ContextMeter } from "./components/ContextMeter";
import { ControlsBar } from "./components/ControlsBar";
import { CostPanel } from "./components/CostPanel";
import { FilesystemPanel } from "./components/FilesystemPanel";
import { SessionList, fetchSessions } from "./components/SessionList";
import { SessionView } from "./components/SessionView";
import { StatusPill } from "./components/StatusPill";
// Tool schemas advertised to the LLM with each turn. Names map 1:1 to bus
// function ids — turn-orchestrator dispatches `tool_call.name` directly via
// `iii.trigger`. Add or trim entries here to widen/narrow the model's surface.
//
// Permission lives one layer down: `policy-denylist` subscribes to
// `agent::before_tool_call` and refuses by name. Set its env var when starting
// the worker, e.g.:
//   POLICY_DENIED_TOOLS=shell::filesystem::rm,shell::filesystem::sed,shell::filesystem::edit,shell::filesystem::chmod,shell::filesystem::mv
const TOOLS = [
    {
        name: "shell::filesystem::ls",
        description: "List directory entries inside the sandbox.",
        input_schema: {
            type: "object",
            properties: {
                path: {
                    type: "string",
                    description: "Absolute path to a directory inside the sandbox.",
                },
            },
            required: ["path"],
        },
    },
    {
        name: "shell::filesystem::read",
        description: "Read a file inside the sandbox. Returns UTF-8 contents (max 256 KB inline).",
        input_schema: {
            type: "object",
            properties: {
                path: {
                    type: "string",
                    description: "Absolute path to a file inside the sandbox.",
                },
            },
            required: ["path"],
        },
    },
    {
        name: "shell::filesystem::write",
        description: "Write content to a file inside the sandbox. Creates parent dirs as needed; overwrites any existing file at the path.",
        input_schema: {
            type: "object",
            properties: {
                path: {
                    type: "string",
                    description: "Absolute path inside the sandbox.",
                },
                content: {
                    type: "string",
                    description: "UTF-8 file contents.",
                },
            },
            required: ["path", "content"],
        },
    },
    {
        name: "shell::filesystem::mkdir",
        description: "Create a directory (and parents) inside the sandbox.",
        input_schema: {
            type: "object",
            properties: { path: { type: "string" } },
            required: ["path"],
        },
    },
    {
        name: "shell::filesystem::stat",
        description: "Return metadata (size, mode, mtime) for a sandbox path.",
        input_schema: {
            type: "object",
            properties: { path: { type: "string" } },
            required: ["path"],
        },
    },
    {
        name: "skill::fetch",
        description: "Read one or more iii:// skill URIs as markdown. Use to drill into specific worker docs after seeing them in the iii://skills index.",
        input_schema: {
            type: "object",
            properties: {
                uri: {
                    type: "string",
                    description: "Single iii:// URI to read (e.g. iii://auth-credentials/get_token).",
                },
                uris: {
                    type: "array",
                    items: { type: "string" },
                    description: "Multiple iii:// URIs to read and concatenate. Wins when both `uri` and `uris` are provided.",
                },
            },
        },
    },
];
const BASE_SYSTEM_PROMPT = [
    "You have filesystem tools that operate inside a sandbox:",
    "  - shell::filesystem::ls   → list a directory",
    "  - shell::filesystem::read → read a file",
    "  - shell::filesystem::write → create or overwrite a file",
    "  - shell::filesystem::mkdir → make a directory",
    "  - shell::filesystem::stat → file metadata",
    "",
    "Use them whenever the user asks to read, inspect, create, or modify files. Paths must be absolute (e.g. /tmp/notes.md). Some destructive ops (rm, mv, chmod, sed, edit) may be denied by policy — if a tool comes back with `blocked` in the result, explain to the user which policy refused and stop, do not retry.",
].join("\n");
function buildSystemPrompt(skillsIndex) {
    const skillsSection = skillsIndex
        ? `## Available skills

${skillsIndex}

Use the \`skill::fetch\` tool to load any \`iii://\` URI you see above when you need its full content.`
        : "## Available skills\n\n(Skills index not loaded — call `skill::fetch` with `uri: \"iii://skills\"` to discover what's registered.)";
    return `${BASE_SYSTEM_PROMPT}\n\n${skillsSection}`;
}
// Providers we have actual workers for in iii.worker.yaml. Don't add others
// here — they'd appear in the UI but every send would error with "function
// not found: provider::<name>::complete".
const SUPPORTED_PROVIDERS = ["anthropic", "openai"];
const DEFAULT_MODEL_BY_PROVIDER = {
    anthropic: "claude-opus-4-7",
    openai: "gpt-5",
};
function newSessionId() {
    const d = new Date();
    const pad = (n) => String(n).padStart(2, "0");
    return `s${d.getMonth() + 1}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
}
export default function App() {
    const [sessions, setSessions] = useState([]);
    const [active, setActive] = useState(null);
    const [draftId, setDraftId] = useState(null);
    const [messages, setMessages] = useState([]);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState(null);
    // Skills-index fetch is strictly non-blocking. The fallback branch in
    // buildSystemPrompt(null) is the agent's recovery path; do not gate
    // rendering or send() on the index being loaded.
    const { index: skillsIndex } = useSkillsIndex();
    const [provider, setProvider] = useState("anthropic");
    const [model, setModel] = useState(DEFAULT_MODEL_BY_PROVIDER.anthropic);
    const [models, setModels] = useState([]);
    const [authByProvider, setAuthByProvider] = useState({});
    const authPanelRef = useRef(null);
    const [tab, setTab] = useState("chat");
    const refreshSessions = useCallback(async () => {
        try {
            setSessions(await fetchSessions());
        }
        catch {
            // bridge unreachable; StatusPill surfaces it
        }
    }, []);
    const refreshAuth = useCallback(async (p) => {
        try {
            const s = await bridge("auth::status", { provider: p });
            setAuthByProvider((prev) => ({ ...prev, [p]: s }));
        }
        catch {
            setAuthByProvider((prev) => ({
                ...prev,
                [p]: { configured: false, source: null, label: null },
            }));
        }
    }, []);
    const loadMessages = useCallback(async (id) => {
        try {
            const msgs = await bridge("state::get", {
                scope: "agent",
                key: `session/${id}/messages`,
            });
            setMessages(Array.isArray(msgs) ? msgs : []);
        }
        catch (e) {
            // brand new session — state::get returns null; treat as empty
            if (e instanceof BridgeError && /session not found|null/.test(e.message)) {
                setMessages([]);
                return;
            }
            setMessages([]);
        }
    }, []);
    // Pull sessions on a slow tick so new turns surface in the rail.
    useEffect(() => {
        void refreshSessions();
        const id = setInterval(refreshSessions, 4000);
        return () => clearInterval(id);
    }, [refreshSessions]);
    // Load model catalog once.
    useEffect(() => {
        bridge("models::list")
            .then((r) => setModels(r.models ?? []))
            .catch(() => setModels([]));
    }, []);
    // Resolve auth status for both providers up front so the controls bar
    // tells the truth without round-trips when the user clicks between them.
    useEffect(() => {
        SUPPORTED_PROVIDERS.forEach((p) => void refreshAuth(p));
    }, [refreshAuth]);
    useEffect(() => {
        if (active)
            void loadMessages(active);
        else
            setMessages([]);
    }, [active, loadMessages]);
    // When the catalog or provider changes, ensure the selected model belongs
    // to the active provider; otherwise pick the configured default or first
    // available model for the provider.
    useEffect(() => {
        const pool = models.filter((m) => m.provider === provider);
        if (pool.length === 0)
            return;
        if (pool.some((m) => m.id === model))
            return;
        const preferred = DEFAULT_MODEL_BY_PROVIDER[provider];
        const next = pool.find((m) => m.id === preferred) ?? pool[0];
        setModel(next.id);
    }, [models, provider, model]);
    const startNew = () => {
        setActive(null);
        setMessages([]);
        setDraftId(newSessionId());
        setError(null);
    };
    const send = async (prompt) => {
        setError(null);
        const sid = active ?? draftId ?? newSessionId();
        setActive(sid);
        setDraftId(null);
        setLoading(true);
        const optimistic = {
            role: "user",
            content: [{ type: "text", text: prompt }],
            timestamp: Date.now(),
        };
        // turn-orchestrator's `run::start` overwrites the persisted transcript
        // with whatever `messages` arrives in the payload (run_start.rs:28).
        // To keep multi-turn memory we send the full prior transcript plus the
        // new user message every time. The bus will replay it back to the LLM.
        const fullHistory = [...messages, optimistic];
        setMessages(fullHistory);
        try {
            const result = await bridge("run::start_and_wait", {
                session_id: sid,
                provider,
                model,
                messages: fullHistory,
                system_prompt: buildSystemPrompt(skillsIndex),
                tools: [],
                // Tool-calling turns roundtrip the model 2+ times plus filesystem
                // ops; allow generous headroom. The engine's HTTP trigger and the
                // bridge::trigger inner call must each be ≥ this for the turn to
                // surface a result instead of a 504.
                timeout_ms: 240000,
            });
            setMessages(result.messages ?? fullHistory);
            void refreshSessions();
        }
        catch (e) {
            const msg = e instanceof BridgeError ? e.message : String(e);
            setError(msg);
        }
        finally {
            setLoading(false);
        }
    };
    const sessionId = active ?? draftId ?? "";
    const currentAuth = authByProvider[provider] ?? null;
    const composerDisabled = !(currentAuth?.configured ?? false);
    const contextWindow = useMemo(() => {
        const m = models.find((x) => x.id === model);
        return m?.context_window ?? null;
    }, [models, model]);
    return (_jsxs("div", { className: "app", children: [_jsxs("header", { className: "app-head", children: [_jsxs("div", { className: "app-mark", children: [_jsx("span", { className: "app-mark-glyph", children: "\u2318" }), _jsx("span", { className: "app-mark-name", children: "harness" }), _jsx("span", { className: "app-mark-sub", children: "bus console" })] }), _jsx(StatusPill, {})] }), _jsxs("div", { className: "app-body", children: [_jsx(SessionList, { sessions: sessions, active: active, onPick: (id) => {
                            setActive(id);
                            setDraftId(null);
                            setError(null);
                        }, onNew: startNew }), _jsxs("main", { className: "main", children: [_jsx("nav", { className: "tabs", role: "tablist", "aria-label": "harness panels", children: ["chat", "cost", "files"].map((t) => (_jsx("button", { type: "button", role: "tab", className: "tab", "data-active": tab === t, "aria-selected": tab === t, onClick: () => setTab(t), children: t }, t))) }), tab === "chat" ? (_jsxs(_Fragment, { children: [_jsx(ControlsBar, { providers: SUPPORTED_PROVIDERS, provider: provider, onProvider: (p) => setProvider(p), models: models, model: model, onModel: setModel, authStatus: currentAuth, onManageAuth: () => authPanelRef.current?.open() }), _jsx(AuthPanel, { ref: authPanelRef, provider: provider, status: currentAuth, onStored: () => void refreshAuth(provider) }), _jsx(SessionView, { sessionId: sessionId, messages: messages, loading: loading }), error ? (_jsx("p", { className: "app-error", role: "alert", children: error })) : null, _jsx(ContextMeter, { messages: messages, contextWindow: contextWindow, model: model }), _jsx(Composer, { disabled: composerDisabled, onSend: send })] })) : null, tab === "cost" ? _jsx(CostPanel, {}) : null, tab === "files" ? _jsx(FilesystemPanel, {}) : null] })] }), _jsxs("footer", { className: "app-foot", children: [_jsxs("span", { children: ["provider \u00B7 ", provider] }), _jsxs("span", { children: ["model \u00B7 ", model] }), _jsx("span", { children: "endpoint \u00B7 POST /bridge/trigger" })] })] }));
}
