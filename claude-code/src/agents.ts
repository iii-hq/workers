/**
 * `claude::agents::list`: the Claude Code sessions this worker can see, in one
 * list. Its own headless runs come from `claude_sessions`; interactive and
 * background sessions on the terminal host come from `claude agents --json`
 * there, a CLI output with no versioned contract, so every field of it is
 * read as untrusted.
 */

import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import { isLive, jsonSchema } from './run.js';
import { listSessions } from './state.js';
import { exec, quote } from './terminal/host.js';
import type { Prepared } from './terminal/workspace.js';
import { TURN_SCOPE } from './turns.js';
import type { SessionRecord } from './types.js';

/** One row of `claude agents --json`, as the CLI 2.1.331 prints it. */
export type CliRow = {
  pid?: unknown;
  id?: unknown;
  cwd?: unknown;
  kind?: unknown;
  startedAt?: unknown;
  sessionId?: unknown;
  name?: unknown;
  status?: unknown;
  state?: unknown;
  waitingFor?: unknown;
};

const AgentRowSchema = z.object({
  id: z
    .string()
    .describe('iii session_id for a headless run; the Claude Code session id otherwise'),
  kind: z.enum(['headless', 'interactive', 'background']),
  status: z.enum(['busy', 'idle', 'waiting', 'done', 'error', 'stopped', 'unknown']),
  alive: z.boolean().describe('A process is running the session now'),
  session_id: z.string().optional(),
  claude_session_id: z.string().optional(),
  short_id: z
    .string()
    .optional()
    .describe('The CLI short id of a background session; only with external_sessions on'),
  pty_session_id: z
    .string()
    .optional()
    .describe('The console terminal (shell::pty) that runs this session'),
  name: z.string().optional(),
  cwd: z.string().optional(),
  waiting_for: z.string().optional(),
  started_at_ms: z.number().optional(),
  updated_at_ms: z.number().optional().describe('When a headless run last changed'),
});
export type AgentRow = z.infer<typeof AgentRowSchema>;

/** How a terminal opens a listed session; `copy` when it opens a fork of it. */
export type OpenSpec = { args: string[]; cwd: string; copy?: true };
export type OpenMode = 'original' | 'fork';
export type OpenRequest = { mode: OpenMode; claude_session_id: string };
export type OpenPlan =
  | OpenSpec
  | { pty_session_id: string }
  | { follow: true; claude_session_id: string };
export type ResolveOpen = (
  id: string,
  request?: OpenRequest,
  reserve?: boolean,
) => Promise<OpenPlan>;

const ListPayloadSchema = z.object({
  all: z
    .boolean()
    .optional()
    .describe('Also list completed background sessions (`claude agents --json --all`)'),
});

const StopPayloadSchema = z.object({
  id: z
    .string()
    .min(1)
    .max(200)
    .describe('The Claude Code session id, as claude::agents::list gives it'),
});

/** The CLI's 8-hex background id; anything else never reaches the shell. */
const SHORT_ID = /^[0-9a-f]{8}$/;

/**
 * How long a conversation counts as open after a plain `--resume` was handed
 * out for it: the CLI lists the session only once its process has started.
 * shortcut: time-based, a slow start or a trust prompt outlasts it; upgrade if
 * shell::pty::sessions ever reports args.
 */
const OPENING_MS = 15_000;

export function parseAgentsJson(stdout: string): CliRow[] | null {
  try {
    const parsed: unknown = JSON.parse(stdout);
    if (!Array.isArray(parsed)) return null;
    return parsed.filter((row): row is CliRow => typeof row === 'object' && row !== null);
  } catch {
    return null;
  }
}

const str = (value: unknown) => (typeof value === 'string' ? value : undefined);

function cliStatus(row: CliRow): AgentRow['status'] {
  if (row.state === 'done') return 'done';
  if (row.state === 'failed') return 'error';
  if (row.state === 'stopped') return 'stopped';
  if (row.state === 'blocked' || row.status === 'waiting') return 'waiting';
  if (row.status === 'busy' || row.status === 'shell') return 'busy';
  if (row.status === 'idle') return 'idle';
  if (row.state === 'working') return 'busy';
  return 'unknown';
}

function headlessStatus(record: SessionRecord, alive: boolean): AgentRow['status'] {
  if (alive) return 'busy';
  if (record.status === 'error') return 'error';
  // `working` with no run here: a lost final status, not a turn in progress.
  if (record.status === 'working') return 'unknown';
  if (record.stop_reason === 'interrupted') return 'stopped';
  return 'done';
}

export function mergeInventory(input: {
  records: SessionRecord[];
  isLive: (sessionId: string) => boolean;
  cliRows: CliRow[];
  known: (claudeSessionId: string) => boolean;
  /** The console terminal whose process is `pid`. */
  ptyOf: (pid: number) => string | undefined;
  external: boolean;
}): { agents: AgentRow[]; hidden: number } {
  const ours = new Set(input.records.map((r) => r.claude_session_id).filter(Boolean));
  const runningHere = new Set(
    input.records.filter((r) => input.isLive(r.session_id)).map((r) => r.claude_session_id),
  );
  // Our sessions that a process this worker does not run has open. A message
  // would put a second writer on the transcript, so the CLI row stands for them.
  const inTerminal = new Set<string | null>();
  for (const { pid, sessionId } of input.cliRows) {
    const id = str(sessionId);
    if (typeof pid === 'number' && id && ours.has(id) && !runningHere.has(id)) inTerminal.add(id);
  }
  const records = input.records.filter((r) => !inTerminal.has(r.claude_session_id));
  const headless: AgentRow[] = records.map((record) => {
    const alive = input.isLive(record.session_id);
    return {
      id: record.session_id,
      kind: 'headless',
      status: headlessStatus(record, alive),
      alive,
      session_id: record.session_id,
      ...(record.claude_session_id ? { claude_session_id: record.claude_session_id } : {}),
      ...(record.cwd ? { cwd: record.cwd } : {}),
      updated_at_ms: record.updated_at_ms,
    };
  });
  // claude_sessions is never pruned: newest first, so a recent run is on screen.
  headless.sort((a, b) => (b.updated_at_ms ?? 0) - (a.updated_at_ms ?? 0));

  const cli = new Map<string, AgentRow>();
  for (const row of input.cliRows) {
    const sessionId = str(row.sessionId);
    // A headless run shows up in the CLI listing too; its headless row stands for it.
    if (!sessionId || (ours.has(sessionId) && !inTerminal.has(sessionId))) continue;
    const kind = row.kind === 'background' ? 'background' : 'interactive';
    const status = cliStatus(row);
    const waitingFor = status === 'waiting' ? str(row.waitingFor) : undefined;
    const pty = typeof row.pid === 'number' ? input.ptyOf(row.pid) : undefined;
    const agent: AgentRow = {
      id: sessionId,
      kind,
      status,
      alive: typeof row.pid === 'number',
      claude_session_id: sessionId,
      // Only where claude::agents::stop would act on it.
      ...(input.external && kind === 'background' && str(row.id) ? { short_id: str(row.id) } : {}),
      ...(pty ? { pty_session_id: pty } : {}),
      ...(str(row.name) ? { name: str(row.name) } : {}),
      ...(str(row.cwd) ? { cwd: str(row.cwd) } : {}),
      ...(waitingFor ? { waiting_for: waitingFor } : {}),
      ...(typeof row.startedAt === 'number' ? { started_at_ms: row.startedAt } : {}),
    };
    const seen = cli.get(sessionId);
    if (!seen || (!seen.alive && agent.alive)) cli.set(sessionId, agent);
  }

  let hidden = 0;
  const visible = [...cli.values()].filter((agent) => {
    // Neither a run of ours, open in a terminal, nor a console terminal is an external session.
    if (input.external || ours.has(agent.id) || agent.pty_session_id || input.known(agent.id)) {
      return true;
    }
    hidden++;
    return false;
  });
  return { agents: [...headless, ...visible], hidden };
}

/** The terminal host's `claude agents --json`, or why there is none. */
async function cliSnapshot(
  iii: IIIClient,
  prepared: { executable: string; env: Record<string, string>; detail: string },
  all: boolean,
): Promise<{ rows: CliRow[]; detail?: string }> {
  if (!prepared.executable) {
    return { rows: [], detail: prepared.detail || 'claude is not available on the terminal host' };
  }
  try {
    const result = await exec(
      iii,
      `${quote(prepared.executable)} agents --json${all ? ' --all' : ''}`,
      { env: prepared.env, timeoutMs: 15_000 },
    );
    const rows = result.exit_code === 0 ? parseAgentsJson(result.stdout) : null;
    if (rows) return { rows };
    const why = result.exit_code === 0 ? 'output is not a JSON array' : `exit ${result.exit_code}`;
    const stderr = result.stderr.trim().slice(0, 200);
    return {
      rows: [],
      detail: `claude agents --json failed: ${why}${stderr ? `: ${stderr}` : ''}`,
    };
  } catch (err) {
    return { rows: [], detail: `claude agents --json failed: ${String(err)}` };
  }
}

/** Claude session ids whose hooks reached this worker: they wrote a hook settle. */
async function hookSessions(iii: IIIClient): Promise<Set<string>> {
  try {
    const settles = await iii.call<unknown, Array<{ id?: unknown; source?: unknown }> | null>({
      function_id: 'state::list',
      payload: { scope: TURN_SCOPE },
      timeoutMs: 15_000,
    });
    const ids = (Array.isArray(settles) ? settles : [])
      .filter((s) => s?.source === 'hook')
      .map((s) => str(s.id));
    return new Set(ids.filter((id): id is string => !!id));
  } catch (err) {
    console.warn(`claude-code: could not list hook settles: ${String(err)}`);
    return new Set();
  }
}

/** The console's claude terminals: the pid of each one's process, to its PTY session. */
async function claudePtys(iii: IIIClient): Promise<Map<number, string>> {
  const byPid = new Map<number, string>();
  try {
    const res = await iii.call<
      unknown,
      { sessions?: Array<{ session_id?: unknown; pid?: unknown; ui?: unknown }> } | null
    >({ function_id: 'shell::pty::sessions', payload: {}, timeoutMs: 15_000 });
    for (const s of Array.isArray(res?.sessions) ? res.sessions : []) {
      const id = str(s?.session_id);
      if (s?.ui === 'claude' && typeof s.pid === 'number' && id) byPid.set(s.pid, id);
    }
  } catch (err) {
    console.warn(`claude-code: could not list the console terminals: ${String(err)}`);
  }
  return byPid;
}

export function registerAgents(
  iii: IIIClient,
  current: () => Prepared,
  external: () => boolean,
  tracker: { has(sessionId: string): boolean },
  visibilityEpoch: () => number | null = () => (external() ? 0 : null),
): {
  replyRow: (id: string) => Promise<{ row: AgentRow; env: Prepared['env'] }>;
  /** How a terminal opens a listed session; throws why one cannot be. */
  resolveOpen: ResolveOpen;
  /** Whether a process on the terminal host has a conversation open, or is about to. */
  heldElsewhere: (claudeSessionId: string) => Promise<boolean>;
} {
  const opening = new Map<string, number>();
  const isOpening = (claudeId: string) => (opening.get(claudeId) ?? 0) > Date.now();

  const inventory = async (
    all: boolean,
    host = current(),
    replies = false,
    showAll = replies || external(),
  ) => {
    const [records, snapshot, hooked, ptys] = await Promise.all([
      listSessions(iii),
      cliSnapshot(iii, host, all),
      showAll ? new Set<string>() : hookSessions(iii),
      replies ? new Map<number, string>() : claudePtys(iii),
    ]);
    const merged = mergeInventory({
      // Older headless records must not shadow an explicitly listed CLI row.
      // Keep excluding CLI rows for headless runs this worker still owns.
      records: replies ? records.filter((record) => isLive(record.session_id)) : records,
      isLive,
      cliRows: replies
        ? snapshot.rows.filter((row) => row.kind === 'interactive' || row.kind === 'background')
        : snapshot.rows,
      known: (id) => tracker.has(id) || hooked.has(id),
      ptyOf: (pid) => ptys.get(pid),
      external: showAll,
    });
    return { ...merged, detail: snapshot.detail, cliRows: snapshot.rows };
  };

  iii.registerFunction(
    'claude::agents::list',
    async (payload: unknown) => {
      const { all } = ListPayloadSchema.parse(payload ?? {});
      const { agents, hidden, detail } = await inventory(all === true);
      const details = detail ? [detail] : [];
      if (hidden > 0) {
        details.push(
          `${hidden} session${hidden === 1 ? ' on the terminal host is' : 's on the terminal host are'} hidden; set external_sessions: true to list them`,
        );
      }
      return { agents, ...(details.length ? { detail: details.join('; ') } : {}) };
    },
    {
      description:
        "List Claude Code sessions: this worker's headless runs (claude_sessions) plus the interactive and background sessions on the terminal host, read from its `claude agents --json`. Host sessions this worker never saw are hidden unless the external_sessions setting is on.",
      request_format: jsonSchema(ListPayloadSchema),
      response_format: jsonSchema(
        z.object({ agents: z.array(AgentRowSchema), detail: z.string().optional() }),
      ),
    },
  );

  iii.registerFunction(
    'claude::agents::stop',
    async (payload: unknown) => {
      const { id } = StopPayloadSchema.parse(payload ?? {});
      const refuse = (reason: string) => ({ id, stopped: false, reason });
      if (!external()) {
        return refuse('external_sessions is off: this worker does not stop host sessions');
      }
      const prepared = current();
      // Fresh, never cached: the short id is what `claude stop` acts on, and
      // a stale one could name a session that has since been replaced.
      const snapshot = await cliSnapshot(iii, prepared, false);
      const row = snapshot.rows.find((r) => r.sessionId === id && r.kind === 'background');
      if (!row) {
        return refuse(snapshot.detail ?? 'no background session with that id on the terminal host');
      }
      if (typeof row.pid !== 'number') return refuse('the background session is not running');
      const short = str(row.id);
      if (!short || !SHORT_ID.test(short)) {
        return refuse('the background session has no short id to stop it by');
      }
      try {
        const result = await exec(iii, `${quote(prepared.executable)} stop ${short}`, {
          env: prepared.env,
          timeoutMs: 30_000,
        });
        if (result.exit_code === 0) return { id, stopped: true };
        const stderr = result.stderr.trim().slice(0, 200);
        return refuse(`claude stop failed: exit ${result.exit_code}${stderr ? `: ${stderr}` : ''}`);
      } catch (err) {
        return refuse(`claude stop failed: ${String(err)}`);
      }
    },
    {
      description:
        'Stop a background Claude Code session on the terminal host (`claude stop <short id>`); its conversation is kept. Only background sessions with a live process, and only with the external_sessions setting on. A headless run of this worker stops through claude::stop.',
      request_format: jsonSchema(StopPayloadSchema),
      response_format: jsonSchema(
        z.object({ id: z.string(), stopped: z.boolean(), reason: z.string().optional() }),
      ),
    },
  );

  const resolveOpen: ResolveOpen = async (id, request, reserve = true) => {
    const epoch = visibilityEpoch();
    const { agents, detail, cliRows } = await inventory(
      true,
      current(),
      false,
      request ? epoch !== null : external(),
    );
    if (request && epoch !== visibilityEpoch())
      throw new Error('session visibility changed; refresh the list');
    // Without the CLI's listing, a session open in another terminal reads as
    // closed, and resuming it would put a second writer on its transcript.
    if (detail) throw new Error(detail);
    const row = agents.find((agent) => agent.id === id);
    if (!row) throw new Error('no session with that id is listed');
    const prepared = current();
    const claudeId = row.claude_session_id;
    if (request) {
      if (
        !claudeId ||
        !z.guid().safeParse(claudeId).success ||
        request.claude_session_id !== claudeId
      ) {
        throw new Error('the listed conversation changed or is unavailable');
      }
      if (request.mode === 'fork') {
        if (!row.cwd) throw new Error('this session has no recorded working directory');
        return {
          args: [...prepared.args, '--resume', claudeId, '--fork-session'],
          cwd: row.cwd,
          copy: true,
        };
      }
      if (row.pty_session_id) {
        if (
          agents.some(
            (agent) =>
              agent.pty_session_id === row.pty_session_id && agent.claude_session_id !== claudeId,
          )
        )
          throw new Error('ambiguous original terminal');
        return { pty_session_id: row.pty_session_id };
      }
      if (
        row.kind === 'background' &&
        row.short_id &&
        cliRows.some((r) => r.id === row.short_id && r.sessionId !== claudeId)
      )
        throw new Error('ambiguous background session id');
      if (row.kind !== 'headless') {
        if (
          row.kind === 'background' &&
          row.alive &&
          row.short_id &&
          SHORT_ID.test(row.short_id) &&
          epoch !== null
        ) {
          return { args: ['attach', row.short_id], cwd: row.cwd ?? prepared.workspace };
        }
        // A missing pid is not proof of a finished external conversation.
        if (row.alive || !['done', 'stopped', 'error'].includes(row.status)) {
          if (epoch === null) throw new Error('external session access is disabled');
          return { follow: true, claude_session_id: claudeId };
        }
      }
    }
    if (row.kind === 'background') {
      if (!external()) {
        throw new Error('external_sessions is off: this worker does not open host sessions');
      }
      if (!row.short_id || !SHORT_ID.test(row.short_id)) {
        throw new Error('the background session has no short id to open it by');
      }
      return { args: ['attach', row.short_id], cwd: row.cwd ?? prepared.workspace };
    }
    // It becomes argv: anything but a UUID could read as a flag.
    if (!claudeId || !z.guid().safeParse(claudeId).success) {
      throw new Error('this session has no Claude Code conversation to open');
    }
    if (!row.cwd) throw new Error('this session has no recorded working directory');
    const args = [...prepared.args, '--resume', claudeId];
    // By conversation, not by row: `options.resume` can give two runs the same one.
    const alive =
      isOpening(claudeId) ||
      agents.some((agent) => agent.alive && agent.claude_session_id === claudeId);
    if (request && alive)
      throw new Error('the conversation is already open and has no attachable terminal');
    if (alive) return { args: [...args, '--fork-session'], cwd: row.cwd, copy: true };
    if (reserve) opening.set(claudeId, Date.now() + OPENING_MS);
    return { args, cwd: row.cwd };
  };

  const heldElsewhere = async (claudeId: string) => {
    // A listing that fails has no rows: a host with no terminal half still takes its turns.
    const { rows } = await cliSnapshot(iii, current(), false);
    return (
      isOpening(claudeId) ||
      rows.some((row) => typeof row.pid === 'number' && row.sessionId === claudeId)
    );
  };

  const replyRow = async (id: string) => {
    const host = current();
    const snapshot = await inventory(true, host, true).catch(() => null);
    if (!snapshot || snapshot.detail) throw new Error('inventory unavailable');
    const row = snapshot.agents.find((r) => r.kind !== 'headless' && r.claude_session_id === id);
    if (!row) throw new Error('session not listed');
    return { row, env: host.env };
  };

  return { resolveOpen, heldElsewhere, replyRow };
}
