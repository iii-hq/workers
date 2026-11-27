/**
 * `claude::agents::list`: the Claude Code sessions this worker can see, in one
 * list. Its own headless runs come from `claude_sessions`; interactive and
 * background sessions on the terminal host come from `claude agents --json`
 * there, a CLI output with no versioned contract, so every field of it is
 * read as untrusted.
 */

import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto';
import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import { CONFIG_ID, type ReplyAccess, replyLease } from './configuration.js';
import { isLive, jsonSchema } from './run.js';
import { listSessions } from './state.js';
import { readTranscript, Reply } from './terminal/external-transcript.js';
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
  external_only: z.boolean().optional().describe('This row requires live external visibility.'),
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
type OpenRequest = { mode: 'fork'; claude_session_id: string };
export type ResolveOpen = (id: string, request?: OpenRequest) => Promise<OpenSpec>;
export type OpenVisibility = {
  (): number | null;
  (mode: 'legacy' | 'fork'): number | null | Promise<number | null>;
};

const ListPayloadSchema = z.object({
  all: z
    .boolean()
    .optional()
    .describe('Also list completed background sessions (`claude agents --json --all`)'),
  include_config_id: z
    .boolean()
    .optional()
    .describe('Include the configuration entry ID and visibility snapshot for the console viewer.'),
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
const INVENTORY_UNAVAILABLE = 'Terminal host inventory unavailable';

/**
 * How long a conversation counts as open after a plain `--resume` was handed
 * out for it: the CLI lists the session only once its process has started.
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

// For duplicate conversation rows, keep the first live row (or the first
// row if none is live). Stop must use the same row as the listing.
function selectCliRows(rows: CliRow[]) {
  const selected = new Map<string, CliRow>();
  for (const row of rows) {
    const id = str(row.sessionId);
    if (!id) continue;
    const previous = selected.get(id);
    if (!previous || (typeof previous.pid !== 'number' && typeof row.pid === 'number')) {
      selected.set(id, row);
    }
  }
  return selected;
}

export function mergeInventory(input: {
  records: SessionRecord[];
  isLive: (sessionId: string) => boolean;
  cliRows: CliRow[];
  known: (claudeSessionId: string) => boolean;
  /** The console terminal whose process is `pid`. */
  ptyOf: (pid: number) => string | undefined;
  external: boolean;
  markExternal?: boolean;
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

  const cli: AgentRow[] = [];
  for (const [sessionId, row] of selectCliRows(input.cliRows)) {
    // A headless run shows up in the CLI listing too; its headless row stands for it.
    if (ours.has(sessionId) && !inTerminal.has(sessionId)) continue;
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
      ...(input.external &&
      kind === 'background' &&
      typeof row.id === 'string' &&
      SHORT_ID.test(row.id)
        ? { short_id: str(row.id) }
        : {}),
      ...(pty ? { pty_session_id: pty } : {}),
      ...(str(row.name) ? { name: str(row.name) } : {}),
      ...(str(row.cwd) ? { cwd: str(row.cwd) } : {}),
      ...(waitingFor ? { waiting_for: waitingFor } : {}),
      ...(typeof row.startedAt === 'number' ? { started_at_ms: row.startedAt } : {}),
    };
    cli.push(agent);
  }

  let hidden = 0;
  const visible = cli.filter((agent) => {
    if (ours.has(agent.id) || agent.pty_session_id || input.known(agent.id)) return true;
    if (input.external) {
      if (input.markExternal) agent.external_only = true;
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

// The rail allows 20s for the entire list. A timed-out shell command can take
// longer to report over RPC; don't let that discard the worker's own rows.
async function railSnapshot(...args: Parameters<typeof cliSnapshot>) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      cliSnapshot(...args),
      new Promise<Awaited<ReturnType<typeof cliSnapshot>>>((resolve) => {
        timer = setTimeout(() => resolve({ rows: [], detail: INVENTORY_UNAVAILABLE }), 18_000);
      }),
    ]);
  } finally {
    clearTimeout(timer);
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

type PtyRow = { session_id?: unknown; pid?: unknown; ui?: unknown; status?: unknown };

const livePty = (pty: PtyRow) => pty.status === 'attached' || pty.status === 'detached';
/** Read fresh PTY metadata, never the output or the browser's lease book. */
async function claudePtys(iii: IIIClient): Promise<PtyRow[] | null> {
  try {
    const res = await iii.call<unknown, { sessions?: PtyRow[] } | null>({
      function_id: 'shell::pty::sessions',
      payload: {},
      timeoutMs: 15_000,
    });
    return Array.isArray(res?.sessions) ? res.sessions.filter((s) => s?.ui === 'claude') : null;
  } catch (err) {
    console.warn(`claude-code: could not list the console terminals: ${String(err)}`);
    return null;
  }
}

export function registerAgents(
  iii: IIIClient,
  current: () => Prepared,
  tracker: { has(sessionId: string): boolean },
  access: ReplyAccess,
): {
  replyRow: (id: string) => Promise<{ row: AgentRow; env: Prepared['env'] }>;
  /** How a terminal opens a listed session; throws why one cannot be. */
  resolveOpen: ResolveOpen;
  openVisibility: OpenVisibility;
  /** Whether a process on the terminal host has a conversation open, or is about to. */
  heldElsewhere: (claudeSessionId: string) => Promise<boolean>;
} {
  const opening = new Map<string, number>();
  const isOpening = (claudeId: string) => (opening.get(claudeId) ?? 0) > Date.now();

  const inventory = async (
    all: boolean,
    {
      host = current(),
      replies = false,
      showAll = replies,
      markExternal = false,
      readSnapshot = cliSnapshot,
    }: {
      host?: Prepared;
      replies?: boolean;
      showAll?: boolean;
      markExternal?: boolean;
      readSnapshot?: typeof cliSnapshot;
    } = {},
  ) => {
    const [records, snapshot, hooked, ptys] = await Promise.all([
      listSessions(iii),
      readSnapshot(iii, host, all),
      !showAll ? hookSessions(iii) : new Set<string>(),
      replies ? null : claudePtys(iii),
    ]);
    const merge = (externalVisibility = showAll) =>
      mergeInventory({
        // Older headless records must not shadow an explicitly listed CLI row.
        // Keep excluding CLI rows for headless runs this worker still owns.
        records: replies ? records.filter((record) => isLive(record.session_id)) : records,
        isLive,
        cliRows: replies
          ? snapshot.rows.filter((row) => row.kind === 'interactive' || row.kind === 'background')
          : snapshot.rows,
        known: (id) => tracker.has(id) || hooked.has(id),
        ptyOf: (pid) => str(ptys?.find((pty) => pty.pid === pid && livePty(pty))?.session_id),
        external: externalVisibility,
        markExternal,
      });
    return { merge, detail: snapshot.detail };
  };

  // Bind cached inventory to this worker instance as well as its live permission epoch.
  const visibilityKey = randomBytes(32);
  const visibilityToken = (epoch: number | null) =>
    epoch === null
      ? null
      : createHmac('sha256', visibilityKey).update(String(epoch)).digest('base64url');
  const visibilitySchema = z.string().nullable();
  const visibilityArgs = z.object({ confirm_live_config: z.boolean().optional() });
  // Settle within the rail's 5s RPC; undefined means still pending.
  async function withinRail<T>(work: Promise<T>): Promise<{ value: T } | undefined> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([
        work.then((value) => ({ value })),
        new Promise<undefined>((resolve) => {
          timer = setTimeout(() => resolve(undefined), 4_500);
        }),
      ]);
    } finally {
      clearTimeout(timer);
    }
  }
  // A live read that outlasts one probe answers the next heartbeat's probe
  // instead of starting another read that races the same deadline. Only that
  // next probe may use it, and only shortly after it settled.
  type LateConfirmation = { promise: Promise<number | null>; settledAt?: number };
  let late: LateConfirmation | undefined;
  async function confirmRailVisibility() {
    if (late?.settledAt !== undefined && Date.now() - late.settledAt > 5_000) late = undefined;
    let entry = late;
    late = undefined;
    if (!entry) {
      const fresh: LateConfirmation = { promise: access.confirm() };
      const settle = () => {
        fresh.settledAt = Date.now();
      };
      fresh.promise.then(settle, settle);
      entry = fresh;
    }
    const settled = await withinRail(entry.promise);
    if (!settled) late ??= entry;
    return settled ? settled.value : null;
  }
  // Don't publish a grant while the retained-watch boundary is still being
  // captured: a reply written meanwhile would fall behind that boundary.
  async function ordinaryVisibility() {
    const epoch = access.epoch ?? (await access.confirm({ waitForWatchCheckpoint: false }));
    if (epoch !== null) await access.watchCheckpointSettled?.();
    return epoch;
  }
  iii.registerFunction(
    'claude::agents::visibility',
    async (payload: unknown) => {
      const { confirm_live_config } = visibilityArgs.parse(payload ?? {});
      // A pending probe returns null; the rail retries on its next heartbeat.
      const epoch = confirm_live_config
        ? await confirmRailVisibility()
        : ((await withinRail(ordinaryVisibility()))?.value ?? null);
      if (epoch === null || epoch !== access.epoch) return { external_visibility: null };
      return {
        external_visibility: visibilityToken(epoch),
        ...(confirm_live_config ? { live_config_confirmed: true } : {}),
      };
    },
    {
      description:
        'Read the current external inventory visibility snapshot without listing sessions.',
      metadata: { internal: true },
      request_format: jsonSchema(visibilityArgs),
      response_format: jsonSchema(
        z.object({
          external_visibility: visibilitySchema,
          live_config_confirmed: z.literal(true).optional(),
        }),
      ),
    },
  );

  iii.registerFunction(
    'claude::agents::list',
    async (payload: unknown) => {
      const { all, include_config_id } = ListPayloadSchema.parse(payload ?? {});
      // The ordinary worker listing needs inventory even with external access off.
      // Read it alongside confirmation, but authorize external rows only at publication.
      const [epoch, { merge, detail }] = await Promise.all([
        access.confirm({ waitForWatchCheckpoint: !include_config_id }),
        inventory(all === true, {
          showAll: false,
          markExternal: include_config_id,
          readSnapshot: include_config_id ? railSnapshot : cliSnapshot,
        }),
      ]);
      // Recheck at publication; merge the same snapshot without external rows if revoked.
      const currentEpoch = access.epoch;
      // The rail skips the checkpoint wait to stay within its deadline. Until the
      // retained-watch boundary exists, publishing the grant would let a reply
      // written meanwhile fall behind it; the next heartbeat picks the grant up.
      const capturing = include_config_id === true && access.watchCheckpointPending === true;
      const authorized = epoch !== null && epoch === currentEpoch && !capturing;
      const { agents, hidden } = merge(authorized);
      const details = detail ? [authorized ? detail : INVENTORY_UNAVAILABLE] : [];
      if (hidden > 0 && !capturing) {
        details.push(
          `${hidden} session${hidden === 1 ? ' on the terminal host is' : 's on the terminal host are'} hidden; set external_sessions: true to list them`,
        );
      }
      return {
        agents,
        inventory_status: detail ? 'unavailable' : 'available',
        ...(include_config_id
          ? {
              config_id: CONFIG_ID,
              external_visibility: visibilityToken(authorized ? epoch : null),
            }
          : {}),
        ...(details.length ? { detail: details.join('; ') } : {}),
      };
    },
    {
      description:
        "List Claude Code sessions: this worker's headless runs (claude_sessions) plus the interactive and background sessions on the terminal host, read from its `claude agents --json`. External-only host sessions require confirmed live external_sessions permission.",
      request_format: jsonSchema(ListPayloadSchema),
      response_format: jsonSchema(
        z.object({
          agents: z.array(AgentRowSchema),
          inventory_status: z.enum(['available', 'unavailable']),
          config_id: z.string().optional(),
          external_visibility: visibilitySchema.optional(),
          detail: z.string().optional(),
        }),
      ),
    },
  );

  iii.registerFunction(
    'claude::agents::stop',
    async (payload: unknown) => {
      const { id } = StopPayloadSchema.parse(payload ?? {});
      const refuse = (reason: string) => ({ id, stopped: false, reason });
      const epoch = await access.confirm();
      const authorized = () => epoch !== null && epoch === access.epoch;
      if (!authorized()) return refuse(INVENTORY_UNAVAILABLE);
      const prepared = current();
      // Fresh, never cached: the short id is what `claude stop` acts on, and
      // a stale one could name a session that has since been replaced.
      const snapshot = await cliSnapshot(iii, prepared, false);
      if (!authorized()) return refuse(INVENTORY_UNAVAILABLE);
      const row = selectCliRows(snapshot.rows).get(id);
      if (row?.kind !== 'background') {
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
        if (!authorized()) return refuse('claude stop failed');
        const stderr = result.stderr.trim().slice(0, 200);
        return refuse(`claude stop failed: exit ${result.exit_code}${stderr ? `: ${stderr}` : ''}`);
      } catch (err) {
        return refuse(authorized() ? `claude stop failed: ${String(err)}` : 'claude stop failed');
      }
    },
    {
      description:
        'Stop a background Claude Code session on the terminal host (`claude stop <short id>`); its conversation is kept. Only background sessions with a live process, and only with confirmed live external_sessions permission. A headless run of this worker stops through claude::stop.',
      request_format: jsonSchema(StopPayloadSchema),
      response_format: jsonSchema(
        z.object({ id: z.string(), stopped: z.boolean(), reason: z.string().optional() }),
      ),
    },
  );

  function openVisibility(): number | null;
  function openVisibility(mode: 'legacy' | 'fork'): number | null | Promise<number | null>;
  function openVisibility(mode?: 'legacy' | 'fork') {
    // A listed-session describe can admit itself after silent observer replay.
    // Publication checks only read the grant picked at admission.
    if (mode === 'fork') return access.confirm();
    const epoch = access.epoch;
    return mode === 'legacy' && epoch === null ? access.confirm() : epoch;
  }

  const resolveOpen: ResolveOpen = async (id, request) => {
    const epoch = access.epoch;
    const { merge, detail } = await inventory(true, {
      showAll: epoch !== null,
    });
    if (epoch !== access.epoch) throw new Error('session visibility changed; refresh the list');
    if (detail)
      throw new Error(request || epoch === null ? 'session inventory unavailable' : detail);
    const { agents } = merge();
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
      if (!row.cwd) throw new Error('this session has no recorded working directory');
      return {
        args: [...prepared.args, '--resume', claudeId, '--fork-session'],
        cwd: row.cwd,
        copy: true,
      };
    }
    // Calls without a mode keep the legacy terminal behavior for visible sessions.
    if (row.kind === 'background') {
      if (epoch === null) {
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
    if (alive) return { args: [...args, '--fork-session'], cwd: row.cwd, copy: true };
    opening.set(claudeId, Date.now() + OPENING_MS);
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

  const replyRow = async (id: string, host = current()) => {
    const snapshot = await inventory(true, { host, replies: true }).catch(() => null);
    if (!snapshot || snapshot.detail) throw new Error('inventory unavailable');
    const row = snapshot
      .merge()
      .agents.find((r) => r.kind !== 'headless' && r.claude_session_id === id);
    if (!row) throw new Error('session not listed');
    return { row, env: host.env };
  };

  const followKey = randomBytes(32);
  const followArgs = z.strictObject({
    claude_session_id: z.guid(),
    cursor: z.string().max(256).optional(),
  });
  const sign = (id: string, epoch: number, cursor: string) =>
    createHmac('sha256', followKey)
      .update(JSON.stringify([id, epoch, cursor]))
      .digest();
  iii.registerFunction(
    'claude::agents::follow-snapshot',
    async (payload: unknown) => {
      // A fresh live grant is enough for this independently validated read.
      const lease = replyLease(access, await access.confirm({ waitForWatchCheckpoint: false }));
      const generation = lease.epoch;
      const { claude_session_id: id, cursor } = followArgs.parse(payload);
      let boundary: string | undefined;
      if (cursor !== undefined) {
        const parsed = /^([0-9]+:[0-9]+:[0-9]+:[a-f0-9]{64})[.]([a-f0-9]{64})$/.exec(cursor);
        if (
          !parsed ||
          !timingSafeEqual(Buffer.from(parsed[2], 'hex'), sign(id, generation, parsed[1]))
        )
          throw new Error('invalid follow cursor');
        boundary = parsed[1];
      }
      const host = current();
      const listed = lease.read(() => replyRow(id, host));
      // The live grant permits a bounded read, but neither its result nor
      // its error is published until fresh inventory confirms the exact ID.
      const data = (async () => {
        const since = boundary
          ? await lease.read(() => readTranscript(iii, host.env, id, 'recent', boundary))
          : undefined;
        // An earlier reply may still be on the active branch. Only combine
        // reads from the same snapshot; another heartbeat can retry a change.
        const latest = since?.replies.length
          ? since
          : await lease.read(() => readTranscript(iii, host.env, id, 'last'));
        return { since, latest };
      })().catch(() => null);
      const { row, env } = await listed;
      const snapshot = await data;
      lease.check();
      if (!snapshot) throw new Error('transcript unavailable');
      const { since, latest } = snapshot;
      if (since && since.cursor !== latest.cursor) return { retry: true };
      access.offerWatchBoundary?.({ id, epoch: generation, env, cursor: latest.cursor });
      return {
        claude_session_id: id,
        name: row.name ?? null,
        status: row.status,
        last_reply: latest.replies.at(-1) ?? null,
        replies: since?.replies ?? [],
        cursor: [latest.cursor, sign(id, generation, latest.cursor).toString('hex')].join('.'),
      };
    },
    {
      description: 'Read the active replies since a viewer baseline, or the current latest reply.',
      observable: false,
      metadata: { trace_hidden: true },
      request_format: jsonSchema(followArgs),
      response_format: jsonSchema(
        z.union([
          z.object({ retry: z.literal(true) }),
          z.object({
            claude_session_id: z.guid(),
            name: z.string().nullable(),
            status: z.string(),
            last_reply: Reply.nullable(),
            replies: z.array(Reply),
            cursor: z.string(),
          }),
        ]),
      ),
    },
  );
  return { resolveOpen, openVisibility, heldElsewhere, replyRow };
}
