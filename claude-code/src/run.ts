/**
 * claude::* function registrations. `claude::run` accepts either a bare
 * `prompt` string or the shared agent entrypoint shape (`messages` array of
 * role/content-block messages), so anything that can drive
 * `run::start_and_wait` — the acp worker, the console, another worker — can
 * drive Claude Code unchanged.
 */

import { randomUUID } from 'node:crypto';
import { query, type Options, type PermissionResult } from '@anthropic-ai/claude-agent-sdk';
import type { ISdk } from 'iii-sdk';
import { z } from 'zod';
import type { Config } from './config.js';
import type { Emit } from './events.js';
import { III_CONTEXT_PROMPT } from './iii-prompt.js';
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

const SessionIdSchema = z.object({
  session_id: z.string().describe('iii session id returned by claude::run / claude::start'),
});

const RUN_REQUEST_FORMAT = z.toJSONSchema(RunPayloadSchema);
const SESSION_ID_FORMAT = z.toJSONSchema(SessionIdSchema);

type LiveRun = { interrupt: () => Promise<void> };
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
  if (!last) throw new Error('claude::run requires `prompt` or a user message in `messages`');
  if (typeof last.content === 'string') return last.content;
  return last.content
    .map((b) => ('text' in b && typeof b.text === 'string' ? b.text : ''))
    .filter(Boolean)
    .join('\n');
}

function gatedCanUseTool(iii: ISdk, session_id: string) {
  return async (toolName: string, input: Record<string, unknown>): Promise<PermissionResult> => {
    try {
      const res = await iii.trigger<unknown, { decision?: string; behavior?: string }>({
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
  iii: ISdk,
  cfg: Config,
  emit: Emit,
  emitRaw: Emit,
  payload: RunPayload,
): Promise<Record<string, unknown>> {
  const session_id = payload.session_id ?? randomUUID();
  const prompt = extractPrompt(payload);
  // one live run per session: the in-process handle is what claude::stop
  // targets, so a second concurrent run would clobber it and race the record
  if (live.has(session_id)) {
    return { session_id, busy: true, reason: 'a run is already active for this session' };
  }
  const prior = await loadSession(iii, session_id);
  const d = cfg.defaults;

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
  const iiiContext = payload.iii_context ?? cfg.iii_context;
  const append = [iiiContext ? III_CONTEXT_PROMPT : '', userAppend].filter(Boolean).join('\n\n');
  const options: Options = {
    ...(record.model ? { model: record.model } : {}),
    ...(record.cwd ? { cwd: record.cwd } : {}),
    ...(prior?.claude_session_id ? { resume: prior.claude_session_id } : {}),
    maxTurns: payload.max_turns ?? d.max_turns,
    // with the approval gate on, the operator wants every tool call routed
    // through canUseTool; bypassPermissions skips it, so the gate forces a
    // mode that keeps canUseTool authoritative regardless of the caller
    permissionMode: cfg.approval_gate ? 'default' : (payload.permission_mode ?? d.permission_mode),
    allowedTools: [
      ...(payload.allowed_tools ?? d.allowed_tools),
      // the iii context teaches discovery through the CLI; the matching
      // allow rule is what lets those calls run headless
      ...(iiiContext ? ['Bash(iii *)'] : []),
    ],
    disallowedTools: payload.disallowed_tools ?? d.disallowed_tools,
    systemPrompt: payload.system_prompt
      ? payload.system_prompt
      : { type: 'preset', preset: 'claude_code', ...(append ? { append } : {}) },
    settingSources: [],
    ...(cfg.claude_executable ? { pathToClaudeCodeExecutable: cfg.claude_executable } : {}),
    ...callerOptions(payload, cfg),
    ...(cfg.approval_gate ? { canUseTool: gatedCanUseTool(iii, session_id) } : {}),
  };

  const q = query({ prompt, options });
  const handle: LiveRun = { interrupt: () => q.interrupt() };
  live.set(session_id, handle);

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

  try {
    for await (const msg of q) {
      await emitRaw(session_id, msg);
      if (msg.type === 'system' && msg.subtype === 'init') {
        record.claude_session_id = msg.session_id;
        await saveSession(iii, record);
      } else if (msg.type === 'assistant') {
        const usage = mapUsage(msg.message.usage);
        const content = mapAssistantContent(msg.message.content as never, calls);
        const assistant = makeAssistantMessage(content, msg.message.model ?? record.model, usage);
        transcript.push(assistant);
        await emit(session_id, { type: 'message_complete', message: assistant });
        for (const block of content) {
          if (block.type === 'function_call') {
            await emit(session_id, {
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
          transcript.push(fr);
          pendingResults.push(fr);
          await emit(session_id, {
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
        resultText = 'result' in msg && typeof msg.result === 'string' ? msg.result : '';
      }
    }
  } catch (err) {
    isError = true;
    stopReason = 'error';
    resultText = String(err);
  } finally {
    if (live.get(session_id) === handle) live.delete(session_id);
  }

  record.status = isError ? 'error' : 'done';
  record.updated_at_ms = Date.now();
  await saveSession(iii, record);

  if (transcript.length === 0) {
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
    claude_session_id: record.claude_session_id,
    result: resultText,
    stop_reason: stopReason,
    is_error: isError,
    num_turns: record.turns,
    total_cost_usd: record.total_cost_usd,
    usage: record.usage,
  };
}

export function register(iii: ISdk, cfg: Config, emit: Emit, emitRaw: Emit): void {
  iii.registerFunction(
    'claude::run',
    async (payload: unknown) =>
      executeRun(iii, cfg, emit, emitRaw, RunPayloadSchema.parse(payload ?? {})),
    {
      description:
        'Run one Claude Code turn and wait for the result. Accepts `prompt` or a `messages` array plus a raw SDK `options` pass-through; streams raw Claude Code messages onto claude::events, AgentEvent frames onto agent::events, and returns {session_id, result, usage, total_cost_usd}.',
      request_format: RUN_REQUEST_FORMAT,
    },
  );

  iii.registerFunction(
    'claude::start',
    async (payload: unknown) => {
      const parsed = RunPayloadSchema.parse(payload ?? {});
      const session_id = parsed.session_id ?? randomUUID();
      void executeRun(iii, cfg, emit, emitRaw, { ...parsed, session_id }).catch(async (err) => {
        console.error(`claude::start background run failed for ${session_id}: ${String(err)}`);
        // never leave the record stuck in `working`: a failure inside the
        // turn's own terminal save lands here, so mark it error best-effort
        await markSessionError(iii, session_id);
      });
      return { session_id, started: true };
    },
    {
      description:
        'Start a Claude Code turn and return immediately; watch agent::events (group_id = session_id) for progress and turn_end.',
      request_format: RUN_REQUEST_FORMAT,
    },
  );

  iii.registerFunction(
    'claude::stop',
    async (payload: unknown) => {
      const { session_id } = SessionIdSchema.parse(payload ?? {});
      const run = live.get(session_id);
      if (!run) return { session_id, stopped: false, reason: 'no live run' };
      await run.interrupt();
      return { session_id, stopped: true };
    },
    {
      description: 'Interrupt a live Claude Code run for a session.',
      request_format: SESSION_ID_FORMAT,
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
      description: 'Point-in-time status of a Claude Code session.',
      request_format: SESSION_ID_FORMAT,
    },
  );

  iii.registerFunction(
    'claude::sessions::list',
    async () => ({ sessions: await listSessions(iii) }),
    {
      description: 'List every Claude Code session this worker has run.',
      request_format: { type: 'object', properties: {} },
    },
  );

  iii.registerFunction(
    'run::start_and_wait',
    async (payload: unknown) =>
      executeRun(iii, cfg, emit, emitRaw, RunPayloadSchema.parse(payload ?? {})),
    {
      description:
        'Alias for claude::run under the shared agent entrypoint: run a turn for {session_id, messages} and return when it ends.',
      request_format: RUN_REQUEST_FORMAT,
    },
  );
}
