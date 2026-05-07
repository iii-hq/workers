import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { bridge, BridgeError } from "./bridge";
import { visibleMessages } from "./reducer";
import { useAgentStream } from "./useAgentStream";
import { useSkillsIndex } from "./useSkillsIndex";
import { ApprovalRow } from "./components/ApprovalRow";
import { AuthPanel, type AuthPanelHandle } from "./components/AuthPanel";
import { Composer } from "./components/Composer";
import { ContextMeter } from "./components/ContextMeter";
import { ControlsBar } from "./components/ControlsBar";
import { CostPanel } from "./components/CostPanel";
import { FilesystemPanel } from "./components/FilesystemPanel";
import { FunctionPalette } from "./components/FunctionPalette";
import { SessionList, fetchSessions } from "./components/SessionList";
import { SessionView } from "./components/SessionView";
import { StatusPill } from "./components/StatusPill";
import { useGlobalShortcut } from "./useGlobalShortcut";
import { loadWorkspace, saveWorkspace } from "./workspace";
import type {
  AgentMessage,
  AuthStatus,
  ModelInfo,
  SessionRow,
} from "./types";

type Tab = "chat" | "cost" | "files";

// Tool schemas are no longer shipped from the client. The harness builds the
// LLM tool catalog server-side from `engine::functions::list` (see
// turn-orchestrator/src/tools_catalog.rs). To widen or narrow the model's
// surface, edit which workers register tools — not this file.
//
// Permission still lives in `policy-denylist`, which subscribes to
// `agent::before_tool_call` and refuses by name. Set its env var when
// starting the worker, e.g.:
//   POLICY_DENIED_TOOLS=shell::filesystem::rm,shell::filesystem::sed,shell::filesystem::edit,shell::filesystem::chmod,shell::filesystem::mv

const BASE_SYSTEM_PROMPT =
  "You have filesystem tools that operate inside a sandbox. Use them when the user asks to read, inspect, create, or modify files. Paths must be absolute (e.g. /tmp/notes.md). Some destructive ops may be denied by policy — if a tool result contains `blocked`, explain which policy refused and stop, do not retry.";

function buildSystemPrompt(skillsIndex: string | null, cwd: string): string {
  const cwdSection = cwd
    ? `## Working directory\n${cwd}\nPrefer paths under this directory. Use absolute paths.\n\n`
    : "";

  const skillsSection = skillsIndex
    ? `## Available skills

${skillsIndex}

Use the \`skill::fetch\` tool to load any \`iii://\` URI you see above when you need its full content.`
    : "## Available skills\n\n(Skills index not loaded — call `skill::fetch` with `uri: \"iii://skills\"` to discover what's registered.)";

  return `${BASE_SYSTEM_PROMPT}\n\n${cwdSection}${skillsSection}`;
}

// Providers we have actual workers for in iii.worker.yaml. Don't add others
// here — they'd appear in the UI but every send would error with "function
// not found: provider::<name>::complete".
const SUPPORTED_PROVIDERS = ["anthropic", "openai"] as const;
type Provider = (typeof SUPPORTED_PROVIDERS)[number];

const DEFAULT_MODEL_BY_PROVIDER: Record<Provider, string> = {
  anthropic: "claude-opus-4-7",
  openai: "gpt-5",
};

const APPROVAL_REQUIRED = [
  "shell::filesystem::write",
  "shell::filesystem::mkdir",
];

function newSessionId(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `s${d.getMonth() + 1}${pad(d.getDate())}-${pad(d.getHours())}${pad(
    d.getMinutes(),
  )}${pad(d.getSeconds())}`;
}

export default function App() {
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [draftId, setDraftId] = useState<string | null>(null);
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Per-session working directory. Advisory only — surfaced in the system
  // prompt so the agent prefers paths under it. `loadedCwd` tracks the value
  // currently persisted so blur/Enter can decide whether to call save.
  const [cwd, setCwd] = useState<string>("");
  const [loadedCwd, setLoadedCwd] = useState<string>("");

  // Skills-index fetch is strictly non-blocking. The fallback branch in
  // buildSystemPrompt(null) is the agent's recovery path; do not gate
  // rendering or send() on the index being loaded.
  const { index: skillsIndex } = useSkillsIndex();

  const [provider, setProvider] = useState<Provider>("anthropic");
  const [model, setModel] = useState<string>(DEFAULT_MODEL_BY_PROVIDER.anthropic);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [authByProvider, setAuthByProvider] = useState<
    Record<string, AuthStatus | null>
  >({});

  const authPanelRef = useRef<AuthPanelHandle>(null);

  const [tab, setTab] = useState<Tab>("chat");

  // Cmd-J (Ctrl-J on Linux/Win) opens the bus function palette. Cmd-K is
  // taken by Chrome's address bar focus, so we deliberately picked J.
  const [paletteOpen, setPaletteOpen] = useState(false);
  const openPalette = useCallback(() => setPaletteOpen(true), []);
  useGlobalShortcut(
    { key: "j", meta: true, ctrl: true },
    openPalette,
  );

  const stream = useAgentStream(active);

  useEffect(() => {
    if (!active) return;
    const visible = visibleMessages(stream);
    if (visible.length > 0) setMessages(visible);
  }, [active, stream]);

  const isRunning = loading || stream.status === "running";

  const refreshSessions = useCallback(async () => {
    try {
      setSessions(await fetchSessions());
    } catch {
      // bridge unreachable; StatusPill surfaces it
    }
  }, []);

  const refreshAuth = useCallback(async (p: string) => {
    try {
      const s = await bridge<AuthStatus>("auth::status", { provider: p });
      setAuthByProvider((prev) => ({ ...prev, [p]: s }));
    } catch {
      setAuthByProvider((prev) => ({
        ...prev,
        [p]: { configured: false, source: null, label: null },
      }));
    }
  }, []);

  const loadMessages = useCallback(async (id: string) => {
    try {
      const msgs = await bridge<AgentMessage[]>("state::get", {
        scope: "agent",
        key: `session/${id}/messages`,
      });
      setMessages(Array.isArray(msgs) ? msgs : []);
    } catch (e) {
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
    bridge<{ models: ModelInfo[] }>("models::list")
      .then((r) => setModels(r.models ?? []))
      .catch(() => setModels([]));
  }, []);

  // Resolve auth status for both providers up front so the controls bar
  // tells the truth without round-trips when the user clicks between them.
  useEffect(() => {
    SUPPORTED_PROVIDERS.forEach((p) => void refreshAuth(p));
  }, [refreshAuth]);

  useEffect(() => {
    if (active) void loadMessages(active);
    else setMessages([]);
  }, [active, loadMessages]);

  // Load workspace cwd on session change. Resets to empty for the draft
  // (no active session yet). Errors are swallowed by loadWorkspace.
  useEffect(() => {
    if (!active) {
      setCwd("");
      setLoadedCwd("");
      return;
    }
    let cancelled = false;
    void loadWorkspace(active).then((ws) => {
      if (cancelled) return;
      const value = ws?.cwd ?? "";
      setCwd(value);
      setLoadedCwd(value);
    });
    return () => {
      cancelled = true;
    };
  }, [active]);

  // Persist cwd when it differs from the loaded value. Called from blur and
  // Enter on the header field. Only writes when there's an active session.
  const commitCwd = useCallback(() => {
    if (!active) return;
    const next = cwd.trim();
    if (next === loadedCwd) return;
    void saveWorkspace(active, next).then(() => {
      setLoadedCwd(next);
    });
  }, [active, cwd, loadedCwd]);

  // When the catalog or provider changes, ensure the selected model belongs
  // to the active provider; otherwise pick the configured default or first
  // available model for the provider.
  useEffect(() => {
    const pool = models.filter((m) => m.provider === provider);
    if (pool.length === 0) return;
    if (pool.some((m) => m.id === model)) return;
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

  const send = async (prompt: string) => {
    setError(null);
    const sid = active ?? draftId ?? newSessionId();
    setActive(sid);
    setDraftId(null);
    setLoading(true);

    const optimistic: AgentMessage = {
      role: "user",
      content: [{ type: "text", text: prompt }],
      timestamp: Date.now(),
    };
    const fullHistory = [...messages, optimistic];
    setMessages(fullHistory);

    try {
      await bridge<{ session_id: string }>("run::start", {
        session_id: sid,
        provider,
        model,
        messages: fullHistory,
        system_prompt: buildSystemPrompt(skillsIndex, cwd.trim()),
        approval_required: APPROVAL_REQUIRED,
      });
      void refreshSessions();
    } catch (e) {
      const msg = e instanceof BridgeError ? e.message : String(e);
      setError(msg);
    } finally {
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

  return (
    <div className="app">
      <header className="app-head">
        <div className="app-mark">
          <span className="app-mark-glyph">⌘</span>
          <span className="app-mark-name">harness</span>
          <span className="app-mark-sub">bus console</span>
          {sessionId ? (
            <span className="app-mark-sub" title="session id">
              · {sessionId}
            </span>
          ) : null}
          <input
            type="text"
            className="app-head-cwd"
            value={cwd}
            placeholder="working directory (optional)"
            spellCheck={false}
            disabled={!active}
            onChange={(e) => setCwd(e.target.value)}
            onBlur={commitCwd}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                commitCwd();
                (e.target as HTMLInputElement).blur();
              }
            }}
            aria-label="working directory"
          />
        </div>
        <StatusPill />
      </header>

      <div className="app-body">
        <SessionList
          sessions={sessions}
          active={active}
          onPick={(id) => {
            setActive(id);
            setDraftId(null);
            setError(null);
          }}
          onNew={startNew}
        />

        <main className="main">
          <nav className="tabs" role="tablist" aria-label="harness panels">
            {(["chat", "cost", "files"] as Tab[]).map((t) => (
              <button
                key={t}
                type="button"
                role="tab"
                className="tab"
                data-active={tab === t}
                aria-selected={tab === t}
                onClick={() => setTab(t)}
              >
                {t}
              </button>
            ))}
          </nav>

          {tab === "chat" ? (
            <>
              <ControlsBar
                providers={SUPPORTED_PROVIDERS}
                provider={provider}
                onProvider={(p) => setProvider(p as Provider)}
                models={models}
                model={model}
                onModel={setModel}
                authStatus={currentAuth}
                onManageAuth={() => authPanelRef.current?.open()}
              />
              <AuthPanel
                ref={authPanelRef}
                provider={provider}
                status={currentAuth}
                onStored={() => void refreshAuth(provider)}
              />
              <SessionView
                sessionId={sessionId}
                messages={messages}
                loading={isRunning}
              />
              {error ? (
                <p className="app-error" role="alert">
                  {error}
                </p>
              ) : null}
              <ContextMeter
                messages={messages}
                contextWindow={contextWindow}
                model={model}
              />
              <ApprovalRow
                sessionId={active ?? ""}
                pending={stream.pendingApprovals}
              />
              <Composer
                disabled={composerDisabled}
                onSend={send}
                cwd={cwd.trim()}
                skillsIndex={skillsIndex}
                sessionMessages={messages}
                callbacks={{
                  onNew: startNew,
                  onClear: () => setError(null),
                  onCwd: (path) => {
                    setCwd(path);
                    if (active) void saveWorkspace(active, path).then(() => setLoadedCwd(path));
                  },
                  onModel: (id) => setModel(id),
                  onProvider: (name) => {
                    if ((SUPPORTED_PROVIDERS as readonly string[]).includes(name)) {
                      setProvider(name as Provider);
                    }
                  },
                  onHelp: () => {
                    setError(
                      "shortcuts: / commands · @ files · ↑ history · shift+enter newline · enter send",
                    );
                  },
                }}
              />
            </>
          ) : null}

          {tab === "cost" ? <CostPanel /> : null}
          {tab === "files" ? <FilesystemPanel /> : null}
        </main>
      </div>

      <footer className="app-foot">
        <span>provider · {provider}</span>
        <span>model · {model}</span>
        <span>endpoint · POST /bridge/trigger</span>
        <span>shortcut · ⌘J palette</span>
      </footer>

      <FunctionPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
      />
    </div>
  );
}
