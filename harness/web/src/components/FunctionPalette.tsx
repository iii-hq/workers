// Bus function palette (Cmd-J): list every function on the bus, drill in to
// any one, edit a JSON payload, send it through /harness/call. JSON-first
// — no schema-driven form generator in v1.
//
// Behavior summary (see plan doc Phase B step F):
//   * Cmd-J opens the palette (wired in App.tsx via useGlobalShortcut).
//   * List view: filter input, ↑/↓ navigate, Enter drills in, Esc closes.
//   * Tabs: Functions (live) and Recent (last-20 from THIS browser).
//   * Drill-in: function header, JSON textarea (pre-filled with `{}` or the
//     most-recent payload for that fn), Send button, response viewer,
//     "Copy as curl".
//   * Sensitive functions (policy::*, auth::set_*, shell fs writes) get a
//     two-step Enter-to-confirm Send.
//   * Click backdrop or Esc to close. Focus returns to whatever was focused
//     before opening.

import {
  type KeyboardEvent as ReactKeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { bridge, BridgeError } from "../bridge";
import {
  type FunctionEntry,
  type RecentInvocation,
  curlForBridge,
  filterPalette,
  isSensitive,
  loadRecent,
  pushRecent,
  workerFromId,
} from "../palette";

interface Props {
  open: boolean;
  onClose: () => void;
}

type View = { kind: "list" } | { kind: "drill"; entry: FunctionEntry };

// Same denylist semantics the harness uses server-side for the LLM `agent_call`
// surface (see turn-orchestrator/src/agent_call.rs). The palette is a
// power-user surface, so we show ONE more layer than the LLM gets:
// `auth::*`, `policy::*`, `shell::*` ARE shown here (with sensitive-call
// gating). Only pure engine plumbing is hidden.
const PALETTE_HIDDEN_PREFIXES = [
  "engine::",
  "ui::",
  "agent::",
];

function isHidden(fnId: string): boolean {
  return PALETTE_HIDDEN_PREFIXES.some((p) => fnId.startsWith(p));
}

/**
 * engine::functions::list returns either an array directly or a
 * `{functions: [...]}` envelope (see agent_call.rs / engine list helpers).
 * Normalize to a plain array of FunctionEntry, dropping anything without a
 * function_id string and any hidden plumbing prefixes.
 */
function normalizeFunctions(raw: unknown): FunctionEntry[] {
  const arr: unknown[] = Array.isArray(raw)
    ? raw
    : Array.isArray((raw as { functions?: unknown[] } | null)?.functions)
      ? ((raw as { functions: unknown[] }).functions)
      : [];
  const out: FunctionEntry[] = [];
  for (const item of arr) {
    if (typeof item !== "object" || item === null) continue;
    const rec = item as Record<string, unknown>;
    const fnId = rec.function_id;
    if (typeof fnId !== "string" || fnId.length === 0) continue;
    if (isHidden(fnId)) continue;
    out.push({
      ...rec,
      function_id: fnId,
      description:
        typeof rec.description === "string" ? rec.description : undefined,
    });
  }
  out.sort((a, b) => a.function_id.localeCompare(b.function_id));
  return out;
}

const prefersReducedMotion = (): boolean => {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
};

export function FunctionPalette({ open, onClose }: Props) {
  const [view, setView] = useState<View>({ kind: "list" });
  const [tab, setTab] = useState<"functions" | "recent">("functions");
  const [items, setItems] = useState<FunctionEntry[]>([]);
  const [recent, setRecent] = useState<RecentInvocation[]>([]);
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const filterRef = useRef<HTMLInputElement | null>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const listRef = useRef<HTMLUListElement | null>(null);
  const reducedMotion = useRef<boolean>(false);

  // Reset state every time the palette opens. Capture focus to restore on
  // close, kick the function list fetch, and pull recent from localStorage.
  useEffect(() => {
    if (!open) return;
    reducedMotion.current = prefersReducedMotion();
    restoreFocusRef.current = (document.activeElement as HTMLElement) ?? null;
    setView({ kind: "list" });
    setTab("functions");
    setQuery("");
    setSelectedIndex(0);
    setError(null);
    setRecent(loadRecent());

    let cancelled = false;
    setLoading(true);
    bridge<unknown>("engine::functions::list", {})
      .then((raw) => {
        if (cancelled) return;
        setItems(normalizeFunctions(raw));
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof BridgeError ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    // Defer focus until after the modal mounts.
    const id = window.setTimeout(() => filterRef.current?.focus(), 0);

    return () => {
      cancelled = true;
      window.clearTimeout(id);
    };
  }, [open]);

  // On close: restore focus to the previously-active element.
  useEffect(() => {
    if (open) return;
    const prev = restoreFocusRef.current;
    if (prev && typeof prev.focus === "function") {
      // Only restore if the element is still in the DOM.
      if (document.body.contains(prev)) prev.focus();
    }
  }, [open]);

  // Filter applied to the active tab.
  const visible: FunctionEntry[] = useMemo(() => {
    if (tab === "functions") return filterPalette(items, query);
    // Recent tab: build entries from recent list, intersect with items map
    // for descriptions when available, then filter.
    const byId = new Map(items.map((i) => [i.function_id, i] as const));
    const recentEntries: FunctionEntry[] = recent.map((r) => {
      const known = byId.get(r.function_id);
      return (
        known ?? { function_id: r.function_id, description: undefined }
      );
    });
    return filterPalette(recentEntries, query);
  }, [tab, items, query, recent]);

  // Clamp selection when the visible list changes.
  useEffect(() => {
    if (selectedIndex >= visible.length) {
      setSelectedIndex(visible.length === 0 ? 0 : visible.length - 1);
    }
  }, [visible.length, selectedIndex]);

  const drillInto = useCallback((entry: FunctionEntry) => {
    setView({ kind: "drill", entry });
    setError(null);
  }, []);

  const backToList = useCallback(() => {
    setView({ kind: "list" });
    setError(null);
    // Refocus the filter on return.
    window.setTimeout(() => filterRef.current?.focus(), 0);
  }, []);

  // List view key handler (filter input + arrow nav + Enter/Esc).
  const onListKey = useCallback(
    (e: ReactKeyboardEvent<HTMLDivElement>) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, Math.max(visible.length - 1, 0)));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const entry = visible[selectedIndex];
        if (entry) drillInto(entry);
      }
    },
    [visible, selectedIndex, drillInto, onClose],
  );

  if (!open) return null;

  return (
    <div
      className="palette-backdrop"
      role="presentation"
      onClick={onClose}
      data-reduced-motion={reducedMotion.current ? "true" : "false"}
    >
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="bus function palette"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={view.kind === "list" ? onListKey : undefined}
      >
        {view.kind === "list" ? (
          <PaletteList
            items={visible}
            tab={tab}
            onTab={(t) => {
              setTab(t);
              setSelectedIndex(0);
            }}
            query={query}
            onQuery={(q) => {
              setQuery(q);
              setSelectedIndex(0);
            }}
            selectedIndex={selectedIndex}
            onPick={drillInto}
            loading={loading}
            error={error}
            filterRef={filterRef}
            listRef={listRef}
          />
        ) : (
          <PaletteDrill
            entry={view.entry}
            recent={recent}
            onBack={backToList}
            onClose={onClose}
            onAfterSend={(updated) => setRecent(updated)}
          />
        )}
      </div>
    </div>
  );
}

// ---- list view ----

interface ListProps {
  items: FunctionEntry[];
  tab: "functions" | "recent";
  onTab: (t: "functions" | "recent") => void;
  query: string;
  onQuery: (q: string) => void;
  selectedIndex: number;
  onPick: (entry: FunctionEntry) => void;
  loading: boolean;
  error: string | null;
  filterRef: React.MutableRefObject<HTMLInputElement | null>;
  listRef: React.MutableRefObject<HTMLUListElement | null>;
}

function PaletteList({
  items,
  tab,
  onTab,
  query,
  onQuery,
  selectedIndex,
  onPick,
  loading,
  error,
  filterRef,
  listRef,
}: ListProps) {
  const activeId =
    items[selectedIndex] != null
      ? `palette-opt-${items[selectedIndex].function_id}`
      : undefined;

  // Scroll the selected option into view as the user arrows through.
  useEffect(() => {
    if (!listRef.current || !activeId) return;
    const el = listRef.current.querySelector<HTMLElement>(`#${CSS.escape(activeId)}`);
    if (el && typeof el.scrollIntoView === "function") {
      el.scrollIntoView({ block: "nearest" });
    }
  }, [activeId, listRef]);

  return (
    <div className="palette-list-view">
      <div className="palette-tabs" role="tablist" aria-label="palette source">
        {(["functions", "recent"] as const).map((t) => (
          <button
            key={t}
            type="button"
            role="tab"
            className="palette-tab"
            data-active={tab === t}
            aria-selected={tab === t}
            onClick={() => onTab(t)}
          >
            {t}
          </button>
        ))}
      </div>
      <input
        ref={filterRef}
        type="text"
        className="palette-filter"
        placeholder={
          tab === "functions"
            ? "filter functions… (↑/↓ Enter Esc)"
            : "filter recent invocations…"
        }
        value={query}
        onChange={(e) => onQuery(e.target.value)}
        aria-label="filter"
        aria-controls="palette-options"
        aria-activedescendant={activeId}
        spellCheck={false}
        autoComplete="off"
      />
      {error ? (
        <p className="palette-error" role="alert">
          {error}
        </p>
      ) : null}
      {loading ? (
        <p className="palette-loading">loading bus functions…</p>
      ) : null}
      {!loading && items.length === 0 ? (
        <p className="palette-empty">
          {tab === "recent"
            ? "no recent invocations from this browser yet"
            : "no functions match your filter"}
        </p>
      ) : null}
      <ul
        ref={listRef}
        id="palette-options"
        className="palette-options"
        role="listbox"
      >
        {items.map((entry, idx) => (
          <li
            key={entry.function_id}
            id={`palette-opt-${entry.function_id}`}
            role="option"
            aria-selected={idx === selectedIndex}
            className="palette-option"
            data-selected={idx === selectedIndex}
            data-sensitive={isSensitive(entry.function_id)}
            onClick={() => onPick(entry)}
          >
            <span className="palette-option-id">{entry.function_id}</span>
            {isSensitive(entry.function_id) ? (
              <span
                className="palette-chip palette-chip-sensitive"
                title="this function mutates state or escalates authority"
              >
                mutates / sensitive
              </span>
            ) : null}
            {entry.description ? (
              <span className="palette-option-desc">{entry.description}</span>
            ) : null}
            <span className="palette-option-worker">
              {workerFromId(entry.function_id)}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

// ---- drill-in view ----

interface DrillProps {
  entry: FunctionEntry;
  recent: RecentInvocation[];
  onBack: () => void;
  onClose: () => void;
  onAfterSend: (updated: RecentInvocation[]) => void;
}

function PaletteDrill({
  entry,
  recent,
  onBack,
  onClose,
  onAfterSend,
}: DrillProps) {
  const sensitive = isSensitive(entry.function_id);
  const lastForFn = recent.find((r) => r.function_id === entry.function_id);
  const initialPayload = lastForFn
    ? safeStringify(lastForFn.payload)
    : "{}";

  const [payloadText, setPayloadText] = useState<string>(initialPayload);
  const [response, setResponse] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [copyHint, setCopyHint] = useState<string | null>(null);

  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // Focus the textarea on mount so the user can immediately edit.
  useEffect(() => {
    const id = window.setTimeout(() => textareaRef.current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, []);

  const parsePayload = useCallback((): {
    ok: true;
    value: unknown;
  } | { ok: false; error: string } => {
    const trimmed = payloadText.trim();
    if (trimmed.length === 0) return { ok: true, value: {} };
    try {
      return { ok: true, value: JSON.parse(trimmed) };
    } catch (e) {
      return {
        ok: false,
        error: `invalid JSON: ${e instanceof Error ? e.message : String(e)}`,
      };
    }
  }, [payloadText]);

  const doSend = useCallback(async () => {
    const parsed = parsePayload();
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    setError(null);
    setResponse(null);
    setPending(true);
    setConfirming(false);
    let ok = true;
    let body: unknown;
    try {
      body = await bridge<unknown>(
        entry.function_id,
        parsed.value as Record<string, unknown>,
      );
      setResponse(safeStringify(body));
    } catch (e) {
      ok = false;
      const msg = e instanceof BridgeError ? e.message : String(e);
      setError(msg);
      setResponse(null);
    } finally {
      setPending(false);
    }
    const updated = pushRecent({
      function_id: entry.function_id,
      payload: parsed.value,
      ts: Date.now(),
      ok,
    });
    onAfterSend(updated);
  }, [entry.function_id, parsePayload, onAfterSend]);

  // Send button: for sensitive functions, click → "press Enter to confirm",
  // then a second activation actually sends. For non-sensitive, click sends.
  const onSendClick = useCallback(() => {
    if (!sensitive) {
      void doSend();
      return;
    }
    if (confirming) {
      void doSend();
    } else {
      setConfirming(true);
    }
  }, [sensitive, confirming, doSend]);

  // Allow Enter (without Shift) inside the JSON textarea to advance the
  // confirm flow when the sensitive prompt is up. Otherwise Enter is a
  // newline (default).
  const onTextareaKey = useCallback(
    (e: ReactKeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onBack();
        return;
      }
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        onSendClick();
      }
    },
    [onBack, onSendClick],
  );

  const onDrillKey = useCallback(
    (e: ReactKeyboardEvent<HTMLDivElement>) => {
      if (e.key === "Escape") {
        e.preventDefault();
        // First Esc: back to list. The list-view Esc handler closes.
        onBack();
      }
    },
    [onBack],
  );

  const copyCurl = useCallback(async () => {
    const parsed = parsePayload();
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    const cmd = curlForBridge(entry.function_id, parsed.value);
    try {
      if (
        typeof navigator !== "undefined" &&
        navigator.clipboard?.writeText
      ) {
        await navigator.clipboard.writeText(cmd);
        setCopyHint("copied");
      } else {
        setCopyHint("clipboard unavailable");
      }
    } catch {
      setCopyHint("copy failed");
    } finally {
      window.setTimeout(() => setCopyHint(null), 1500);
    }
  }, [entry.function_id, parsePayload]);

  return (
    <div className="palette-drill" onKeyDown={onDrillKey}>
      <div className="palette-drill-head">
        <button
          type="button"
          className="palette-back"
          onClick={onBack}
          aria-label="back to list"
        >
          ← back
        </button>
        <div className="palette-drill-id">
          <span className="palette-drill-fn">{entry.function_id}</span>
          <span className="palette-drill-worker">
            worker · {workerFromId(entry.function_id)}
          </span>
        </div>
        <button
          type="button"
          className="palette-back"
          onClick={onClose}
          aria-label="close palette"
        >
          ✕
        </button>
      </div>
      {entry.description ? (
        <p className="palette-drill-desc">{entry.description}</p>
      ) : null}
      <label className="palette-drill-label" htmlFor="palette-payload">
        payload (JSON)
      </label>
      <textarea
        ref={textareaRef}
        id="palette-payload"
        className="palette-drill-textarea"
        value={payloadText}
        onChange={(e) => {
          setPayloadText(e.target.value);
          if (confirming) setConfirming(false);
        }}
        onKeyDown={onTextareaKey}
        spellCheck={false}
        rows={8}
      />
      <div className="palette-drill-actions">
        {sensitive ? (
          <span
            className="palette-chip palette-chip-sensitive"
            aria-label="sensitive function"
          >
            mutates / sensitive
          </span>
        ) : null}
        <button
          type="button"
          className="palette-send"
          data-confirming={confirming}
          onClick={onSendClick}
          disabled={pending}
        >
          {pending
            ? "sending…"
            : confirming
              ? "press again to confirm"
              : sensitive
                ? "send (confirm)"
                : "send"}
        </button>
        <button
          type="button"
          className="palette-copy"
          onClick={() => void copyCurl()}
        >
          copy as curl
        </button>
        {copyHint ? (
          <span className="palette-copy-hint">{copyHint}</span>
        ) : null}
      </div>
      {error ? (
        <p className="palette-error" role="alert">
          {error}
        </p>
      ) : null}
      {response != null ? (
        <pre className="palette-response" aria-label="response">
          {response}
        </pre>
      ) : null}
    </div>
  );
}

function safeStringify(v: unknown): string {
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}
