/**
 * The Sessions rail of the claude page: every Claude Code session this worker
 * can see (`claude::agents::list`), what each is doing, and the one action
 * that applies to it. A row opens its session in the terminal beside it.
 *
 * Three groups: the sessions open in this console, which a click switches
 * between; the others on the host, mostly watched (only a live background one
 * can be stopped from here); and the worker's own headless runs, which take a
 * message or a stop. The runs come last because they are never pruned, and a
 * session waiting on the operator must not sit below them. Whatever waits
 * comes first in each group.
 *
 * Live through the engine: a turn starting or settling writes
 * `claude_sessions` / `claude_turns`, and a hooked terminal reports its
 * prompts and stops. A session no hook reaches is re-read when the window
 * gains focus and on the refresh button.
 */
import {
  Button,
  EmptyState,
  IconButton,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  type LiveAnnouncement,
  LiveRegion,
  ShadcnButton,
  ShadcnTextarea,
  Skeleton,
  StatusBar,
  StatusDot,
  StatusPanel,
  useConfirm,
} from '@iii-dev/console-ui';
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format';
import { useWorkerLive } from '@iii-dev/console-ui/hooks';
import type { AgentTerminalApi } from '@iii-workers/agent-terminal-ui';
import { MessageSquare, RefreshCw, Square, SquareTerminal } from 'lucide-react';
import { type FormEvent, type KeyboardEvent, useEffect, useId, useRef, useState } from 'react';
import { SessionDetail, STATUS, type Status } from './session-detail';

/** The console injects an untyped host object; this is the boundary. */
// biome-ignore lint/suspicious/noExplicitAny: the injected host is untyped
type Host = any;

interface AgentRow {
  id: string;
  kind: 'headless' | 'interactive' | 'background';
  status: Status;
  alive: boolean;
  claude_session_id?: string;
  short_id?: string;
  pty_session_id?: string;
  name?: string;
  cwd?: string;
  waiting_for?: string;
  started_at_ms?: number;
  updated_at_ms?: number;
}

interface AgentList {
  agents: AgentRow[];
  detail?: string;
}

const LIST_TIMEOUT_MS = 20_000;
// Describe prepares the session in its folder, installing the iii plugin there
// when it is missing (a marketplace clone): longer than a listing.
const DESCRIBE_TIMEOUT_MS = 180_000;
const ACTION_TIMEOUT_MS = 30_000;
/** Above what claude::agents::stop may take: its listing plus `claude stop`. */
const HOST_STOP_TIMEOUT_MS = 50_000;

const SOURCES = [
  { function_id: 'claude::on-config-change' },
  {
    function_id: 'state::on-changed',
    filters: { 'input.scope': { $in: ['claude_sessions', 'claude_turns'] } },
  },
  // Every hook but the tool calls, which are many. The stops too: a headless
  // run open in a terminal settles nothing in claude_turns.
  {
    function_id: 'claude::terminal::activity',
    filters: {
      'input.hook_event_name': {
        $in: ['SessionStart', 'UserPromptSubmit', 'Stop', 'StopFailure', 'SessionEnd'],
      },
    },
  },
];

/** The last two path segments: enough to tell checkouts apart in a 300px rail. */
function shortPath(cwd: string): string {
  const parts = cwd.split('/').filter(Boolean);
  return parts.length <= 2 ? cwd : `…/${parts.slice(-2).join('/')}`;
}

/** What the operator calls a session: its name, else its id. */
function nameOf(row: AgentRow): string {
  return row.name ?? row.short_id ?? row.id.slice(0, 8);
}

function backgroundAttachmentKey(row: AgentRow): string | undefined {
  if (row.kind === 'background' && row.claude_session_id)
    return `original:background:${row.claude_session_id}`;
}

function originalResumeKey(row: AgentRow): string | undefined {
  if (row.kind !== 'background' && !row.alive && row.claude_session_id)
    return `original:resume:${row.claude_session_id}`;
}

function describe(row: AgentRow): string {
  const parts = [
    row.status === 'waiting' && row.waiting_for
      ? `Waiting for ${row.waiting_for}`
      : STATUS[row.status].word,
  ];
  // The group already says "this worker" for a headless row; on the host the
  // kind is what separates a terminal someone drives from a background agent.
  if (row.kind === 'background') parts.push('Background');
  if (row.kind === 'interactive') parts.push('Interactive');
  const when = row.started_at_ms ?? row.updated_at_ms;
  if (when) parts.push(formatRelative(when));
  if (row.cwd) parts.push(shortPath(row.cwd));
  return parts.join(' · ');
}

/** What needs the operator first: a session waiting on them, then one working. */
function byAttention(rows: AgentRow[]): AgentRow[] {
  const rank = (row: AgentRow) => (row.status === 'waiting' ? 0 : row.status === 'busy' ? 1 : 2);
  return rows.sort((a, b) => rank(a) - rank(b));
}

/**
 * One listing in flight, and one more after it, shared by everyone who asks
 * meanwhile: a turn writes state several times in a row, and each listing runs
 * `claude agents --json` on the terminal host.
 */
const listing: { current: Promise<AgentList> | null; next: Promise<AgentList> | null } = {
  current: null,
  next: null,
};

function listAgents(host: Host): Promise<AgentList> {
  const run = (): Promise<AgentList> => {
    const request: Promise<AgentList> = host.iii
      .call('claude::agents::list', {}, { timeoutMs: LIST_TIMEOUT_MS })
      .finally(() => {
        listing.current = null;
      });
    listing.current = request;
    return request;
  };
  if (!listing.current) return run();
  listing.next ??= listing.current
    .catch(() => undefined)
    .then(() => {
      listing.next = null;
      return run();
    });
  return listing.next;
}

export function SessionsRail({ host, terminal }: { host: Host; terminal: AgentTerminalApi }) {
  const sessions = useWorkerLive<AgentList>({
    iii: host.iii,
    sources: SOURCES,
    fetch: () => listAgents(host),
    handlerId: 'iii::claude-ui::sessions',
  });
  const { confirm, dialog } = useConfirm();
  const [composing, setComposing] = useState<string | null>(null);
  const [stopping, setStopping] = useState<string | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null);
  const seq = useRef(0);
  // Where focus returns when the composer closes: the Message button that opened it.
  const opener = useRef<HTMLElement | null>(null);
  const groupId = useId();
  const [follow, setFollow] = useState<string | null>(null);
  const followOpener = useRef<HTMLButtonElement>(null);
  const announce = (text: string) =>
    setAnnouncement({ seq: ++seq.current, text, urgency: 'polite' });

  const { refresh } = sessions;
  useEffect(() => {
    // A session no hook reaches changes without any write this page can see.
    const onFocus = () => {
      if (!document.hidden) refresh();
    };
    window.addEventListener('focus', onFocus);
    document.addEventListener('visibilitychange', onFocus);
    return () => {
      window.removeEventListener('focus', onFocus);
      document.removeEventListener('visibilitychange', onFocus);
    };
  }, [refresh]);

  // An exit is listed only now: the end hook runs while the process still does.
  useEffect(() => {
    refresh();
  }, [terminal.current, refresh]);

  // Held until the list shows the stop: the row stays alive until then, and a
  // second click would read as a failure.
  useEffect(() => {
    setStopping(null);
  }, [sessions.data]);

  const rows = sessions.data?.agents ?? [];
  const here = byAttention(rows.filter((row) => row.pty_session_id));
  const hosted = byAttention(rows.filter((row) => !row.pty_session_id && row.kind !== 'headless'));
  const own = byAttention(rows.filter((row) => !row.pty_session_id && row.kind === 'headless'));
  const onScreen = (
    rows.find((row) => row.pty_session_id && row.pty_session_id === terminal.current) ??
    rows.find(
      (row) => (backgroundAttachmentKey(row) ?? originalResumeKey(row)) === terminal.currentKey,
    )
  )?.id;
  const followed = rows.find((row) => row.claude_session_id === follow && row.kind !== 'headless');

  const closeComposer = () => {
    setComposing(null);
    opener.current?.focus();
  };

  const openRow = async (row: AgentRow, mode: 'original' | 'fork') => {
    if (opening) return;
    const name = nameOf(row);
    setFailure(null);
    setOpening(row.id);
    try {
      if (
        mode === 'fork' &&
        !(await confirm({
          title: `Fork ${name}?`,
          description:
            'Start a separate conversation with the history so far. New messages in the fork won’t change the original.',
          confirmLabel: 'Create fork',
        }))
      )
        return;
      const resumeKey = mode === 'original' ? originalResumeKey(row) : undefined;
      const reopenHeldResume = resumeKey && terminal.has(resumeKey);
      const spec = await host.iii.call(
        'claude::terminal::describe',
        {
          session: row.id,
          mode,
          claude_session_id: row.claude_session_id,
          ...(reopenHeldResume ? { reopen_held_resume: true } : {}),
        },
        { timeoutMs: DESCRIBE_TIMEOUT_MS },
      );
      if (mode === 'original' && spec.follow && spec.claude_session_id === row.claude_session_id) {
        setFollow(spec.claude_session_id);
        return;
      }
      const newKey = () => `${mode}:${crypto.randomUUID()}`;
      let opened: boolean;
      if (spec.reopen_held_resume && resumeKey) {
        // Do not launch another process if the lease is gone or was taken.
        opened = await terminal.open({ key: resumeKey });
      } else if (mode === 'original' && spec.pty_session_id) {
        const take = terminal.open({ key: newKey(), pty: spec.pty_session_id });
        if (row.kind === 'interactive' && row.claude_session_id) {
          // A listed PTY may belong to another browser. Follow the same UUID
          // through the gated lookup instead of trying a different writer.
          opened = await take.catch(() => false);
          if (!opened) {
            setFollow(row.claude_session_id);
            return;
          }
        } else {
          opened = await take;
        }
      } else {
        if (
          !spec.program ||
          !Array.isArray(spec.args) ||
          Boolean(spec.copy) !== (mode === 'fork') ||
          (mode === 'original' && row.alive && spec.args[0] !== 'attach')
        ) {
          throw new Error(
            'The original session changed or has no available terminal. Refresh the list and try again.',
          );
        }
        // The native attach runs in a PTY whose PID can differ from the one in
        // inventory. Reuse its lease after a fresh describe authorizes the attach.
        // A fork or a resume must never claim this attachment key.
        const attachmentKey =
          mode === 'original' && spec.args[0] === 'attach'
            ? backgroundAttachmentKey(row)
            : undefined;
        opened = await terminal.open({ key: attachmentKey ?? resumeKey ?? newKey(), spec });
      }
      if (!opened)
        throw new Error('This terminal is no longer available. Refresh the list and try again.');
      setFollow(null);
      setComposing(null);
      announce(mode === 'fork' ? `Opened a fork of ${name}` : `Opened ${name}`);
    } catch (err) {
      setFailure(errorMessage(err));
    } finally {
      setOpening(null);
      // After a refusal too: the row was probably out of date.
      refresh();
    }
  };

  const stop = async (row: AgentRow) => {
    const background = row.kind === 'background';
    const ok = await confirm({
      title: `Stop ${nameOf(row)}?`,
      description: background
        ? `Its conversation is kept; resume it later with claude attach ${row.short_id}.`
        : 'The turn ends now; the conversation is kept and takes the next message.',
      confirmLabel: 'Stop',
      tone: 'danger',
    });
    if (!ok) return;
    setFailure(null);
    setStopping(row.id);
    try {
      const res = background
        ? await host.iii.call(
            'claude::agents::stop',
            { id: row.id },
            { timeoutMs: HOST_STOP_TIMEOUT_MS },
          )
        : await host.iii.call(
            'claude::stop',
            { session_id: row.id },
            { timeoutMs: ACTION_TIMEOUT_MS },
          );
      if (res?.stopped === false) {
        setFailure(res.reason ?? 'The session was not running.');
        setStopping(null);
      } else {
        announce(`Stopped ${nameOf(row)}`);
      }
    } catch (err) {
      setFailure(errorMessage(err));
      setStopping(null);
    }
    refresh();
  };

  const send = async (row: AgentRow, prompt: string) => {
    setFailure(null);
    try {
      const res = await host.iii.call(
        'claude::start',
        { session_id: row.id, prompt },
        { timeoutMs: ACTION_TIMEOUT_MS },
      );
      if (res?.started === false) {
        setFailure(res.reason ?? 'The session is busy; send again when it settles.');
        return false;
      }
      announce(`Message sent to ${nameOf(row)}`);
      // Another row may have opened its own composer while this one was sending.
      setComposing((open) => (open === row.id ? null : open));
      refresh();
      return true;
    } catch (err) {
      setFailure(errorMessage(err));
      return false;
    }
  };

  const renderRow = (row: AgentRow) => {
    const status = STATUS[row.status];
    const isStopping = stopping === row.id;
    // Open in the terminal, or about to be: a message would put a second writer on its transcript.
    const inTerminal = row.id === onScreen || row.id === opening;
    const canMessage =
      row.kind === 'headless' && !row.alive && row.status !== 'unknown' && !inTerminal;
    // A background session is stoppable where the list gives it a short id.
    const canStop =
      row.alive && (row.kind === 'headless' || (row.kind === 'background' && !!row.short_id));
    const name = nameOf(row);
    const openLabel = `Open original: ${name}`;
    const canFork = !!row.claude_session_id;
    return (
      // biome-ignore lint/a11y/useSemanticElements: ListGroup is the console's div, so its rows are too
      <div key={`${row.kind}:${row.id}`} role="listitem">
        <ListItem
          as="div"
          selected={row.id === onScreen}
          // The button says it with aria-current; a plain div may not be aria-selected.
          aria-selected={undefined}
          leading={<StatusDot tone={status.tone} pulse={status.pulse} aria-hidden />}
          label={
            <button
              type="button"
              className="claude-sessions-open"
              aria-label={openLabel}
              aria-current={row.id === onScreen ? 'true' : undefined}
              title={[openLabel, row.cwd, row.id].filter(Boolean).join('\n')}
              aria-disabled={!!opening}
              onClick={(event) => {
                if (!opening) {
                  followOpener.current = event.currentTarget;
                  void openRow(row, 'original');
                }
              }}
            >
              {row.name ? row.name : <span className="claude-sessions-id">{name}</span>}
            </button>
          }
          description={opening === row.id ? 'Opening…' : isStopping ? 'Stopping…' : describe(row)}
          trailing={
            canMessage || canStop || canFork ? (
              <span className="claude-sessions-actions">
                {canFork ? (
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    aria-label={`Fork ${name}`}
                    aria-disabled={!!opening}
                    onClick={() => void openRow(row, 'fork')}
                  >
                    Fork
                  </Button>
                ) : null}
                {canMessage ? (
                  <IconButton
                    variant="ghost"
                    label={`Message ${name}`}
                    aria-expanded={composing === row.id}
                    onClick={(event) => {
                      opener.current = event.currentTarget;
                      setComposing((open) => (open === row.id ? null : row.id));
                    }}
                  >
                    <MessageSquare aria-hidden />
                  </IconButton>
                ) : null}
                {canStop ? (
                  <IconButton
                    variant="ghost"
                    label={`Stop ${name}`}
                    disabled={isStopping}
                    onClick={() => void stop(row)}
                  >
                    <Square aria-hidden />
                  </IconButton>
                ) : null}
              </span>
            ) : undefined
          }
        />
        {composing === row.id && !inTerminal ? (
          <Composer name={name} onSend={(prompt) => send(row, prompt)} onClose={closeComposer} />
        ) : null}
      </div>
    );
  };

  const group = (key: string, text: string, items: AgentRow[]) =>
    items.length > 0 ? (
      <ListGroup role="list" aria-labelledby={`${groupId}-${key}`}>
        <ListGroupLabel id={`${groupId}-${key}`}>
          {text} <span className="claude-sessions-count">{items.length}</span>
        </ListGroupLabel>
        {items.map(renderRow)}
      </ListGroup>
    ) : null;

  let body: React.ReactNode;
  if (sessions.error && !sessions.data) {
    body = (
      <StatusPanel
        role="alert"
        variant="alert"
        headline="Could not list sessions"
        detail={sessions.error}
        action={
          <Button size="sm" variant="ghost" onClick={refresh}>
            Retry
          </Button>
        }
      />
    );
  } else if (!sessions.data) {
    body = (
      <div
        className="claude-sessions-skeleton"
        role="status"
        aria-busy="true"
        aria-label="Loading sessions"
      >
        {[0, 1, 2].map((n) => (
          <div key={n} className="claude-sessions-skeleton-row">
            <Skeleton className="claude-sessions-skeleton-dot" />
            <span className="claude-sessions-skeleton-lines">
              <Skeleton className="claude-sessions-skeleton-line" />
              <Skeleton className="claude-sessions-skeleton-line claude-sessions-skeleton-line-short" />
            </span>
          </div>
        ))}
      </div>
    );
  } else if (rows.length === 0) {
    body = (
      <EmptyState
        compact
        icon={SquareTerminal}
        title="No sessions listed"
        description="Turns started with claude::run, and Claude Code sessions opened in this terminal, appear here."
      />
    );
  } else {
    // Named by the rail's own landmark (its "Sessions" label).
    body = (
      <List>
        {group('here', 'In this console', here)}
        {group('host', 'On this host', hosted)}
        {group('own', 'This worker', own)}
      </List>
    );
  }

  const waiting = rows.filter((row) => row.status === 'waiting').length;
  const working = rows.filter((row) => row.status === 'busy').length;
  const summary = sessions.data
    ? [
        `${rows.length} ${rows.length === 1 ? 'session' : 'sessions'}`,
        waiting ? `${waiting} waiting` : '',
        working ? `${working} working` : '',
        sessions.error ? 'refresh failed' : '',
      ]
        .filter(Boolean)
        .join(' · ')
    : sessions.error && !sessions.loading
      ? 'Sessions unavailable'
      : 'Reading sessions…';

  return (
    <div className="claude-sessions">
      {dialog}
      {follow ? (
        <SessionDetail
          key={follow}
          iii={host.iii}
          id={follow}
          available={!!followed && !sessions.error}
          opener={followOpener}
          onClose={() => setFollow(null)}
        />
      ) : null}
      <LiveRegion announcement={announcement} />
      {failure || (sessions.error && sessions.data) ? (
        <div className="claude-sessions-notices">
          {failure ? (
            <StatusPanel
              role="alert"
              variant="alert"
              headline="That did not go through"
              detail={failure}
              action={
                <Button size="sm" variant="ghost" onClick={() => setFailure(null)}>
                  Dismiss
                </Button>
              }
            />
          ) : null}
          {sessions.error && sessions.data ? (
            <StatusPanel
              role="alert"
              variant="warn"
              headline="Sessions may be out of date"
              detail={sessions.error}
              action={
                <Button size="sm" variant="ghost" onClick={refresh}>
                  Retry
                </Button>
              }
            />
          ) : null}
        </div>
      ) : null}
      <div className="claude-sessions-scroll">
        {body}
        {sessions.data?.detail ? (
          <p className="claude-sessions-note">{sessions.data.detail}</p>
        ) : null}
      </div>
      <StatusBar
        aria-label="Sessions status"
        end={
          <ShadcnButton
            variant="ghost"
            size="icon-sm"
            aria-label="Refresh sessions"
            title="Refresh sessions"
            onClick={refresh}
          >
            <RefreshCw aria-hidden />
          </ShadcnButton>
        }
      >
        {summary}
      </StatusBar>
    </div>
  );
}

/** One message to a headless session: ⌘/Ctrl + Enter sends, Escape closes. */
function Composer({
  name,
  onSend,
  onClose,
}: {
  name: string;
  onSend: (prompt: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const [prompt, setPrompt] = useState('');
  const [sending, setSending] = useState(false);
  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    const text = prompt.trim();
    if (!text || sending) return;
    setSending(true);
    // A sent message unmounts this; only a refusal leaves it to try again.
    if (!(await onSend(text))) setSending(false);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Escape') {
      // The rail is inside a drawer on narrow panes, which Escape would close too.
      event.preventDefault();
      event.stopPropagation();
      onClose();
    } else if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void submit();
    }
  };
  return (
    <form className="claude-sessions-composer" onSubmit={submit}>
      <ShadcnTextarea
        aria-label={`Message ${name}`}
        placeholder="Continue this session…"
        value={prompt}
        onChange={(event) => setPrompt(event.target.value)}
        onKeyDown={onKeyDown}
        rows={2}
        autoFocus
        disabled={sending}
      />
      <span className="claude-sessions-composer-actions">
        <Button type="button" size="sm" variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button type="submit" size="sm" disabled={!prompt.trim() || sending}>
          {sending ? 'Sending…' : 'Send'}
        </Button>
      </span>
    </form>
  );
}
