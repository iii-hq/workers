import {
  FormEvent,
  KeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { bridge, BridgeError } from "../bridge";
import {
  extractQuery,
  shouldOpenAt,
  shouldOpenSlash,
  useCommandMenu,
  type MenuItem,
} from "../useCommandMenu";
import {
  BUILT_IN_COMMANDS,
  filterCommands,
  skillsIndexToMenuItems,
} from "../menuItems";
import type { AgentMessage, ToolResult, FsLsDetails, FsEntry } from "../types";

// ─── Built-in command callbacks ────────────────────────────────────────────
// Each /name route the Composer can dispatch. Optional callbacks; the
// Composer falls back to a sensible default for missing handlers.
export interface ComposerCallbacks {
  onNew?: () => void;
  onClear?: () => void;
  onCwd?: (path: string) => void;
  onModel?: (id: string) => void;
  onProvider?: (name: string) => void;
  onHelp?: () => void;
  /** /repair — reconciles session-tree against state::* snapshot. */
  onRepair?: () => void | Promise<void>;
  /** /fork — forks the active session from the last message with an entry_id. */
  onFork?: () => void | Promise<void>;
  /** /export md|json — downloads a transcript file via the browser. */
  onExport?: (format: "md" | "json") => void | Promise<void>;
}

interface Props {
  disabled: boolean;
  onSend: (prompt: string) => Promise<void>;
  /** Working directory used as the @-mention browse root. Empty = unset. */
  cwd: string;
  /** Markdown skills index for slash menu (`null` → built-in commands only; full index is loaded server-side for the model). */
  skillsIndex: string | null;
  /** Prior messages of the active session — drives ↑ history walk. */
  sessionMessages: AgentMessage[];
  /** Per-builtin handlers. */
  callbacks?: ComposerCallbacks;
}

const AT_PAGE_SIZE = 25;

interface AtBrowseState {
  /** Directory currently being listed. Empty when no @-mention is active. */
  dir: string;
  /** Raw entries returned by `shell::filesystem::ls` for `dir`. */
  entries: FsEntry[];
  loading: boolean;
  /** Empty-state reason, mapped to user-visible text. */
  error: "cwd-unset" | "permission" | "fetch-failed" | "io" | null;
}

const AT_BROWSE_INITIAL: AtBrowseState = {
  dir: "",
  entries: [],
  loading: false,
  error: null,
};

function joinPath(base: string, name: string): string {
  if (!base) return name;
  if (base.endsWith("/")) return `${base}${name}`;
  return `${base}/${name}`;
}

function buildSkillsItems(index: string | null): MenuItem[] {
  return [...BUILT_IN_COMMANDS, ...skillsIndexToMenuItems(index)];
}

function entriesToMenuItems(dir: string, entries: FsEntry[]): MenuItem[] {
  const sorted = [...entries].sort((a, b) => {
    const aDir = a.kind === "dir" ? 1 : 0;
    const bDir = b.kind === "dir" ? 1 : 0;
    if (aDir !== bDir) return bDir - aDir;
    return a.name.localeCompare(b.name);
  });
  return sorted.map((e) => {
    const abs = joinPath(dir, e.name);
    const isDir = e.kind === "dir";
    return {
      kind: "file" as const,
      id: abs,
      label: e.name + (isDir ? "/" : ""),
      description: abs,
      meta: { isDir, dir },
    };
  });
}

/** Pull all prior user-text messages for ↑ history walk. */
function userTexts(messages: AgentMessage[]): string[] {
  const out: string[] = [];
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.role !== "user") continue;
    const text = m.content
      .filter((b): b is { type: "text"; text: string } => b.type === "text")
      .map((b) => b.text)
      .join("");
    if (text.trim().length > 0) out.push(text);
  }
  return out;
}

export function Composer({
  disabled,
  onSend,
  cwd,
  skillsIndex,
  sessionMessages,
  callbacks,
}: Props) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [menu, dispatch] = useCommandMenu();
  const [atBrowse, setAtBrowse] = useState<AtBrowseState>(AT_BROWSE_INITIAL);
  const [atPage, setAtPage] = useState(1);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // Memoized so reference equality is stable for the menu effect.
  const slashCatalog = useMemo(() => buildSkillsItems(skillsIndex), [skillsIndex]);
  const history = useMemo(() => userTexts(sessionMessages), [sessionMessages]);

  // ─── @-mention IO ───────────────────────────────────────────────────────
  // Whenever atBrowse.dir changes, fetch its contents. Errors map to a typed
  // empty-state so the popover can render clear text without re-throwing.
  useEffect(() => {
    if (!atBrowse.dir) return;
    let cancelled = false;
    setAtBrowse((s) => ({ ...s, loading: true, error: null }));
    bridge<ToolResult<FsLsDetails>>("shell::filesystem::ls", {
      path: atBrowse.dir,
    })
      .then((res) => {
        if (cancelled) return;
        const detailsErr = res.details?.error;
        if (detailsErr) {
          setAtBrowse({
            dir: atBrowse.dir,
            entries: [],
            loading: false,
            error: /permission|denied/i.test(detailsErr) ? "permission" : "io",
          });
          return;
        }
        setAtBrowse({
          dir: atBrowse.dir,
          entries: res.details?.entries ?? [],
          loading: false,
          error: null,
        });
        setAtPage(1);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        const isPerm =
          e instanceof BridgeError && /permission|denied/i.test(e.message);
        setAtBrowse({
          dir: atBrowse.dir,
          entries: [],
          loading: false,
          error: isPerm ? "permission" : "fetch-failed",
        });
      });
    return () => {
      cancelled = true;
    };
  }, [atBrowse.dir]);

  // ─── Trigger detection on every text change ─────────────────────────────
  // Composer parses the text + caret position to decide whether the user just
  // entered or remained in slash/at mode. History mode is keyboard-driven only.
  const recomputeMenu = useCallback(
    (nextText: string, caret: number) => {
      // If currently in history mode, leaving it requires Esc/Enter — typing
      // also exits because the textarea no longer matches the recalled msg.
      if (menu.mode === "history") {
        dispatch({ kind: "close" });
      }

      // Slash takes precedence over at when both could be derived (a `/` at
      // line-start always wins; `@` only fires at word-boundaries).
      if (shouldOpenSlash(nextText, caret)) {
        dispatch({ kind: "open-slash", items: slashCatalog });
        return;
      }
      if (shouldOpenAt(nextText, caret)) {
        if (!cwd) {
          // Open the menu in error state so the user sees why nothing listed.
          dispatch({ kind: "open-at", items: [] });
          setAtBrowse({ dir: "", entries: [], loading: false, error: "cwd-unset" });
          return;
        }
        setAtBrowse({ dir: cwd, entries: [], loading: true, error: null });
        dispatch({ kind: "open-at", items: [] });
        return;
      }

      // If a mode is open, refresh items based on the current query.
      if (menu.mode === "slash") {
        const q = extractQuery(nextText, caret, "/");
        if (q == null) {
          dispatch({ kind: "close" });
          return;
        }
        dispatch({
          kind: "filter",
          query: q,
          items: filterCommands(slashCatalog, q),
        });
      } else if (menu.mode === "at") {
        const q = extractQuery(nextText, caret, "@");
        if (q == null) {
          dispatch({ kind: "close" });
          setAtBrowse(AT_BROWSE_INITIAL);
          return;
        }
        // If query ends with `/`, the user is descending into a subdir.
        // We rebase the browse dir and clear the query portion below the slash.
        if (q.endsWith("/") && q.length > 1) {
          const subdir = q.slice(0, q.length - 1);
          const target = subdir.startsWith("/") ? subdir : joinPath(cwd, subdir);
          if (target !== atBrowse.dir) {
            setAtBrowse({ dir: target, entries: [], loading: true, error: null });
          }
        }
        // The sub-string after the last `/` is the per-row filter.
        const trailing = q.split("/").pop() ?? "";
        const items = entriesToMenuItems(atBrowse.dir, atBrowse.entries);
        const filtered =
          trailing.length === 0
            ? items
            : items.filter((it) =>
                it.label.toLowerCase().includes(trailing.toLowerCase()),
              );
        dispatch({ kind: "filter", query: q, items: filtered });
      }
    },
    [menu.mode, dispatch, slashCatalog, cwd, atBrowse.dir, atBrowse.entries],
  );

  // Keep the at-mention popover in sync when its IO completes.
  useEffect(() => {
    if (menu.mode !== "at") return;
    const items = entriesToMenuItems(atBrowse.dir, atBrowse.entries);
    dispatch({ kind: "set-items", items: items.slice(0, atPage * AT_PAGE_SIZE) });
  }, [menu.mode, atBrowse.entries, atBrowse.dir, atPage, dispatch]);

  // ─── Submit + dispatch ──────────────────────────────────────────────────
  const submit = useCallback(
    async (e?: FormEvent) => {
      e?.preventDefault();
      const trimmed = text.trim();
      if (!trimmed || busy) return;

      // Built-in /cwd <path> is intercepted client-side.
      const cwdMatch = /^\/cwd\s+(.+)$/.exec(trimmed);
      if (cwdMatch) {
        callbacks?.onCwd?.(cwdMatch[1].trim());
        setText("");
        return;
      }
      if (trimmed === "/clear") {
        callbacks?.onClear?.();
        setText("");
        return;
      }
      if (trimmed === "/new") {
        callbacks?.onNew?.();
        setText("");
        return;
      }
      if (trimmed === "/help") {
        callbacks?.onHelp?.();
        return;
      }
      if (trimmed === "/repair") {
        void callbacks?.onRepair?.();
        setText("");
        return;
      }
      if (trimmed === "/fork") {
        void callbacks?.onFork?.();
        setText("");
        return;
      }
      if (trimmed === "/export md") {
        void callbacks?.onExport?.("md");
        setText("");
        return;
      }
      if (trimmed === "/export json") {
        void callbacks?.onExport?.("json");
        setText("");
        return;
      }
      const modelMatch = /^\/model\s+(.+)$/.exec(trimmed);
      if (modelMatch) {
        callbacks?.onModel?.(modelMatch[1].trim());
        setText("");
        return;
      }
      const providerMatch = /^\/provider\s+(.+)$/.exec(trimmed);
      if (providerMatch) {
        callbacks?.onProvider?.(providerMatch[1].trim());
        setText("");
        return;
      }

      setBusy(true);
      try {
        await onSend(trimmed);
        setText("");
      } finally {
        setBusy(false);
      }
    },
    [text, busy, onSend, callbacks],
  );

  const acceptItem = useCallback(
    (item: MenuItem) => {
      if (menu.mode === "slash") {
        if (item.kind === "builtin") {
          // Commands that take an arg insert with trailing space and stay in
          // the textarea so the user can type the arg.
          if (
            item.id === "/cwd" ||
            item.id === "/model" ||
            item.id === "/provider"
          ) {
            setText(`${item.id} `);
            dispatch({ kind: "close" });
            return;
          }
          // Zero-arg commands fire their callback immediately.
          if (item.id === "/new") callbacks?.onNew?.();
          if (item.id === "/clear") callbacks?.onClear?.();
          if (item.id === "/help") callbacks?.onHelp?.();
          if (item.id === "/repair") void callbacks?.onRepair?.();
          if (item.id === "/fork") void callbacks?.onFork?.();
          if (item.id === "/export md") void callbacks?.onExport?.("md");
          if (item.id === "/export json") void callbacks?.onExport?.("json");
          setText("");
          dispatch({ kind: "close" });
          return;
        }
        if (item.kind === "skill") {
          // Skills get inserted as a /skill-id mention so the user can add
          // context after it. The agent picks it up via the system-prompt
          // skills index and skill::fetch.
          setText(`${item.id} `);
          dispatch({ kind: "close" });
          return;
        }
      }
      if (menu.mode === "at" && item.kind === "file") {
        const meta = item.meta as { isDir?: boolean } | undefined;
        if (meta?.isDir) {
          // Descend into the directory. Replace the in-progress @<query>
          // with @<absolute>/ so further typing filters within it.
          replaceAtMention(`${item.id}/`);
          setAtBrowse({ dir: item.id, entries: [], loading: true, error: null });
          return;
        }
        // File: insert absolute path and close.
        replaceAtMention(item.id + " ");
        dispatch({ kind: "close" });
        setAtBrowse(AT_BROWSE_INITIAL);
        return;
      }
    },
    [menu.mode, dispatch, callbacks],
  );

  const replaceAtMention = useCallback(
    (replacement: string) => {
      const ta = textareaRef.current;
      if (!ta) return;
      const caret = ta.selectionStart ?? text.length;
      // Walk back to the most recent @ that opened the mode.
      let at = -1;
      for (let i = caret - 1; i >= 0; i--) {
        const c = text[i];
        if (c === "\n") break;
        if (c === "@") {
          at = i;
          break;
        }
      }
      if (at < 0) return;
      const before = text.slice(0, at);
      const after = text.slice(caret);
      const next = before + replacement + after;
      setText(next);
      // Restore caret after the inserted replacement.
      const newCaret = (before + replacement).length;
      requestAnimationFrame(() => {
        ta.setSelectionRange(newCaret, newCaret);
        ta.focus();
      });
    },
    [text],
  );

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      // Menu-active key handling.
      if (menu.mode === "slash" || menu.mode === "at") {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          dispatch({ kind: "move", delta: 1 });
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          dispatch({ kind: "move", delta: -1 });
          return;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          if (menu.items.length > 0) {
            e.preventDefault();
            acceptItem(menu.items[menu.selectedIndex] ?? menu.items[0]);
            return;
          }
        }
        if (e.key === "Escape") {
          e.preventDefault();
          dispatch({ kind: "close" });
          setAtBrowse(AT_BROWSE_INITIAL);
          return;
        }
      }

      if (menu.mode === "history") {
        if (e.key === "ArrowUp") {
          e.preventDefault();
          dispatch({ kind: "history-step", delta: 1, historyLen: history.length });
          return;
        }
        if (e.key === "ArrowDown") {
          e.preventDefault();
          const cur = menu.historyIndex ?? 0;
          if (cur === 0) {
            // Stepping below 0 returns to the draft.
            setText(menu.historyDraft);
            dispatch({ kind: "close" });
            return;
          }
          dispatch({ kind: "history-step", delta: -1, historyLen: history.length });
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          setText(menu.historyDraft);
          dispatch({ kind: "close" });
          return;
        }
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          dispatch({ kind: "close" });
          void submit();
          return;
        }
      }

      // Idle mode default keys.
      if (menu.mode === "idle") {
        if (e.key === "ArrowUp" && text.length === 0 && history.length > 0) {
          e.preventDefault();
          dispatch({ kind: "open-history", draft: text });
          setText(history[0]);
          return;
        }
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          void submit();
          return;
        }
      }
    },
    [
      menu.mode,
      menu.items,
      menu.selectedIndex,
      menu.historyIndex,
      menu.historyDraft,
      dispatch,
      acceptItem,
      history,
      text,
      submit,
    ],
  );

  // When historyIndex changes, mirror it into the textarea.
  useEffect(() => {
    if (menu.mode !== "history") return;
    const idx = menu.historyIndex;
    if (idx == null) return;
    setText(history[idx] ?? menu.historyDraft);
  }, [menu.mode, menu.historyIndex, history, menu.historyDraft]);

  const onChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const next = e.target.value;
    setText(next);
    const caret = e.target.selectionStart ?? next.length;
    recomputeMenu(next, caret);
  };

  const showPopover = menu.mode === "slash" || menu.mode === "at";
  const hasMore =
    menu.mode === "at" && atBrowse.entries.length > atPage * AT_PAGE_SIZE;

  return (
    <form className="composer" onSubmit={submit}>
      <div className="composer-stack">
        {showPopover ? (
          <div className="composer-popover" role="listbox">
            <div className="composer-popover-head">
              {menu.mode === "slash"
                ? `slash ${menu.query ? "/" + menu.query : ""}`
                : `@ ${atBrowse.dir || "(no cwd)"}`}
            </div>
            {renderMenuBody({
              menu,
              atBrowse,
              hasMore,
              onPick: acceptItem,
              onLoadMore: () => setAtPage((p) => p + 1),
            })}
          </div>
        ) : null}
        <textarea
          ref={textareaRef}
          className="composer-input"
          placeholder={
            disabled
              ? "set a credential to begin"
              : "address the agent. / for commands, @ for files, ↑ for history."
          }
          value={text}
          onChange={onChange}
          onKeyDown={onKeyDown}
          disabled={disabled || busy}
          rows={3}
          spellCheck={false}
        />
      </div>
      <button
        type="submit"
        className="composer-send"
        disabled={disabled || busy || !text.trim()}
      >
        <span className="composer-send-mark" aria-hidden>
          ⏎
        </span>
        <span className="composer-send-label">{busy ? "running" : "send"}</span>
      </button>
    </form>
  );
}

interface BodyProps {
  menu: ReturnType<typeof useCommandMenu>[0];
  atBrowse: AtBrowseState;
  hasMore: boolean;
  onPick: (item: MenuItem) => void;
  onLoadMore: () => void;
}

function renderMenuBody({ menu, atBrowse, hasMore, onPick, onLoadMore }: BodyProps) {
  if (menu.mode === "at" && atBrowse.error) {
    return (
      <div className="composer-popover-empty">{atErrorText(atBrowse.error)}</div>
    );
  }
  if (menu.mode === "at" && atBrowse.loading && menu.items.length === 0) {
    return <div className="composer-popover-empty">listing…</div>;
  }
  if (menu.items.length === 0) {
    if (menu.mode === "slash") {
      return (
        <div className="composer-popover-empty">
          {menu.query ? `no command matches /${menu.query}` : "no commands"}
        </div>
      );
    }
    return <div className="composer-popover-empty">empty directory</div>;
  }
  return (
    <ul className="composer-popover-list">
      {menu.items.map((it, i) => (
        <li
          key={it.id}
          className="composer-popover-row"
          data-active={i === menu.selectedIndex}
          role="option"
          aria-selected={i === menu.selectedIndex}
          onMouseDown={(ev) => {
            // mousedown to fire before the textarea blur cancels the popover.
            ev.preventDefault();
            onPick(it);
          }}
        >
          <span className="composer-popover-label">{it.label}</span>
          {it.description ? (
            <span className="composer-popover-desc">{it.description}</span>
          ) : null}
        </li>
      ))}
      {hasMore ? (
        <li
          className="composer-popover-row composer-popover-more"
          role="option"
          onMouseDown={(ev) => {
            ev.preventDefault();
            onLoadMore();
          }}
        >
          <span className="composer-popover-label">load more…</span>
        </li>
      ) : null}
    </ul>
  );
}

function atErrorText(err: AtBrowseState["error"]): string {
  switch (err) {
    case "cwd-unset":
      return "set a working directory first";
    case "permission":
      return "permission denied for this directory";
    case "fetch-failed":
      return "couldn't reach the bus to list files";
    case "io":
      return "io error listing this directory";
    default:
      return "";
  }
}
