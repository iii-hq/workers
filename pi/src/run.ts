/**
 * pi::* function registrations. `pi::run` accepts either a bare `prompt`
 * string or the shared agent entrypoint shape (`messages` array of
 * role/content-block messages), so anything that can drive
 * `run::start_and_wait` — the acp worker, the console, another worker — can
 * drive Pi unchanged.
 *
 * Pi runs the loop in-process and pushes events through `session.subscribe`.
 * Because that listener is synchronous and stream emits are async, events are
 * drained through a serial promise chain so frames land on the streams in
 * arrival order.
 */

import { randomUUID } from 'node:crypto';
import type { AgentSession } from '@earendil-works/pi-coding-agent';
import type { ISdk } from 'iii-sdk';
import { z } from 'zod';
import type { Config } from './config.js';
import type { Emit } from './events.js';
import { III_CONTEXT_PROMPT } from './iii-prompt.js';
import {
  lastAssistant,
  makeAssistantMessage,
  makeFunctionResult,
  mapMessageContent,
  mapToolResultContent,
  mapUsage,
  toolFunctionId,
  type ToolCallIndex,
} from './map.js';
import { buildSession } from './session.js';
import { listSessions, loadSession, saveSession } from './state.js';
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
    .describe('iii session id; reuse to resume the same Pi conversation'),
  prompt: z.string().optional().describe('The user prompt for this turn'),
  messages: z
    .array(MessageSchema)
    .optional()
    .describe(
      'Alternative to prompt: role/content messages; the last user entry becomes the prompt',
    ),
  model: z.string().optional().describe('Model as "provider/modelId"; empty = Pi settings default'),
  cwd: z.string().optional().describe('Working directory the turn runs in'),
  thinking_level: z
    .enum(['off', 'minimal', 'low', 'medium', 'high', 'xhigh'])
    .optional()
    .describe('Reasoning depth for this turn'),
  tools: z
    .array(z.string())
    .optional()
    .describe('Tool allowlist; empty = Pi defaults (read, bash, edit, write)'),
  iii_context: z
    .boolean()
    .optional()
    .describe('Prepend the iii runtime discovery context (engine catalog via the iii CLI)'),
  timeout_ms: z
    .number()
    .int()
    .positive()
    .optional()
    .describe('Reserved for callers; not forwarded'),
});

export type RunPayload = z.infer<typeof RunPayloadSchema>;
export { RunPayloadSchema };

const SessionIdSchema = z.object({
  session_id: z.string().describe('iii session id returned by pi::run / pi::start'),
});

const SteerPayloadSchema = z.object({
  session_id: z.string().describe('iii session id of a live run'),
  prompt: z.string().describe('Instruction to inject into the live run'),
});

// The registry's publish validator has no `$schema` meta-schema registered, so
// the draft-2020-12 `$schema` key z.toJSONSchema stamps at the root fails
// validation. Strip it; the schema body is what the engine + registry consume.
function jsonSchema(schema: z.ZodType): Record<string, unknown> {
  const out = z.toJSONSchema(schema) as Record<string, unknown>;
  delete out.$schema;
  return out;
}

const RUN_REQUEST_FORMAT = jsonSchema(RunPayloadSchema);
const SESSION_ID_FORMAT = jsonSchema(SessionIdSchema);
const STEER_REQUEST_FORMAT = jsonSchema(SteerPayloadSchema);

const UsageSchema = z.object({
  input_tokens: z.number(),
  output_tokens: z.number(),
  cache_read_tokens: z.number().optional(),
  cache_write_tokens: z.number().optional(),
});

const RunResultSchema = z.object({
  session_id: z.string(),
  pi_session_id: z.string().nullable().optional(),
  result: z.string().optional(),
  stop_reason: z.string().optional(),
  is_error: z.boolean().optional(),
  num_turns: z.number().optional(),
  total_cost_usd: z.number().optional(),
  usage: UsageSchema.nullable().optional(),
  busy: z.boolean().optional(),
  reason: z.string().optional(),
});
const StartResultSchema = z.object({
  session_id: z.string(),
  started: z.boolean(),
  busy: z.boolean().optional(),
  reason: z.string().optional(),
});
const SteerResultSchema = z.object({
  session_id: z.string(),
  steered: z.boolean(),
  reason: z.string().optional(),
});
const FollowUpResultSchema = z.object({
  session_id: z.string(),
  queued: z.boolean(),
  reason: z.string().optional(),
});
const StopResultSchema = z.object({
  session_id: z.string(),
  stopped: z.boolean(),
  reason: z.string().optional(),
});
const SessionRecordSchema = z.object({
  session_id: z.string(),
  pi_session_id: z.string().nullable(),
  session_file: z.string().nullable(),
  cwd: z.string(),
  model: z.string(),
  status: z.enum(['working', 'done', 'error']),
  turns: z.number(),
  total_cost_usd: z.number(),
  usage: UsageSchema.nullable(),
  updated_at_ms: z.number(),
});
const StatusResultSchema = z.object({
  session_id: z.string(),
  live: z.boolean(),
  record: SessionRecordSchema.nullable(),
});
const SessionsResultSchema = z.object({
  sessions: z.array(SessionRecordSchema),
});

const RUN_RESPONSE_FORMAT = jsonSchema(RunResultSchema);
const START_RESPONSE_FORMAT = jsonSchema(StartResultSchema);
const STEER_RESPONSE_FORMAT = jsonSchema(SteerResultSchema);
const FOLLOWUP_RESPONSE_FORMAT = jsonSchema(FollowUpResultSchema);
const STOP_RESPONSE_FORMAT = jsonSchema(StopResultSchema);
const STATUS_RESPONSE_FORMAT = jsonSchema(StatusResultSchema);
const SESSIONS_RESPONSE_FORMAT = jsonSchema(SessionsResultSchema);

type LiveRun = { session: AgentSession };
const live = new Map<string, LiveRun>();

/** Best-effort: flip a session record to `error` so a failed background run
 *  never leaves it stuck in `working`. Swallows its own failure. */
async function markSessionError(iii: ISdk, session_id: string): Promise<void> {
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
  if (!last) throw new Error('pi::run requires `prompt` or a user message in `messages`');
  if (typeof last.content === 'string') return last.content;
  return last.content
    .map((b) => ('text' in b && typeof b.text === 'string' ? b.text : ''))
    .filter(Boolean)
    .join('\n');
}

export async function executeRun(
  iii: ISdk,
  cfg: Config,
  emit: Emit,
  emitRaw: Emit,
  payload: RunPayload,
): Promise<Record<string, unknown>> {
  const session_id = payload.session_id ?? randomUUID();
  const prompt = extractPrompt(payload);
  // One live run per session. Reserve the slot atomically: the has-check and
  // the set happen with NO await between them, so two concurrent same-session
  // calls cannot both pass (JS is single-threaded). Everything past here runs
  // inside the try/finally so the slot is always released.
  if (live.has(session_id)) {
    return { session_id, busy: true, reason: 'a run is already active for this session' };
  }
  const placeholder = { session: undefined as unknown as AgentSession };
  live.set(session_id, placeholder);

  try {
    return await runReserved(iii, cfg, emit, emitRaw, payload, session_id, prompt, placeholder);
  } finally {
    if (live.get(session_id) === placeholder) live.delete(session_id);
  }
}

async function runReserved(
  iii: ISdk,
  cfg: Config,
  emit: Emit,
  emitRaw: Emit,
  payload: RunPayload,
  session_id: string,
  prompt: string,
  slot: LiveRun,
): Promise<Record<string, unknown>> {
  const prior = await loadSession(iii, session_id);
  const d = cfg.defaults;
  const model = payload.model ?? d.model;
  const cwd = payload.cwd ?? prior?.cwd ?? d.cwd;

  const record: SessionRecord = prior ?? {
    session_id,
    pi_session_id: null,
    session_file: null,
    cwd,
    model,
    status: 'working',
    turns: 0,
    total_cost_usd: 0,
    usage: null,
    updated_at_ms: Date.now(),
  };
  if (payload.cwd) record.cwd = payload.cwd;
  if (payload.model) record.model = payload.model;

  // iii context teaches discovery through the CLI; prepend it on a fresh
  // session only — on resume it is already in the conversation history.
  const iiiContext = payload.iii_context ?? cfg.iii_context;
  const promptText =
    iiiContext && !prior?.session_file ? `${III_CONTEXT_PROMPT}\n\n---\n\n${prompt}` : prompt;

  const session = await buildSession({
    cwd: record.cwd || undefined,
    model: record.model || undefined,
    thinkingLevel: payload.thinking_level ?? d.thinking_level,
    tools: payload.tools ?? d.tools,
    agentDir: d.agent_dir || undefined,
    resumeFile: prior?.session_file ?? null,
  });
  slot.session = session;

  record.status = 'working';
  record.updated_at_ms = Date.now();
  await saveSession(iii, record);

  const transcript: AgentMessage[] = [];
  const pendingResults: FunctionResultMessage[] = [];
  const calls: ToolCallIndex = new Map();
  let resultText = '';
  let stopReason = 'end';
  let isError = false;

  // Serial drain: the sync subscribe listener enqueues async emits so frames
  // reach the streams in arrival order without blocking Pi's loop.
  let chain: Promise<void> = Promise.resolve();
  const enqueue = (fn: () => Promise<void>) => {
    chain = chain.then(fn).catch((err) => console.warn(`pi event emit failed: ${String(err)}`));
  };

  const unsubscribe = session.subscribe((event) => {
    enqueue(async () => {
      await emitRaw(session_id, event);
      await translate(event);
    });
  });

  async function translate(event: { type: string; [k: string]: unknown }): Promise<void> {
    if (event.type === 'message_end') {
      const message = event.message as { role?: string } | undefined;
      if (message?.role !== 'assistant') return;
      const content = mapMessageContent(message);
      const assistant = makeAssistantMessage(content, record.model, record.usage);
      transcript.push(assistant);
      const text = content
        .filter((b): b is { type: 'text'; text: string } => b.type === 'text')
        .map((b) => b.text)
        .join('');
      if (text) resultText = text;
      await emit(session_id, { type: 'message_complete', message: assistant });
    } else if (event.type === 'tool_execution_start') {
      const callId = String(event.toolCallId ?? '');
      const function_id = toolFunctionId(String(event.toolName ?? 'unknown'));
      calls.set(callId, { function_id, started_at: Date.now() });
      await emit(session_id, {
        type: 'function_execution_start',
        function_call_id: callId,
        function_id,
        args: event.args ?? {},
      });
    } else if (event.type === 'tool_execution_end') {
      const callId = String(event.toolCallId ?? '');
      const call = calls.get(callId);
      const function_id = call?.function_id ?? toolFunctionId(String(event.toolName ?? 'unknown'));
      const content = mapToolResultContent(event.result);
      const fr = makeFunctionResult(callId, function_id, content, event.isError === true);
      transcript.push(fr);
      pendingResults.push(fr);
      await emit(session_id, {
        type: 'function_execution_end',
        function_call_id: callId,
        function_id,
        result: { content, details: null },
        is_error: fr.is_error,
        duration_ms: call ? Date.now() - call.started_at : 0,
      });
    }
  }

  try {
    await session.prompt(promptText);
    await chain;
    const stats = session.getSessionStats();
    record.usage = mapUsage(stats.tokens);
    record.total_cost_usd += stats.cost ?? 0;
    record.turns += 1;
    record.pi_session_id = session.sessionId;
    record.session_file = stats.sessionFile ?? session.sessionFile ?? record.session_file;
  } catch (err) {
    await chain;
    isError = true;
    stopReason = 'error';
    resultText = String(err);
  } finally {
    unsubscribe();
    session.dispose();
  }

  record.status = isError ? 'error' : 'done';
  record.updated_at_ms = Date.now();
  await saveSession(iii, record);

  if (transcript.length === 0 || transcript[transcript.length - 1].role !== 'assistant') {
    transcript.push(
      makeAssistantMessage(
        [{ type: 'text', text: resultText }],
        record.model,
        record.usage,
        stopReason,
      ),
    );
  }
  await emit(session_id, {
    type: 'turn_end',
    message: lastAssistant(transcript),
    function_results: pendingResults,
  });
  await emit(session_id, { type: 'agent_end', messages: transcript });

  return {
    session_id,
    pi_session_id: record.pi_session_id,
    result: resultText,
    stop_reason: stopReason,
    is_error: isError,
    num_turns: record.turns,
    total_cost_usd: record.total_cost_usd,
    usage: record.usage,
  };
}

export function register(iii: ISdk, getCfg: () => Config, emit: Emit, emitRaw: Emit): void {
  iii.registerFunction(
    'pi::run',
    async (payload: unknown) =>
      executeRun(iii, getCfg(), emit, emitRaw, RunPayloadSchema.parse(payload ?? {})),
    {
      description:
        'Run one Pi coding-agent turn and wait for the result. Accepts `prompt` or a `messages` array; streams raw Pi events onto pi::events, AgentEvent frames onto agent::events, and returns {session_id, result, usage, total_cost_usd}.',
      request_format: RUN_REQUEST_FORMAT,
      response_format: RUN_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'pi::start',
    async (payload: unknown) => {
      const parsed = RunPayloadSchema.parse(payload ?? {});
      const session_id = parsed.session_id ?? randomUUID();
      // executeRun reserves the live slot synchronously, so a prior start for
      // this session is already visible here: report busy instead of a false
      // started: true. Cannot await executeRun — start must return immediately.
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
          console.error(`pi::start background run failed for ${session_id}: ${String(err)}`);
          await markSessionError(iii, session_id);
        },
      );
      return { session_id, started: true };
    },
    {
      description:
        'Start a Pi turn and return immediately; watch agent::events (group_id = session_id) for progress and turn_end.',
      request_format: RUN_REQUEST_FORMAT,
      response_format: START_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'pi::steer',
    async (payload: unknown) => {
      const { session_id, prompt } = SteerPayloadSchema.parse(payload ?? {});
      const run = live.get(session_id);
      if (!run?.session) return { session_id, steered: false, reason: 'no live run' };
      await run.session.steer(prompt);
      return { session_id, steered: true };
    },
    {
      description:
        'Inject a steering instruction into a live Pi run; applied after the current tool calls finish.',
      request_format: STEER_REQUEST_FORMAT,
      response_format: STEER_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'pi::follow_up',
    async (payload: unknown) => {
      const { session_id, prompt } = SteerPayloadSchema.parse(payload ?? {});
      const run = live.get(session_id);
      if (!run?.session) return { session_id, queued: false, reason: 'no live run' };
      await run.session.followUp(prompt);
      return { session_id, queued: true };
    },
    {
      description:
        'Queue a follow-up message for a live Pi run; processed after the agent would otherwise stop.',
      request_format: STEER_REQUEST_FORMAT,
      response_format: FOLLOWUP_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'pi::stop',
    async (payload: unknown) => {
      const { session_id } = SessionIdSchema.parse(payload ?? {});
      const run = live.get(session_id);
      if (!run?.session) return { session_id, stopped: false, reason: 'no live run' };
      await run.session.abort();
      return { session_id, stopped: true };
    },
    {
      description: 'Interrupt a live Pi run for a session.',
      request_format: SESSION_ID_FORMAT,
      response_format: STOP_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'pi::status',
    async (payload: unknown) => {
      const { session_id } = SessionIdSchema.parse(payload ?? {});
      const record = await loadSession(iii, session_id);
      return { session_id, live: live.has(session_id), record };
    },
    {
      description: 'Point-in-time status of a Pi session.',
      request_format: SESSION_ID_FORMAT,
      response_format: STATUS_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction('pi::sessions::list', async () => ({ sessions: await listSessions(iii) }), {
    description: 'List every Pi session this worker has run.',
    request_format: { type: 'object', properties: {} },
    response_format: SESSIONS_RESPONSE_FORMAT,
  });

  iii.registerFunction(
    'run::start_and_wait',
    async (payload: unknown) =>
      executeRun(iii, getCfg(), emit, emitRaw, RunPayloadSchema.parse(payload ?? {})),
    {
      description:
        'Alias for pi::run under the shared agent entrypoint: run a turn for {session_id, messages} and return when it ends.',
      request_format: RUN_REQUEST_FORMAT,
      response_format: RUN_RESPONSE_FORMAT,
    },
  );
}
