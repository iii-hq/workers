import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { bridge, BridgeError } from "./bridge";
import { disposeIiiClient, getIiiClient } from "./iii-client";
import { exportJson, exportMd } from "./export";
import { loadMessagesWithEntryIds } from "./loadMessages";
import { visibleMessages } from "./reducer";
import { useAgentStream } from "./useAgentStream";
import { ApprovalRow } from "./components/ApprovalRow";
import { AuthPanel, type AuthPanelHandle } from "./components/AuthPanel";
import { Composer } from "./components/Composer";
import { ContextMeter } from "./components/ContextMeter";
import { ControlsBar } from "./components/ControlsBar";
import { CostPanel } from "./components/CostPanel";
import { FilesystemPanel } from "./components/FilesystemPanel";
import { FootStatus } from "./components/FootStatus";
import { FunctionPalette } from "./components/FunctionPalette";
import { SessionList, fetchSessions } from "./components/SessionList";
import { SessionView } from "./components/SessionView";
import { StatusPill } from "./components/StatusPill";
import { StatusStrip } from "./components/StatusStrip";
import { StatusTab } from "./components/StatusTab";
import { useConnection } from "./useConnection";
import { useGlobalShortcut } from "./useGlobalShortcut";
import { useStatus } from "./useStatus";
import { loadWorkspace, saveWorkspace } from "./workspace";
import type {
  AgentMessage,
  AuthStatus,
  ModelInfo,
  SessionRow,
} from "./types";

type Tab = "chat" | "cost" | "files" | "status";

// Function catalog: a single `agent_call` tool plus server-built system prompt —
// see turn-orchestrator `agent_call.rs` and `system_prompt.rs`. The client
// does not send `tools` or `system_prompt` on `run::start` (override still
// accepted if you pass a non-empty `system_prompt` for experiments).
//
// Permission still lives in `policy-denylist`, which subscribes to
// `agent::before_function_call` and refuses by function id. Set its env var when
// starting the worker, e.g.:
//   POLICY_DENIED_FUNCTIONS=shell::fs::rm,shell::fs::sed,shell::fs::edit,shell::fs::chmod,shell::fs::mv
// (Legacy `POLICY_DENIED_TOOLS` is still read if the new name is unset.)

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
  "shell::fs::write",
  "shell::fs::mkdir",
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
  // Parallel array to `messages`. Slot is the entry_id keying that message
  // in session-tree, or `null` when the message came from the state::*
  // fallback (drift case — fork is disabled for null entries).
  //
  // INVARIANT: `messageEntryIds.length === messages.length` at all times.
  // Stream events (SSE today) don't carry entry_ids yet — we fill with
  // `null` on stream-driven updates. Step B (WS migration) will let stream
  // events carry real ids and this can become live data.
  const [messageEntryIds, setMessageEntryIds] = useState<(string | null)[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Per-session working directory. Advisory only — surfaced in the system
  // prompt so the agent prefers paths under it. `loadedCwd` tracks the value
  // currently persisted so blur/Enter can decide whether to call save.
  const [cwd, setCwd] = useState<string>("");
  const [loadedCwd, setLoadedCwd] = useState<string>("");

  // Working directory is sent on `run::start` for server-side system prompt
  // assembly; skills index is fetched in turn-orchestrator provisioning.
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

  // Live ambient status — header chip + foot chips + status tab.
  // The hook owns the rolling 200-event buffer and the per-page subscription
  // to all-sessions topics (cost/workers/approvals).
  const status = useStatus();
  const connection = useConnection();
  const isConnected = connection.status === "connected";

  useEffect(() => {
    if (!active) return;
    const visible = visibleMessages(stream);
    if (visible.length > 0) {
      setMessages(visible);
      // Stream events don't carry entry_ids today (Phase B step B will fix
      // this when WS replaces SSE). Until then, fork buttons stay disabled
      // for live messages — the next loadSessionMessages refresh hydrates
      // entry_ids from session-tree.
      setMessageEntryIds(visible.map(() => null));
    }
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

  const loadSessionMessages = useCallback(async (id: string) => {
    try {
      const pairs = await loadMessagesWithEntryIds(id);
      setMessages(pairs.map((p) => p.message));
      setMessageEntryIds(pairs.map((p) => p.entry_id));
    } catch {
      setMessages([]);
      setMessageEntryIds([]);
    }
  }, []);

  // Refresh sessions when the harness fanout pushes `ui::sessions::changed`.
  // Replaces the 4-second polling interval. The all-sessions subscription is
  // owned by App for the lifetime of the page; per-session subscriptions are
  // managed by useAgentStream.
  useEffect(() => {
    void refreshSessions();
    let cancelled = false;
    let off: (() => void) | undefined;
    let subscribed = false;
    let browserId: string | null = null;

    void (async () => {
      try {
        const client = await getIiiClient();
        if (cancelled) return;
        browserId = client.browserId;
        off = client.on("ui::sessions::changed", () => {
          void refreshSessions();
        });
        await client.call("ui::subscribe", {
          browser_id: browserId,
          session_id: null,
        });
        subscribed = true;
      } catch {
        // No connection — refreshSessions above already ran once. The status
        // pill will show the disconnected state via useConnection.
      }
    })();

    return () => {
      cancelled = true;
      off?.();
      if (subscribed && browserId) {
        void getIiiClient().then((client) =>
          client
            .call("ui::unsubscribe", {
              browser_id: browserId,
              session_id: null,
            })
            .catch(() => {}),
        );
      }
    };
  }, [refreshSessions]);

  // Tear down the iii-client on page unload so the fanout can drop our
  // subscriptions promptly. The browser will close the WS regardless, but
  // an explicit shutdown lets the engine clean up registered handlers.
  useEffect(() => {
    const handler = () => {
      void disposeIiiClient();
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, []);

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
    if (active) void loadSessionMessages(active);
    else {
      setMessages([]);
      setMessageEntryIds([]);
    }
  }, [active, loadSessionMessages]);

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
    setMessageEntryIds([]);
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
    // Optimistic message has no entry_id yet — pad to keep arrays in sync.
    setMessageEntryIds([...messageEntryIds, null]);

    try {
      await bridge<{ session_id: string }>("run::start", {
        session_id: sid,
        provider,
        model,
        messages: fullHistory,
        approval_required: APPROVAL_REQUIRED,
        ...(cwd.trim() ? { cwd: cwd.trim() } : {}),
      });
      void refreshSessions();
    } catch (e) {
      const msg = e instanceof BridgeError ? e.message : String(e);
      setError(msg);
    } finally {
      setLoading(false);
    }
  };

  // Fork the active session at a specific message's entry_id. The new
  // session's id is returned by session-tree::fork and becomes the active
  // session. Refreshing the rail picks up the new row.
  const handleForkFromMessage = useCallback(
    async (entryId: string) => {
      if (!active) return;
      try {
        const { session_id } = await bridge<{ session_id: string }>(
          "session-tree::fork",
          {
            source_session_id: active,
            from_entry_id: entryId,
          },
        );
        setActive(session_id);
        void refreshSessions();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [active, refreshSessions],
  );

  // /repair — read state::* snapshot, hand it to session-tree::reconcile
  // so any drifted rows get re-keyed with entry_ids. Reload after so the
  // UI's entry_ids reflect the new tree state.
  const handleRepair = useCallback(async () => {
    if (!active) return;
    try {
      const snapshot = await bridge<unknown>("state::get", {
        scope: "agent",
        key: `session/${active}/messages`,
      });
      const result = await bridge<{ repaired: number }>(
        "session-tree::reconcile",
        { session_id: active, state_snapshot: snapshot },
      );
      void loadSessionMessages(active);
      if (result.repaired === 0) setError(null);
    } catch (e) {
      setError(`/repair failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [active, loadSessionMessages]);

  // /fork — convenience: fork from the most recent message that has a
  // real entry_id. Anything without an entry_id is a stream artifact or
  // a drifted state row.
  const handleForkLast = useCallback(async () => {
    if (!active) return;
    for (let i = messageEntryIds.length - 1; i >= 0; i--) {
      const id = messageEntryIds[i];
      if (id !== null) {
        await handleForkFromMessage(id);
        return;
      }
    }
    setError("No messages with entry_ids — try /repair first");
  }, [active, messageEntryIds, handleForkFromMessage]);

  const handleExport = useCallback(
    async (format: "md" | "json") => {
      if (!active) return;
      try {
        if (format === "md") await exportMd(active);
        else await exportJson(active);
      } catch (e) {
        setError(`/export failed: ${e instanceof Error ? e.message : String(e)}`);
      }
    },
    [active],
  );

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
        <div className="app-head-right">
          <StatusStrip
            pendingApprovals={status.pendingApprovals}
            connected={isConnected}
            onJumpToStatus={() => setTab("status")}
          />
          <StatusPill />
        </div>
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
            {(["chat", "cost", "files", "status"] as Tab[]).map((t) => (
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
                messageEntryIds={messageEntryIds}
                loading={isRunning}
                onForkFromMessage={handleForkFromMessage}
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
                skillsIndex={null}
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
                  onRepair: handleRepair,
                  onFork: handleForkLast,
                  onExport: handleExport,
                }}
              />
            </>
          ) : null}

          {tab === "cost" ? <CostPanel /> : null}
          {tab === "files" ? <FilesystemPanel /> : null}
          {tab === "status" ? (
            <StatusTab
              cost={status.cost}
              workers={status.workers}
              events={status.events}
              hydrated={status.hydrated}
              connected={isConnected}
              connectionSince={connection.since}
              onClearEvents={status.clearEvents}
            />
          ) : null}
        </main>
      </div>

      <footer className="app-foot">
        <span>provider · {provider}</span>
        <span>model · {model}</span>
        <span>endpoint · POST /bridge/trigger</span>
        <span>shortcut · ⌘J palette</span>
        <span className="app-foot-spacer" />
        <FootStatus
          cost={status.cost}
          workers={status.workers}
          connected={isConnected}
        />
      </footer>

      <FunctionPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
      />
    </div>
  );
}
