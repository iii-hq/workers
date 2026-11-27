/**
 * The Sessions rail of the claude page: every Claude Code session this worker
 * can see (`claude::agents::list`), what each is doing, and the one action
 * that applies to it. Select an original to follow replies without a terminal.
 *
 * Three groups: the sessions open in this console; the others on the host,
 * mostly watched (only a live background one
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
  ConfirmDialog,
  EmptyState,
  type ExtensionIii,
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
} from '@iii-dev/console-ui';
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format';
import { useWorkerLive } from '@iii-dev/console-ui/hooks';
import type { AgentTerminalApi } from '@iii-workers/agent-terminal-ui';
import { MessageSquare, RefreshCw, Square, SquareTerminal } from 'lucide-react';
import { type FormEvent, type KeyboardEvent, useEffect, useId, useRef, useState } from 'react';
import { SessionDetail, STATUS, type Status, sessionActionNonce } from './session-detail';

/** The console injects an untyped host object; this is the boundary. */
// biome-ignore lint/suspicious/noExplicitAny: the injected host is untyped
type Host = any;

interface AgentRow {
  id: string;
  kind: 'headless' | 'interactive' | 'background';
  status: Status;
  alive: boolean;
  external_only?: boolean;
  claude_session_id?: string;
  short_id?: string;
  pty_session_id?: string;
  name?: string;
  cwd?: string;
  waiting_for?: string;
  started_at_ms?: number;
  updated_at_ms?: number;
}

type SessionAnnouncement = LiveAnnouncement & { externalId?: string };
type SessionFailure = { text: string; externalId?: string };
type SessionConfirmation = { row: AgentRow; action: 'fork' | 'stop'; generation: number };

interface AgentList {
  agents: AgentRow[];
  inventory_status?: 'available' | 'unavailable';
  config_id?: string;
  external_visibility?: string | null;
  detail?: string;
}

const LIST_TIMEOUT_MS = 20_000;
const WATCH_CHECK_MS = 5_000;
// Describe prepares the session in its folder, installing the iii plugin there
// when it is missing (a marketplace clone): longer than a listing.
const DESCRIBE_TIMEOUT_MS = 180_000;
const ACTION_TIMEOUT_MS = 30_000;
/** Above what claude::agents::stop may take: its listing plus `claude stop`. */
const HOST_STOP_TIMEOUT_MS = 50_000;

const SOURCES = [
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
      .call('claude::agents::list', { include_config_id: true }, { timeoutMs: LIST_TIMEOUT_MS })
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
  const iii: ExtensionIii = host.iii;
  const generation = useRef(0);
  const configPending = useRef(false);
  const [visibility, setVisibility] = useState(0);
  const [watch, setWatch] = useState<{
    iii: ExtensionIii;
    configId: string;
    token: string;
  }>();
  const sessions = useWorkerLive<AgentList & { generation: number }>({
    iii,
    sources: SOURCES,
    fetch: async () => {
      const started = configPending.current ? -1 : generation.current;
      return { ...(await listAgents(host)), generation: started };
    },
    handlerId: 'iii::claude-ui::sessions',
  });
  const [confirmation, setConfirmation] = useState<SessionConfirmation | null>(null);
  const refreshButton = useRef<HTMLButtonElement>(null);
  const [composing, setComposing] = useState<string | null>(null);
  const [stopping, setStopping] = useState<string | null>(null);
  const [openingFork, setOpening] = useState<string | null>(null);
  const [failure, setFailure] = useState<SessionFailure | null>(null);
  const [announcement, setAnnouncement] = useState<SessionAnnouncement | null>(null);
  const seq = useRef(0);
  // Where focus returns when the composer closes: the Message button that opened it.
  const opener = useRef<HTMLElement | null>(null);
  const groupId = useId();
  const [follow, setFollow] = useState<string | null>(null);
  const followOpener = useRef<HTMLButtonElement>(null);

  const { refresh } = sessions;
  const configId = sessions.data?.config_id;
  useEffect(() => {
    if (!configId) return;
    const handler = 'iii::claude-ui::inventory-visibility';
    const settled = `${handler}-settled`;
    const target = (id: string) => `${id}::${iii.browserId}`;
    const observerId = sessionActionNonce();
    const off: (() => void)[] = [];
    let connected = true;
    let watched: string | null = null;
    let checking = false;
    let probe = 0;
    const invalidate = () => {
      setVisibility(++generation.current);
      setAnnouncement((current) => (current?.externalId ? null : current));
      setFailure((current) => (current?.externalId ? null : current));
    };
    const acceptWatch = (token: string | null) => {
      const confirmedChange = token !== null && configPending.current;
      if (confirmedChange) configPending.current = false;
      if (watched === token && !confirmedChange) return;
      watched = token;
      invalidate();
      setWatch(token ? { iii, configId, token } : undefined);
      // Never reopen from cached rows: get a list stamped with the new permission.
      if (token) refresh();
    };
    const checkWatch = async () => {
      if (!connected || checking) return;
      checking = true;
      const current = probe;
      const confirmLiveConfig = configPending.current;
      let token: string | null = null;
      try {
        const [watch, visibility] = await Promise.all([
          iii.call<{
            id?: string;
            function_id?: string;
            target?: { function_id?: string };
            state?: string;
          }>('engine::observers::get', { id: observerId }, { timeoutMs: WATCH_CHECK_MS }),
          iii.call<{ external_visibility?: unknown; live_config_confirmed?: unknown }>(
            'claude::agents::visibility',
            confirmLiveConfig ? { confirm_live_config: true } : {},
            { timeoutMs: WATCH_CHECK_MS },
          ),
        ]);
        const snapshot = visibility?.external_visibility;
        if (
          watch?.id === observerId &&
          watch.function_id === 'configuration::on-updated' &&
          watch.target?.function_id === target(handler) &&
          watch.state === 'active' &&
          (!confirmLiveConfig || visibility?.live_config_confirmed === true) &&
          typeof snapshot === 'string' &&
          snapshot
        ) {
          token = snapshot;
        }
      } catch {
        // Neither a missing browser watch nor an unconfirmed worker snapshot can authorize data.
      }
      if (current !== probe) return;
      checking = false;
      acceptWatch(token);
    };
    const change = {
      function_id: 'configuration::on-updated',
      filters: { 'input.id': { $eq: configId } },
      capture: [],
    };
    try {
      off.push(
        iii.on(handler, () => {
          configPending.current = true;
          probe++;
          checking = false;
          invalidate();
          void checkWatch();
        }),
      );
      off.push(
        iii.on(settled, () => {
          // A probe started before the reload cannot settle its result.
          probe++;
          checking = false;
          invalidate();
          refresh();
          void checkWatch();
        }),
      );
      off.push(iii.observe(change, target(handler), { id: observerId }));
      off.push(iii.observe('claude::on-config-change', target(settled)));
      off.push(
        iii.addConnectionStateListener((state) => {
          connected = state === 'connected';
          if (!connected) {
            probe++;
            checking = false;
            acceptWatch(null);
          } else {
            refresh();
            void checkWatch();
          }
        }),
      );
      // ADE returns only a disposer, not the SDK's asynchronous admission state.
      // Check both sides without rerunning the terminal host inventory on every heartbeat.
      const timer = window.setInterval(() => void checkWatch(), WATCH_CHECK_MS);
      off.push(() => window.clearInterval(timer));
      void checkWatch();
    } catch {
      connected = false;
      probe++;
      acceptWatch(null);
    }
    return () => {
      connected = false;
      probe++;
      setWatch(undefined);
      for (const dispose of off.reverse()) dispose();
    };
  }, [iii, configId, refresh]);
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

  const currentVisibility =
    !!configId &&
    watch?.iii === iii &&
    watch.configId === configId &&
    sessions.data?.external_visibility === watch.token &&
    sessions.data?.generation === visibility &&
    !sessions.error;
  const listed = sessions.data?.agents ?? [];
  // What an action that awaits reads to learn whether its row is still shown and authorized.
  const latest = useRef({ visible: false, visibility: 0, listed: [] as AgentRow[] });
  latest.current = { visible: currentVisibility, visibility, listed };
  const noticeVisible = (externalId?: string) =>
    !externalId || (currentVisibility && listed.some((row) => row.id === externalId));
  const liveAnnouncement = noticeVisible(announcement?.externalId) ? announcement : null;
  const liveFailure = noticeVisible(failure?.externalId) ? failure : null;
  const reportFailure = (row: AgentRow, text: string) => {
    if (row.external_only && (!currentVisibility || visibility !== generation.current)) return;
    setFailure({ text, externalId: row.external_only ? row.id : undefined });
  };
  const announce = (row: AgentRow, text: string) => {
    // An action may finish after the visibility that authorized its row is gone.
    if (row.external_only && (!currentVisibility || visibility !== generation.current)) return;
    setAnnouncement({
      seq: ++seq.current,
      text,
      urgency: 'polite',
      externalId: row.external_only ? row.id : undefined,
    });
  };
  const rows = currentVisibility
    ? listed
    : listed.filter((row) => !row.external_only).map((row) => ({ ...row, short_id: undefined }));
  const activeConfirmation =
    confirmation &&
    ((!confirmation.row.external_only &&
      !(confirmation.action === 'stop' && confirmation.row.kind === 'background')) ||
      (currentVisibility &&
        confirmation.generation === visibility &&
        rows.some((row) => row.id === confirmation.row.id)))
      ? confirmation
      : null;
  useEffect(() => {
    if (confirmation && !activeConfirmation) {
      setConfirmation(null);
      refreshButton.current?.focus();
    }
  }, [confirmation, activeConfirmation]);
  const opening =
    openingFork ?? (activeConfirmation?.action === 'fork' ? activeConfirmation.row.id : null);

  const here = byAttention(rows.filter((row) => row.pty_session_id));
  const hosted = byAttention(rows.filter((row) => !row.pty_session_id && row.kind !== 'headless'));
  const own = byAttention(rows.filter((row) => !row.pty_session_id && row.kind === 'headless'));
  const onScreen = rows.find(
    (row) => row.pty_session_id && row.pty_session_id === terminal.current,
  )?.id;

  const closeComposer = () => {
    setComposing(null);
    opener.current?.focus();
  };

  const forkRow = async (row: AgentRow) => {
    if (openingFork || !row.claude_session_id) return;
    const name = nameOf(row);
    setFailure(null);
    setOpening(row.id);
    // An external row forks only while the permission that showed it holds and it is still listed.
    const stillCurrent = () => {
      if (!row.external_only) return true;
      const now = latest.current;
      return (
        now.visible &&
        now.visibility === visibility &&
        visibility === generation.current &&
        now.listed.some(
          (listedRow) =>
            listedRow.id === row.id && listedRow.claude_session_id === row.claude_session_id,
        )
      );
    };
    try {
      const spec = await host.iii.call(
        'claude::terminal::describe',
        { session: row.id, mode: 'fork', claude_session_id: row.claude_session_id },
        { timeoutMs: DESCRIBE_TIMEOUT_MS },
      );
      if (!stillCurrent()) return;
      if (
        spec.claude_session_id !== row.claude_session_id ||
        spec.copy !== true ||
        !spec.program ||
        !Array.isArray(spec.args) ||
        spec.args.at(-3) !== '--resume' ||
        spec.args.at(-2) !== row.claude_session_id ||
        spec.args.at(-1) !== '--fork-session'
      )
        throw new Error('The fork could not be verified. Refresh the list and try again.');
      const key = `fork:${row.claude_session_id}:${sessionActionNonce()}`;
      if (!stillCurrent()) return;
      if (!(await terminal.open({ key, spec })))
        throw new Error('This terminal is no longer available. Refresh the list and try again.');
      setFollow(null);
      // Another row's open draft is not this fork's to close.
      setComposing((open) => (open === row.id ? null : open));
      announce(row, `Opened a fork of ${name}`);
    } catch (err) {
      reportFailure(row, errorMessage(err));
    } finally {
      setOpening(null);
      // After a refusal too: the row was probably out of date.
      refresh();
    }
  };

  const stop = async (row: AgentRow) => {
    const background = row.kind === 'background';
    // Even a worker-known background row uses a permission-gated host action.
    const noticeRow = background ? { ...row, external_only: true } : row;
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
        reportFailure(
          noticeRow,
          background
            ? 'Could not stop the host session. Refresh and try again.'
            : (res.reason ?? 'The session was not running.'),
        );
        setStopping(null);
      } else {
        announce(noticeRow, `Stopped ${nameOf(row)}`);
      }
    } catch (err) {
      reportFailure(
        noticeRow,
        background ? 'Could not stop the host session. Refresh and try again.' : errorMessage(err),
      );
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
        reportFailure(row, res.reason ?? 'The session is busy; send again when it settles.');
        return false;
      }
      announce(row, `Message sent to ${nameOf(row)}`);
      // Another row may have opened its own composer while this one was sending.
      setComposing((open) => (open === row.id ? null : open));
      refresh();
      return true;
    } catch (err) {
      reportFailure(row, errorMessage(err));
      return false;
    }
  };

  const renderRow = (row: AgentRow) => {
    const status = STATUS[row.status];
    const isStopping = stopping === row.id;
    // A message to the same transcript would put a second writer on it.
    const inTerminal = row.id === onScreen;
    // A fork being confirmed or opened only holds the row's actions; its draft stays.
    const isOpening = row.id === opening;
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
            // A headless run has no conversation to follow: its name is just text.
            row.kind === 'headless' ? (
              <span title={[name, row.cwd, row.id].filter(Boolean).join('\n')}>
                {row.name ? row.name : <span className="claude-sessions-id">{name}</span>}
              </span>
            ) : (
              <button
                type="button"
                className="claude-sessions-open"
                aria-label={openLabel}
                aria-current={row.id === onScreen ? 'true' : undefined}
                title={[openLabel, row.cwd, row.id].filter(Boolean).join('\n')}
                aria-disabled={!!opening}
                onClick={(event) => {
                  if (opening) return;
                  setFailure(null);
                  if (!row.claude_session_id) {
                    reportFailure(row, 'The original conversation is not available yet.');
                    return;
                  }
                  followOpener.current = event.currentTarget;
                  setFollow(row.claude_session_id);
                }}
              >
                {row.name ? row.name : <span className="claude-sessions-id">{name}</span>}
              </button>
            )
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
                    onClick={() => {
                      if (!opening)
                        setConfirmation({ row, action: 'fork', generation: visibility });
                    }}
                  >
                    Fork
                  </Button>
                ) : null}
                {canMessage ? (
                  <IconButton
                    variant="ghost"
                    label={`Message ${name}`}
                    aria-expanded={composing === row.id}
                    aria-disabled={isOpening}
                    onClick={(event) => {
                      if (isOpening) return;
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
                    onClick={() => setConfirmation({ row, action: 'stop', generation: visibility })}
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

  const hostUnavailable = sessions.data?.inventory_status === 'unavailable';
  let body: React.ReactNode;
  if (sessions.error && !sessions.data) {
    body = (
      <StatusPanel
        role="alert"
        variant="alert"
        headline="Could not list sessions"
        detail="Refresh to try again."
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
  } else if (rows.length === 0 && hostUnavailable) {
    body = null; // The inventory warning below replaces the successful empty state.
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

  const target = activeConfirmation?.row;
  const isFork = activeConfirmation?.action === 'fork';
  const confirmationDescription = !target
    ? undefined
    : isFork
      ? 'Start a separate conversation with the history so far. New messages in the fork won’t change the original.'
      : target.kind === 'background'
        ? `Its conversation is kept; resume it later with claude attach ${target.short_id}.`
        : 'The turn ends now; the conversation is kept and takes the next message.';
  return (
    <div className="claude-sessions">
      <ConfirmDialog
        open={!!target}
        onOpenChange={(open) => {
          if (!open) setConfirmation(null);
        }}
        title={target ? `${isFork ? 'Fork' : 'Stop'} ${nameOf(target)}?` : ''}
        description={confirmationDescription}
        confirmLabel={target ? (isFork ? 'Create fork' : 'Stop') : undefined}
        tone={target && !isFork ? 'danger' : 'default'}
        onConfirm={() => {
          if (!target || (target.external_only && visibility !== generation.current)) return;
          if (isFork) void forkRow(target);
          else void stop(target);
        }}
      />
      {follow ? (
        <SessionDetail
          key={follow}
          iii={host.iii}
          id={follow}
          configId={configId}
          // The gated reply snapshot can still find originals that finish and leave the rail.
          available={currentVisibility}
          opener={{
            get current() {
              return followOpener.current?.isConnected
                ? followOpener.current
                : refreshButton.current;
            },
          }}
          onClose={() => setFollow(null)}
        />
      ) : null}
      <LiveRegion announcement={liveAnnouncement} />
      {liveFailure || (sessions.error && sessions.data) ? (
        <div className="claude-sessions-notices">
          {liveFailure ? (
            <StatusPanel
              role="alert"
              variant="alert"
              headline="That did not go through"
              detail={liveFailure.text}
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
              detail="Refresh to try again."
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
        {hostUnavailable ? (
          <StatusPanel
            role="status"
            variant="warn"
            headline="Host sessions unavailable"
            detail="Refresh to try reading the terminal host again."
          />
        ) : currentVisibility && sessions.data?.detail ? (
          <p className="claude-sessions-note">{sessions.data.detail}</p>
        ) : null}
      </div>
      <StatusBar
        aria-label="Sessions status"
        end={
          <ShadcnButton
            variant="ghost"
            size="icon-sm"
            ref={refreshButton}
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
