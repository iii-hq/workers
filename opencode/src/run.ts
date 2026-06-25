/**
 * opencode::* function registrations. `opencode::run` spawns
 * `opencode run --format json`, parses the JSON event stream line by line,
 * translates it onto agent::events, mirrors it raw onto opencode::events, and
 * returns the final result with token usage and cost. Accepts a bare `prompt`
 * or the shared agent entrypoint shape (`messages`), so anything that drives
 * `run::start_and_wait` can drive OpenCode unchanged.
 */

import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { createInterface } from 'node:readline';
import type { ISdk } from 'iii-sdk';
import { z } from 'zod';
import type { Config } from './config.js';
import type { Emit } from './events.js';
import { III_CONTEXT_PROMPT } from './iii-prompt.js';
import {
  addUsage,
  makeAssistantMessage,
  makeFunctionResult,
  mapToolOutput,
  type OpencodeEvent,
  toolFunctionId,
} from './map.js';
import { listSessions, loadSession, saveSession } from './state.js';
import type { ContentBlock, FunctionResultMessage, SessionRecord, Usage } from './types.js';

const ContentBlockSchema = z.object({ type: z.string() }).passthrough();
const MessageSchema = z.object({
  role: z.string(),
  content: z.union([z.string(), z.array(ContentBlockSchema)]),
});

const RunPayloadSchema = z.object({
  session_id: z
    .string()
    .optional()
    .describe('iii session id; reuse to resume the same OpenCode conversation'),
  prompt: z.string().optional().describe('The user prompt for this turn'),
  messages: z
    .array(MessageSchema)
    .optional()
    .describe(
      'Alternative to prompt: role/content messages; the last user entry becomes the prompt',
    ),
  model: z.string().optional().describe('Model as "provider/model"; empty = OpenCode default'),
  cwd: z.string().optional().describe('Working directory the turn runs in'),
  agent: z.string().optional().describe('OpenCode agent to use'),
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
  session_id: z.string().describe('iii session id returned by opencode::run / opencode::start'),
});

function jsonSchema(schema: z.ZodType): Record<string, unknown> {
  const out = z.toJSONSchema(schema) as Record<string, unknown>;
  delete out.$schema;
  return out;
}

const UsageSchema = z.object({
  input_tokens: z.number(),
  output_tokens: z.number(),
  cache_read_tokens: z.number().optional(),
  cache_write_tokens: z.number().optional(),
  reasoning_tokens: z.number().optional(),
});
const SessionRecordSchema = z.object({
  session_id: z.string(),
  opencode_session_id: z.string().nullable(),
  cwd: z.string(),
  model: z.string(),
  status: z.enum(['working', 'done', 'error']),
  turns: z.number(),
  total_cost_usd: z.number(),
  usage: UsageSchema.nullable(),
  updated_at_ms: z.number(),
});

const RUN_REQUEST_FORMAT = jsonSchema(RunPayloadSchema);
const SESSION_ID_FORMAT = jsonSchema(SessionIdSchema);
const RUN_RESPONSE_FORMAT = jsonSchema(
  z.object({
    session_id: z.string(),
    opencode_session_id: z.string().nullable().optional(),
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
  z.object({ session_id: z.string(), started: z.boolean() }),
);
const STOP_RESPONSE_FORMAT = jsonSchema(
  z.object({ session_id: z.string(), stopped: z.boolean(), reason: z.string().optional() }),
);
const STATUS_RESPONSE_FORMAT = jsonSchema(
  z.object({ session_id: z.string(), live: z.boolean(), record: SessionRecordSchema.nullable() }),
);
const SESSIONS_RESPONSE_FORMAT = jsonSchema(z.object({ sessions: z.array(SessionRecordSchema) }));

type LiveRun = { kill: () => void };
const live = new Map<string, LiveRun>();

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
  if (!last) throw new Error('opencode::run requires `prompt` or a user message in `messages`');
  if (typeof last.content === 'string') return last.content;
  return last.content
    .map((b) => ('text' in b && typeof b.text === 'string' ? b.text : ''))
    .filter(Boolean)
    .join('\n');
}

export function buildArgs(
  payload: RunPayload,
  cfg: Config,
  prompt: string,
  resumeId: string | null,
): string[] {
  const d = cfg.defaults;
  const model = payload.model ?? d.model;
  const cwd = payload.cwd ?? d.cwd;
  const agent = payload.agent ?? d.agent;
  const args = ['run', '--format', 'json'];
  if (resumeId) args.push('--session', resumeId);
  if (model) args.push('--model', model);
  if (agent) args.push('--agent', agent);
  if (cwd) args.push('--dir', cwd);
  args.push(prompt);
  return args;
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
  if (live.has(session_id)) {
    return { session_id, busy: true, reason: 'a run is already active for this session' };
  }
  const handle: LiveRun = { kill: () => {} };
  live.set(session_id, handle);
  try {
    return await runReserved(iii, cfg, emit, emitRaw, payload, session_id, prompt, handle);
  } finally {
    if (live.get(session_id) === handle) live.delete(session_id);
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
  handle: LiveRun,
): Promise<Record<string, unknown>> {
  const prior = await loadSession(iii, session_id);
  const d = cfg.defaults;
  const record: SessionRecord = prior ?? {
    session_id,
    opencode_session_id: null,
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

  const iiiContext = payload.iii_context ?? cfg.iii_context;
  const promptText =
    iiiContext && !prior?.opencode_session_id
      ? `${III_CONTEXT_PROMPT}\n\n---\n\n${prompt}`
      : prompt;

  const args = buildArgs(payload, cfg, promptText, prior?.opencode_session_id ?? null);
  const bin = cfg.opencode_executable || 'opencode';
  // stdin must be closed: opencode reads the prompt from argv, but an open
  // stdin pipe (spawn's default) makes it block waiting for input headlessly.
  const child = spawn(bin, args, {
    cwd: record.cwd || undefined,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  handle.kill = () => child.kill('SIGTERM');

  record.status = 'working';
  record.updated_at_ms = Date.now();
  await saveSession(iii, record);

  const transcript: ContentBlock[] = [];
  const pendingResults: FunctionResultMessage[] = [];
  let usage: Usage | null = null;
  let cost = 0;
  let resultText = '';
  let stopReason = 'end';
  let isError = false;

  const handleEvent = async (ev: OpencodeEvent): Promise<void> => {
    await emitRaw(session_id, ev);
    const part = (ev.part ?? {}) as Record<string, unknown>;
    if (ev.sessionID && !record.opencode_session_id) {
      record.opencode_session_id = String(ev.sessionID);
    }
    if (ev.type === 'text') {
      const text = typeof part.text === 'string' ? part.text : '';
      if (!text) return;
      resultText = text;
      transcript.push({ type: 'text', text });
      const assistant = makeAssistantMessage([{ type: 'text', text }], record.model, usage);
      await emit(session_id, { type: 'message_complete', message: assistant });
    } else if (ev.type === 'tool_use') {
      const callId = String(part.callID ?? randomUUID());
      const function_id = toolFunctionId(String(part.tool ?? 'unknown'));
      const st = (part.state ?? {}) as Record<string, unknown>;
      await emit(session_id, {
        type: 'function_execution_start',
        function_call_id: callId,
        function_id,
        args: st.input ?? {},
      });
      const content = mapToolOutput(st.output);
      const meta = (st.metadata ?? {}) as Record<string, unknown>;
      const toolErr = st.status === 'error' || (typeof meta.exit === 'number' && meta.exit !== 0);
      const fr = makeFunctionResult(callId, function_id, content, toolErr);
      pendingResults.push(fr);
      await emit(session_id, {
        type: 'function_execution_end',
        function_call_id: callId,
        function_id,
        result: { content, details: meta },
        is_error: toolErr,
        duration_ms: 0,
      });
    } else if (ev.type === 'step_finish') {
      usage = addUsage(usage, part.tokens);
      if (typeof part.cost === 'number') cost += part.cost;
    }
  };

  try {
    await new Promise<void>((resolve, reject) => {
      const rl = createInterface({ input: child.stdout });
      let chain: Promise<void> = Promise.resolve();
      rl.on('line', (line) => {
        const trimmed = line.trim();
        if (!trimmed) return;
        let ev: OpencodeEvent;
        try {
          ev = JSON.parse(trimmed);
        } catch {
          return;
        }
        chain = chain
          .then(() => handleEvent(ev))
          .catch((e) => {
            console.warn(`opencode event handling failed: ${String(e)}`);
          });
      });
      let stderr = '';
      child.stderr.on('data', (d) => {
        // Keep only the latest 8 KiB so a chatty child can't grow this
        // unbounded (slicing each chunk alone would still accumulate).
        stderr = (stderr + String(d)).slice(-8192);
      });
      child.on('error', reject);
      child.on('close', (code) => {
        chain.then(() => {
          if (code !== 0) {
            isError = true;
            stopReason = 'error';
            if (!resultText) resultText = stderr.trim() || `opencode exited ${code}`;
          }
          resolve();
        });
      });
    });
  } catch (err) {
    isError = true;
    stopReason = 'error';
    resultText = String(err);
  }

  record.turns += 1;
  record.usage = usage;
  record.total_cost_usd += cost;
  record.status = isError ? 'error' : 'done';
  record.updated_at_ms = Date.now();
  await saveSession(iii, record);

  const finalMessage = makeAssistantMessage(
    transcript.length ? transcript : [{ type: 'text', text: resultText }],
    record.model,
    usage,
    stopReason,
  );
  await emit(session_id, {
    type: 'turn_end',
    message: finalMessage,
    function_results: pendingResults,
  });
  await emit(session_id, { type: 'agent_end', messages: [finalMessage] });

  const envelope: Record<string, unknown> = {
    session_id,
    opencode_session_id: record.opencode_session_id,
    result: resultText,
    stop_reason: stopReason,
    is_error: isError,
    num_turns: record.turns,
    total_cost_usd: record.total_cost_usd,
    usage,
  };
  return envelope;
}

export function register(iii: ISdk, getCfg: () => Config, emit: Emit, emitRaw: Emit): void {
  iii.registerFunction(
    'opencode::run',
    async (payload: unknown) =>
      executeRun(iii, getCfg(), emit, emitRaw, RunPayloadSchema.parse(payload ?? {})),
    {
      description:
        'Run one OpenCode turn and wait for the result. Accepts `prompt` or a `messages` array; streams raw OpenCode JSON events onto opencode::events, AgentEvent frames onto agent::events, and returns {session_id, result, usage, total_cost_usd}.',
      request_format: RUN_REQUEST_FORMAT,
      response_format: RUN_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'opencode::start',
    async (payload: unknown) => {
      const parsed = RunPayloadSchema.parse(payload ?? {});
      const session_id = parsed.session_id ?? randomUUID();
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
          console.error(`opencode::start background run failed for ${session_id}: ${String(err)}`);
          await markSessionError(iii, session_id);
        },
      );
      return { session_id, started: true };
    },
    {
      description:
        'Start an OpenCode turn and return immediately; watch agent::events (group_id = session_id) for progress and turn_end.',
      request_format: RUN_REQUEST_FORMAT,
      response_format: START_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'opencode::stop',
    async (payload: unknown) => {
      const { session_id } = SessionIdSchema.parse(payload ?? {});
      const run = live.get(session_id);
      if (!run) return { session_id, stopped: false, reason: 'no live run' };
      run.kill();
      return { session_id, stopped: true };
    },
    {
      description: 'Interrupt a live OpenCode run for a session.',
      request_format: SESSION_ID_FORMAT,
      response_format: STOP_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'opencode::status',
    async (payload: unknown) => {
      const { session_id } = SessionIdSchema.parse(payload ?? {});
      const record = await loadSession(iii, session_id);
      return { session_id, live: live.has(session_id), record };
    },
    {
      description: 'Point-in-time status of an OpenCode session.',
      request_format: SESSION_ID_FORMAT,
      response_format: STATUS_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'opencode::sessions::list',
    async () => ({ sessions: await listSessions(iii) }),
    {
      description: 'List every OpenCode session this worker has run.',
      request_format: { type: 'object', properties: {} },
      response_format: SESSIONS_RESPONSE_FORMAT,
    },
  );

  iii.registerFunction(
    'run::start_and_wait',
    async (payload: unknown) =>
      executeRun(iii, getCfg(), emit, emitRaw, RunPayloadSchema.parse(payload ?? {})),
    {
      description:
        'Alias for opencode::run under the shared agent entrypoint: run a turn for {session_id, messages} and return when it ends.',
      request_format: RUN_REQUEST_FORMAT,
      response_format: RUN_RESPONSE_FORMAT,
    },
  );
}
