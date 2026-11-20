/**
 * claude::* function registrations. `claude::run` accepts either a bare
 * `prompt` string or the shared agent entrypoint shape (`messages` array of
 * role/content-block messages), so the acp worker (`--brain-fn claude::run`)
 * or another worker can drive Claude Code unchanged.
 */

import { randomUUID } from 'node:crypto';
import { query, type Options, type PermissionResult } from '@anthropic-ai/claude-agent-sdk';
import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import type { Config } from './config.js';
import type { Emit } from './agent-feed.js';
import { fetchIiiContext } from './iii-context.js';
import { installLocal } from './iii-plugin/install.js';
import { headlessTools } from './iii-plugin/shared/mode.js';
import {
  lastAssistant,
  makeAssistantMessage,
  makeFunctionResult,
  mapAssistantContent,
  mapToolResultContent,
  mapUsage,
  type ToolCallIndex,
} from './map.js';
import { listSessions, loadSession, saveSession } from './state.js';
import {
  childSessionOwnership,
  linkChildSession,
  recordTaskOutcome,
  setSessionStatus,
  settleSessionStatus,
  type Ownership,
} from './session-link.js';
import { ChildTranscript, type TurnSink } from './session-writer.js';
import { runTurnSpan } from './trace.js';
import type { AgentMessage, FunctionResultMessage, SessionRecord } from './types.js';

const ContentBlockSchema = z.object({ type: z.string() }).passthrough();
const MessageSchema = z.object({
  role: z.string(),
  content: z.union([z.string(), z.array(ContentBlockSchema)]),
});

const RunPayloadSchema = z.object({
  session_id: z
    .string()
    .optional()
    .describe('iii session id; reuse to resume the same Claude Code conversation'),
  parent_session_id: z
    .string()
    .optional()
    .describe(
      'The session delegating this run. Written to the child session metadata, which is what nests it under its parent in the console — the same link a harness sub-agent carries.',
    ),
  prompt: z.string().optional().describe('The user prompt for this turn'),
  messages: z
    .array(MessageSchema)
    .optional()
    .describe(
      'Alternative to prompt: role/content messages; the last user entry becomes the prompt',
    ),
  model: z.string().optional().describe('Model id or alias; empty = Claude Code default'),
  cwd: z.string().optional().describe('Working directory the turn runs in'),
  system_prompt: z.string().optional().describe('Replace the Claude Code system prompt'),
  append_system_prompt: z.string().optional().describe('Append to the default system prompt'),
  permission_mode: z
    .enum(['default', 'acceptEdits', 'plan', 'bypassPermissions'])
    .optional()
    .describe('Claude Code permission mode for this turn'),
  allowed_tools: z.array(z.string()).optional().describe('Allow rules, e.g. "Bash(git *)"'),
  disallowed_tools: z.array(z.string()).optional().describe('Deny rules'),
  max_turns: z.number().int().positive().optional().describe('Cap on agentic turns'),
  iii_context: z
    .boolean()
    .optional()
    .describe('Append the iii runtime discovery prompt (engine catalog via the iii CLI)'),
  timeout_ms: z
    .number()
    .int()
    .positive()
    .optional()
    .describe('Reserved for callers; not forwarded'),
  /** Raw Agent SDK options, spread over everything the worker derives.
   *  This is the pure pass-through: any Options field the SDK accepts
   *  (fork_session, include_partial_messages, betas, add dirs, mcp
   *  servers, ...) goes through untouched, camelCase as in the SDK. */
  options: z
    .record(z.string(), z.unknown())
    .optional()
    .describe(
      'Raw Agent SDK options forwarded verbatim (camelCase), e.g. forkSession, includePartialMessages',
    ),
});

export type RunPayload = z.infer<typeof RunPayloadSchema>;
export { RunPayloadSchema };

/**
 * A delegated task. Same shape as a run, except the work is named `task` and
 * required: an orchestrator handing off work has one thing to say, and a
 * sub-agent with no task is a bug rather than an empty turn.
 */
const TaskPayloadSchema = RunPayloadSchema.omit({ prompt: true, messages: true }).extend({
  task: z
    .string()
    .min(1)
    .describe(
      "The child's self-contained goal. It cannot see the caller's context, so name every resource it needs.",
    ),
});

const SessionIdSchema = z.object({
  session_id: z.string().describe('iii session id returned by claude::run / claude::start'),
});

// The registry's publish validator has no `$schema` meta-schema registered, so
// the draft-2020-12 `$schema` key z.toJSONSchema stamps at the root fails
// publish. Strip it; the schema body is what the engine + registry consume.
function jsonSchema(schema: z.ZodType): Record<string, unknown> {
  const out = z.toJSONSchema(schema) as Record<string, unknown>;
  delete out.$schema;
  return out;
}

const RUN_REQUEST_FORMAT = jsonSchema(RunPayloadSchema);
const SESSION_ID_FORMAT = jsonSchema(SessionIdSchema);
const TASK_REQUEST_FORMAT = jsonSchema(TaskPayloadSchema);

const UsageSchema = z.object({
  input_tokens: z.number(),
  output_tokens: z.number(),
  cache_read_tokens: z.number().optional(),
  cache_write_tokens: z.number().optional(),
});
const SessionRecordSchema = z.object({
  session_id: z.string(),
  claude_session_id: z.string().nullable(),
  cwd: z.string(),
  model: z.string(),
  status: z.enum(['working', 'done', 'error']),
  turns: z.number(),
  total_cost_usd: z.number(),
  usage: UsageSchema.nullable(),
  updated_at_ms: z.number(),
});
const RUN_RESPONSE_FORMAT = jsonSchema(
  z.object({
    session_id: z.string(),
    claude_session_id: z.string().nullable().optional(),
    result: z.string().optional(),
    stop_reason: z.string().optional(),
    is_error: z.boolean().optional(),
    num_turns: z.number().optional(),
    total_cost_usd: z.number().optional(),
    usage: UsageSchema.nullable().optional(),
    busy: z.boolean().optional(),
    reason: z.string().optional(),
  }),
);
const START_RESPONSE_FORMAT = jsonSchema(
  z.object({
    session_id: z.string(),
    started: z.boolean(),
    // A follow-up while the previous run is still saving: not started.
    busy: z.boolean().optional(),
    reason: z.string().optional(),
  }),
);
const STOP_RESPONSE_FORMAT = jsonSchema(
  z.object({ session_id: z.string(), stopped: z.boolean(), reason: z.string().optional() }),
);
const STATUS_RESPONSE_FORMAT = jsonSchema(
  z.object({ session_id: z.string(), live: z.boolean(), record: SessionRecordSchema.nullable() }),
);
const SESSIONS_RESPONSE_FORMAT = jsonSchema(z.object({ sessions: z.array(SessionRecordSchema) }));

/**
 * A run's slot. `interrupt` is a placeholder until the query exists; a stop
 * that arrives before then sets `stopped`, which the run checks just before
 * creating the query, so the stop is honored rather than lost.
 */
type LiveRun = { interrupt: () => Promise<void>; stopped?: boolean };
const live = new Map<string, LiveRun>();

/** Sessions this worker has run a turn in since it started, kept until the
 *  startup scan ends: that scan must not settle any of them. */
const ranSinceBoot = new Set<string>();
let trackingSinceBoot = true;

/** Whether this worker has run (or is running) a turn in `sessionId` since it started. */
export function ranSinceStart(sessionId: string): boolean {
  return live.has(sessionId) || ranSinceBoot.has(sessionId);
}

/** The startup scan is over: stop remembering. */
export function startupScanDone(): void {
  trackingSinceBoot = false;
  ranSinceBoot.clear();
}

/** Best-effort: flip a session record to `error` so a failed background run
 *  never leaves it stuck in `working`. Swallows its own failure. */
async function markSessionError(iii: IIIClient, session_id: string): Promise<void> {
  try {
    const record = await loadSession(iii, session_id);
    if (record && record.status === 'working') {
      record.status = 'error';
      record.updated_at_ms = Date.now();
      await saveSession(iii, record);
    }
  } catch (err) {
    console.error(`failed to mark session ${session_id} error: ${String(err)}`);
  }
}

export function extractPrompt(payload: RunPayload): string {
  if (typeof payload.prompt === 'string') return payload.prompt;
  const users = (payload.messages ?? []).filter((m) => m.role === 'user');
  const last = users[users.length - 1];
  if (!last) throw new Error('claude::run requires `prompt` or a user message in `messages`');
  if (typeof last.content === 'string') return last.content;
  return last.content
    .map((b) => ('text' in b && typeof b.text === 'string' ? b.text : ''))
    .filter(Boolean)
    .join('\n');
}

function gatedCanUseTool(iii: IIIClient, session_id: string) {
  return async (toolName: string, input: Record<string, unknown>): Promise<PermissionResult> => {
    try {
      const res = await iii.call<unknown, { decision?: string; behavior?: string }>({
        function_id: 'policy::check_permissions',
        payload: { session_id, function_id: `claude-code::${toolName}`, args: input },
        timeoutMs: 5_000,
      });
      const decision = res?.decision ?? res?.behavior ?? 'deny';
      if (decision === 'allow') return { behavior: 'allow', updatedInput: input };
      return { behavior: 'deny', message: `denied by approval gate (${decision})` };
    } catch {
      return { behavior: 'deny', message: 'approval gate unreachable (fail-closed)' };
    }
  };
}

function callerOptions(payload: RunPayload, cfg: Config): Partial<Options> {
  const opts = { ...(payload.options as Partial<Options> | undefined) };
  if (cfg.approval_gate) {
    // enforcement keys must not shadow the gate
    delete (opts as Record<string, unknown>).canUseTool;
    delete (opts as Record<string, unknown>).permissionMode;
  }
  return opts;
}

export async function executeRun(
  iii: IIIClient,
  cfg: Config,
  emit: Emit,
  emitRaw: Emit,
  payload: RunPayload,
): Promise<Record<string, unknown>> {
  const session_id = payload.session_id ?? randomUUID();
  const prompt = extractPrompt(payload);
  // One live run per session. Reserve the slot atomically: the has-check and
  // the set happen with NO await between them, so two concurrent same-session
  // calls cannot both pass (JS is single-threaded). The handle's interrupt is
  // a no-op placeholder until the real query exists; everything past here runs
  // inside the try/finally below so the slot is always released.
  if (live.has(session_id)) {
    return { session_id, busy: true, reason: 'a run is already active for this session' };
  }
  const handle: LiveRun = {
    interrupt: async () => {
      handle.stopped = true;
    },
  };
  live.set(session_id, handle);
  if (trackingSinceBoot) ranSinceBoot.add(session_id);

  try {
    // A run that names a parent is a sub-agent: give it a linked session first,
    // because `parent_session_id` only applies on creation, and that link is
    // what nests it in the console. A later turn on the same child (resumed
    // with its session id alone) stays a child.
    const link = payload.parent_session_id
      ? await linkChildSession(iii, {
          sessionId: session_id,
          parentSessionId: payload.parent_session_id,
          task: prompt,
        })
      : undefined;
    // Ownership, not the link's success, decides: `session::ensure` adopts an
    // existing session without comparing owners, but it does answer with the
    // stored metadata — the evidence used first. Only a session this worker
    // created is ever written to.
    let ownership: Ownership =
      link && link.owner !== 'unknown' ? link.owner : await childSessionOwnership(iii, session_id);
    if (ownership === 'child') await setSessionStatus(iii, session_id, 'working');
    // The feeds are a live tape: a window opened after the run has nothing to
    // replay from them. A child's turn is written to session-manager AS IT
    // HAPPENS — the prompt now, each message streaming in place — so the
    // console renders it like any chat, whichever entrypoint started it. When
    // ownership could not be read yet, the writer only keeps the turn and
    // asks again at the end.
    const writer =
      ownership === 'other'
        ? undefined
        : new ChildTranscript(iii, session_id, ownership === 'child', prompt);
    const finish = async (status: 'done' | 'error', reason?: string) => {
      if (!writer) return;
      const written = await writer.finish(async () => {
        ownership = await childSessionOwnership(iii, session_id);
        if (ownership !== 'child') return false;
        await setSessionStatus(iii, session_id, 'working');
        return true;
      });
      if (!written.write) return;
      // Transcript first, status second: the console opens a child on `done`,
      // and a transcript that could not be written whole is never `done`.
      if (written.complete) await settleSessionStatus(iii, session_id, status, reason);
      else {
        await settleSessionStatus(
          iii,
          session_id,
          'error',
          reason ?? 'the transcript could not be saved in full',
        );
      }
    };
    // One turn, one traced scope. The identity keys are the harness's own, so
    // a headless Claude Code turn groups and labels itself in the console's
    // trace views like a native turn — and every call the turn makes inherits
    // them, because baggage propagates across the bus.
    const result = await runTurnSpan(
      'claude::run turn',
      {
        sessionId: session_id,
        turnId: randomUUID(),
        kind: 'claude.run',
        message: prompt,
      },
      () => runReserved(iii, cfg, emit, emitRaw, payload, session_id, prompt, handle, writer),
    ).catch(async (err) => {
      writer?.note('error', String(err));
      await finish('error', String(err));
      throw err;
    });
    const stop = String(result.stop_reason ?? 'end');
    const said =
      typeof result.result === 'string' && result.result
        ? result.result
        : `Claude Code stopped: ${stop}`;
    // A turn that hit a cap or failed ends with how it ended: entries already
    // written cannot have their stop reason rewritten.
    if (stop !== 'end') writer?.note(stop, said);
    await finish(
      result.is_error === true ? 'error' : 'done',
      result.is_error === true ? said : stop === 'interrupted' ? 'stopped' : undefined,
    );
    return result;
  } finally {
    if (live.get(session_id) === handle) live.delete(session_id);
  }
}

function publishing(feed: Emit, name: string): Emit {
  return async (sessionId, event) => {
    try {
      await feed(sessionId, event);
    } catch (err) {
      console.warn(`claude-code: ${name} delivery failed for ${sessionId}: ${String(err)}`);
    }
  };
}

async function runReserved(
  iii: IIIClient,
  cfg: Config,
  emit: Emit,
  emitRaw: Emit,
  payload: RunPayload,
  session_id: string,
  prompt: string,
  handle: LiveRun,
  sink?: TurnSink,
): Promise<Record<string, unknown>> {
  const prior = await loadSession(iii, session_id);
  const d = cfg.defaults;
  // A feed is a live view: one that cannot deliver must never end the turn or
  // cost the child transcript a message.
  const agentFeed = publishing(emit, 'claude::on-agent-event');
  const rawFeed = publishing(emitRaw, 'claude::on-raw-event');

  const record: SessionRecord = prior ?? {
    session_id,
    claude_session_id: null,
    cwd: payload.cwd ?? d.cwd,
    model: payload.model ?? d.model,
    status: 'working',
    turns: 0,
    total_cost_usd: 0,
    usage: null,
    updated_at_ms: Date.now(),
  };
  if (payload.cwd) record.cwd = payload.cwd;
  if (payload.model) record.model = payload.model;

  const userAppend = payload.append_system_prompt ?? d.append_system_prompt;
  // The iii context comes from the `iii-directory` worker, which owns it: this
  // worker keeps no copy of the prompt or of the skills index.
  const iiiContext = payload.iii_context ?? cfg.iii_context;
  const context = iiiContext ? await fetchIiiContext(iii) : null;
  // The same `iii` plugin the terminal half loads (hooks, runtime skill, the
  // engine's mcp-server as its MCP server, /iii:* commands): the SDK turns a
  // local plugin into the CLI's own `--plugin-dir`. It lives in a directory
  // of this turn's own, removed when the turn ends.
  const plugin = iiiContext
    ? await installLocal(iii)
    : { dir: '', detail: '', mode: '', mcp: false, dispose: async () => {} };
  if (plugin.detail) console.warn(`claude-code: ${plugin.detail}`);
  try {
    // default-mcp is appended to Claude Code's own system prompt: an agent
    // (`--agent`) would replace it, and with it Claude Code's operating rules.
    // The runtime text teaches the `iii` CLI as the only way in: with
    // mcp-server, default-mcp and the iii tools take its place.
    const runtime = plugin.mcp ? '' : (context?.text ?? '');
    const append = [runtime, plugin.mode, userAppend].filter(Boolean).join('\n\n');
    const plugins = plugin.dir ? [{ type: 'local' as const, path: plugin.dir }] : [];
    const options: Options = {
      ...(plugins.length ? { plugins } : {}),
      ...(record.model ? { model: record.model } : {}),
      ...(record.cwd ? { cwd: record.cwd } : {}),
      ...(prior?.claude_session_id ? { resume: prior.claude_session_id } : {}),
      maxTurns: payload.max_turns ?? d.max_turns,
      // with the approval gate on, the operator wants every tool call routed
      // through canUseTool; bypassPermissions skips it, so the gate forces a
      // mode that keeps canUseTool authoritative regardless of the caller
      permissionMode: cfg.approval_gate
        ? 'default'
        : (payload.permission_mode ?? d.permission_mode),
      allowedTools: [
        ...(payload.allowed_tools ?? d.allowed_tools),
        // the iii context teaches discovery through the CLI; the matching
        // allow rule is what lets those calls run headless
        ...(iiiContext ? ['Bash(iii *)'] : []),
        // With mcp-server, the iii tools run without a prompt nobody would
        // answer: rbac-proxy is what bounds them.
        ...(plugin.mcp ? headlessTools(cfg.approval_gate) : []),
      ],
      disallowedTools: payload.disallowed_tools ?? d.disallowed_tools,
      systemPrompt: payload.system_prompt
        ? payload.system_prompt
        : { type: 'preset', preset: 'claude_code', ...(append ? { append } : {}) },
      settingSources: [],
      ...(cfg.claude_executable ? { pathToClaudeCodeExecutable: cfg.claude_executable } : {}),
      // A child session renders its turn live: ask for the stream. Before the
      // caller's options, so a caller can still turn it off.
      ...(sink?.streaming ? { includePartialMessages: true } : {}),
      ...callerOptions(payload, cfg),
      ...(cfg.approval_gate ? { canUseTool: gatedCanUseTool(iii, session_id) } : {}),
    };

    // A stop that arrived during setup: start no query. The turn ends like an
    // interrupted one (stop_reason `interrupted`, not an error). No await sits
    // between this check and the handle's promotion, so no stop slips between.
    const stoppedEarly = handle.stopped === true;
    const q = stoppedEarly ? null : query({ prompt, options });
    // promote the placeholder to the real interrupt now that the query exists.
    // Its answer is discarded on purpose: a caller of `claude::stop` learns
    // whether a run WAS live from the stop response, and the SDK's control
    // reply adds nothing a caller can act on.
    if (q) {
      handle.interrupt = async () => {
        await q.interrupt();
      };
    }

    // persist `working` only once the query + live handle exist, so a throw
    // during setup never leaves the record stuck in `working`
    record.status = 'working';
    record.updated_at_ms = Date.now();
    await saveSession(iii, record);

    const transcript: AgentMessage[] = [];
    const pendingResults: FunctionResultMessage[] = [];
    const calls: ToolCallIndex = new Map();
    let resultText = '';
    let stopReason = 'end';
    let isError = false;
    /** Whether a claude::stop had landed when the turn's outcome arrived: a
     *  stop after an SDK error result must not hide that failure. */
    let stoppedAtOutcome = false;

    if (stoppedEarly) {
      stopReason = 'interrupted';
      resultText = 'Stopped before the turn started.';
    }
    try {
      for await (const msg of q ?? []) {
        // The child transcript sees every event before the feeds or the record
        // do, so neither failing can lose what the turn produced.
        if (msg.type === 'stream_event') sink?.streamEvent((msg as { event?: unknown }).event);
        if (msg.type === 'result') stoppedAtOutcome = handle.stopped === true;
        await rawFeed(session_id, msg);
        if (msg.type === 'system' && msg.subtype === 'init') {
          record.claude_session_id = msg.session_id;
          await saveSession(iii, record);
        } else if (msg.type === 'assistant') {
          const usage = mapUsage(msg.message.usage);
          const content = mapAssistantContent(msg.message.content as never, calls);
          const assistant = makeAssistantMessage(content, msg.message.model ?? record.model, usage);
          sink?.assistant(
            (msg.message as { id?: string }).id,
            content,
            msg.message.model ?? record.model,
            usage,
          );
          transcript.push(assistant);
          await agentFeed(session_id, { type: 'message_complete', message: assistant });
          for (const block of content) {
            if (block.type === 'function_call') {
              await agentFeed(session_id, {
                type: 'function_execution_start',
                function_call_id: block.id,
                function_id: block.function_id,
                args: block.arguments,
              });
            }
          }
        } else if (msg.type === 'user') {
          const blocks = Array.isArray(msg.message.content) ? msg.message.content : [];
          for (const b of blocks as unknown as Array<Record<string, unknown>>) {
            if (b.type !== 'tool_result') continue;
            const callId = String(b.tool_use_id ?? '');
            const call = calls.get(callId);
            const content = mapToolResultContent(b.content);
            const fr = makeFunctionResult(
              callId,
              call?.function_id ?? 'claude-code::unknown',
              content,
              b.is_error === true,
            );
            sink?.functionResult(fr);
            transcript.push(fr);
            pendingResults.push(fr);
            await agentFeed(session_id, {
              type: 'function_execution_end',
              function_call_id: callId,
              function_id: fr.function_id,
              result: { content, details: null },
              is_error: fr.is_error,
              duration_ms: call ? Date.now() - call.started_at : 0,
            });
          }
        } else if (msg.type === 'result') {
          record.claude_session_id = msg.session_id ?? record.claude_session_id;
          record.turns += msg.num_turns ?? 1;
          record.total_cost_usd += msg.total_cost_usd ?? 0;
          record.usage = mapUsage(msg.usage);
          isError = msg.is_error === true;
          stopReason = msg.subtype === 'success' ? 'end' : msg.subtype;
          // A failed result may carry its reason only in `errors`.
          resultText =
            'result' in msg && typeof msg.result === 'string' && msg.result
              ? msg.result
              : 'errors' in msg && Array.isArray(msg.errors) && msg.errors.length
                ? msg.errors.map(String).join('; ')
                : '';
        }
      }
    } catch (err) {
      stoppedAtOutcome = handle.stopped === true;
      isError = true;
      stopReason = 'error';
      resultText = String(err);
    }
    // A requested stop: the interrupted query answers with an error result (or
    // throws), which is how it was stopped, not a failure. A turn that finished
    // before the stop landed keeps its own outcome.
    if (stoppedAtOutcome && !stoppedEarly && (isError || stopReason !== 'end')) {
      isError = false;
      stopReason = 'interrupted';
      resultText = 'Stopped by claude::stop.';
    }
    // slot release is handled by executeRun's finally (covers setup throws too)

    record.status = isError ? 'error' : 'done';
    record.updated_at_ms = Date.now();
    await saveSession(iii, record);

    if (transcript.length === 0) {
      const closing = makeAssistantMessage(
        [{ type: 'text', text: resultText }],
        record.model,
        record.usage,
        stopReason,
      );
      sink?.terminal(closing);
      transcript.push(closing);
    }
    await agentFeed(session_id, {
      type: 'turn_end',
      message: lastAssistant(transcript),
      function_results: pendingResults,
    });
    await agentFeed(session_id, { type: 'agent_end', messages: transcript });

    return {
      session_id,
      claude_session_id: record.claude_session_id,
      result: resultText,
      stop_reason: stopReason,
      is_error: isError,
      num_turns: record.turns,
      total_cost_usd: record.total_cost_usd,
      usage: record.usage,
    };
  } finally {
    await plugin.dispose();
  }
}

export function register(iii: IIIClient, getCfg: () => Config, emit: Emit, emitRaw: Emit): void {
  iii.registerFunction(
    'claude::run',
    async (payload: unknown) =>
      executeRun(iii, getCfg(), emit, emitRaw, RunPayloadSchema.parse(payload ?? {})),
    {
      description:
        'Run one Claude Code turn and wait for the result. Accepts `prompt` or a `messages` array plus a raw SDK `options` pass-through; publishes raw Claude Code messages on claude::on-raw-event and AgentEvent frames on claude::on-agent-event (observe with args { session_id }), and returns {session_id, result, usage, total_cost_usd}.',
      request_format: RUN_REQUEST_FORMAT,
      response_format: RUN_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'claude::start',
    async (payload: unknown) => {
      const parsed = RunPayloadSchema.parse(payload ?? {});
      const session_id = parsed.session_id ?? randomUUID();
      // Refuse here, not in the background: a busy result from executeRun
      // would be discarded and the caller told `started: true` for a prompt
      // that never runs (a follow-up sent on `agent_end` lands in the window
      // where the previous turn is still writing its transcript).
      if (live.has(session_id)) {
        return {
          session_id,
          started: false,
          busy: true,
          reason: 'a run is already active for this session',
        };
      }
      void executeRun(iii, getCfg(), emit, emitRaw, { ...parsed, session_id }).catch(
        async (err) => {
          console.error(`claude::start background run failed for ${session_id}: ${String(err)}`);
          // never leave the record stuck in `working`: a failure inside the
          // turn's own terminal save lands here, so mark it error best-effort
          await markSessionError(iii, session_id);
        },
      );
      return { session_id, started: true };
    },
    {
      description:
        'Start a Claude Code turn and return immediately; observe claude::on-agent-event with args { session_id } for progress and turn_end.',
      request_format: RUN_REQUEST_FORMAT,
      response_format: START_RESPONSE_FORMAT,
    },
  );

  // The sub-agent entrypoint an orchestrator hands work to without waiting: a
  // call (or an observer targeting this id) delivers a task, the ids come back
  // at once, and the outcome arrives the way a harness sub-agent's does — on
  // `claude::on-agent-event`, in the child session, never as a return value
  // the caller waits for.
  iii.registerFunction(
    'claude::task',
    async (payload: unknown) => {
      const parsed = TaskPayloadSchema.parse(payload ?? {});
      const session_id = parsed.session_id ?? randomUUID();
      if (live.has(session_id)) {
        return {
          session_id,
          started: false,
          busy: true,
          reason: 'a run is already active for this session',
        };
      }
      const run: RunPayload = {
        ...parsed,
        session_id,
        prompt: parsed.task,
      };
      // Fire-and-forget, and REACTIVE: the caller is not held open for the
      // length of an agent run. When the task settles, its outcome is written
      // to state under `agent_tasks/<session id>`, which is what an
      // orchestrator observes (`state::on-changed`) and gets woken by — the same
      // shape a harness sub-agent uses, with no polling and no blocking call.
      // The transcript itself is persisted by executeRun, like any linked run.
      void executeRun(iii, getCfg(), emit, emitRaw, run)
        .then(async (result) => {
          // An SDK error result resolves normally: the outcome follows it,
          // since the orchestrator reads this record as the task's outcome.
          const text = typeof result.result === 'string' ? result.result : undefined;
          const failed = result.is_error === true;
          await recordTaskOutcome(iii, {
            session_id,
            parent_session_id: parsed.parent_session_id,
            agent: 'claude-code',
            task: parsed.task,
            status: failed ? 'error' : 'done',
            ...(failed
              ? { error: text || `Claude Code stopped: ${String(result.stop_reason ?? 'error')}` }
              : { result: text }),
            updated_at_ms: Date.now(),
          });
        })
        .catch(async (err) => {
          console.error(`claude::task background run failed for ${session_id}: ${String(err)}`);
          await markSessionError(iii, session_id);
          await recordTaskOutcome(iii, {
            session_id,
            parent_session_id: parsed.parent_session_id,
            agent: 'claude-code',
            task: parsed.task,
            status: 'error',
            error: String(err),
            updated_at_ms: Date.now(),
          });
        });
      return { session_id, started: true };
    },
    {
      description:
        'Delegate one task to Claude Code and return its session id immediately — the sub-agent shape: it never parks the caller. The outcome is written to state under scope `agent_tasks`, key the child session id, so observe state::on-changed with filters on input.scope and input.key BEFORE calling and be woken by it; progress arrives on claude::on-agent-event (observe with args { session_id }). Pass `parent_session_id` to nest the child under the session that delegated it.',
      request_format: TASK_REQUEST_FORMAT,
      response_format: START_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'claude::stop',
    async (payload: unknown) => {
      const { session_id } = SessionIdSchema.parse(payload ?? {});
      const run = live.get(session_id);
      if (!run) return { session_id, stopped: false, reason: 'no live run' };
      // Marked first: the run reads it to tell a requested stop from a failure.
      run.stopped = true;
      await run.interrupt();
      return { session_id, stopped: true };
    },
    {
      description: 'Stop a live Claude Code run for a session.',
      request_format: SESSION_ID_FORMAT,
      response_format: STOP_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'claude::status',
    async (payload: unknown) => {
      const { session_id } = SessionIdSchema.parse(payload ?? {});
      const record = await loadSession(iii, session_id);
      return { session_id, live: live.has(session_id), record };
    },
    {
      description: 'Check the status of a Claude Code session.',
      request_format: SESSION_ID_FORMAT,
      response_format: STATUS_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'claude::sessions::list',
    async () => ({ sessions: await listSessions(iii) }),
    {
      description: 'List every Claude Code session this worker has run.',
      request_format: { type: 'object', properties: {} },
      response_format: SESSIONS_RESPONSE_FORMAT,
    },
  );
}
