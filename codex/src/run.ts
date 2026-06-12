/**
 * codex::* function registrations. `codex::run` accepts either a bare
 * `prompt` string or a `messages` array (the same input contract as
 * `run::start_and_wait`), so the acp worker can drive Codex with
 * `--brain-fn codex::run` and any bus caller gets the same shape as the
 * claude-code worker.
 */

import { randomUUID } from 'node:crypto';
import { Codex, type ThreadOptions } from '@openai/codex-sdk';
import type { ISdk } from 'iii-sdk';
import { z } from 'zod';
import type { Config } from './config.js';
import type { Emit } from './events.js';
import { III_CONTEXT_PROMPT } from './iii-prompt.js';
import {
  argsForItem,
  type CodexItem,
  functionIdForItem,
  isErrorItem,
  isExecItem,
  lastAssistant,
  makeAssistantMessage,
  makeFunctionResult,
  mapUsage,
  resultContentForItem,
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
    .describe('iii session id; reuse to resume the same Codex thread'),
  prompt: z.string().optional().describe('The user prompt for this turn'),
  messages: z
    .array(MessageSchema)
    .optional()
    .describe(
      'Alternative to prompt: role/content messages; the last user entry becomes the prompt',
    ),
  model: z.string().optional().describe('Model id; empty = Codex default'),
  cwd: z.string().optional().describe('Working directory the turn runs in'),
  sandbox_mode: z
    .enum(['read-only', 'workspace-write', 'danger-full-access'])
    .optional()
    .describe('Codex sandbox mode for this turn'),
  approval_policy: z
    .enum(['never', 'on-request', 'on-failure', 'untrusted'])
    .optional()
    .describe('Codex approval policy; headless callers use never'),
  reasoning_effort: z
    .enum(['minimal', 'low', 'medium', 'high', 'xhigh'])
    .optional()
    .describe('Model reasoning effort'),
  skip_git_repo_check: z.boolean().optional().describe('Allow running outside a git repository'),
  iii_context: z
    .boolean()
    .optional()
    .describe(
      'Inject the iii runtime discovery prompt (engine catalog via the iii CLI) on new threads',
    ),
  output_schema: z
    .record(z.string(), z.unknown())
    .optional()
    .describe('JSON schema for structured final output'),
  images: z.array(z.string()).optional().describe('Paths to local images attached to the prompt'),
  codex_config: z
    .record(z.string(), z.unknown())
    .optional()
    .describe(
      'Codex config.toml overrides for this turn (flattened to --config key=value), e.g. mcp_servers, model_providers, profiles',
    ),
  /** Raw Codex SDK ThreadOptions, spread over everything the worker
   *  derives — the pure pass-through (camelCase as in the SDK). */
  options: z
    .record(z.string(), z.unknown())
    .optional()
    .describe(
      'Raw Codex SDK ThreadOptions forwarded verbatim (camelCase), e.g. networkAccessEnabled, webSearchMode, additionalDirectories',
    ),
});

export type RunPayload = z.infer<typeof RunPayloadSchema>;
export { RunPayloadSchema };

const SessionIdSchema = z.object({
  session_id: z.string().describe('iii session id returned by codex::run / codex::start'),
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
  if (!last) throw new Error('codex::run requires `prompt` or a user message in `messages`');
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
  // one live run per session: the in-process handle is what codex::stop
  // targets, so a second concurrent run would clobber it and race the record
  if (live.has(session_id)) {
    return { session_id, busy: true, reason: 'a run is already active for this session' };
  }
  const prior = await loadSession(iii, session_id);
  const d = cfg.defaults;

  const record: SessionRecord = prior ?? {
    session_id,
    codex_thread_id: null,
    cwd: payload.cwd ?? d.cwd,
    model: payload.model ?? d.model,
    status: 'working',
    turns: 0,
    usage: null,
    updated_at_ms: Date.now(),
  };
  if (payload.cwd) record.cwd = payload.cwd;
  if (payload.model) record.model = payload.model;

  const reasoning = payload.reasoning_effort ?? (d.reasoning_effort || undefined);
  const threadOptions: ThreadOptions = {
    ...(record.model ? { model: record.model } : {}),
    ...(record.cwd ? { workingDirectory: record.cwd } : {}),
    sandboxMode: payload.sandbox_mode ?? d.sandbox_mode,
    approvalPolicy: payload.approval_policy ?? d.approval_policy,
    skipGitRepoCheck: payload.skip_git_repo_check ?? d.skip_git_repo_check,
    ...(reasoning ? { modelReasoningEffort: reasoning } : {}),
    ...(payload.options as Partial<ThreadOptions> | undefined),
  };

  const iiiContext = payload.iii_context ?? cfg.iii_context;
  const callerConfig = (payload.codex_config ?? {}) as Record<string, unknown>;
  const codexConfig = {
    // developer_instructions is a per-turn developer message in Codex's
    // turn context; a caller-supplied value wins over the iii block
    ...(iiiContext && callerConfig.developer_instructions === undefined
      ? { developer_instructions: III_CONTEXT_PROMPT }
      : {}),
    ...callerConfig,
  };
  const codex = new Codex({
    ...(cfg.codex_executable ? { codexPathOverride: cfg.codex_executable } : {}),
    ...(cfg.base_url ? { baseUrl: cfg.base_url } : {}),
    ...(Object.keys(codexConfig).length ? { config: codexConfig as never } : {}),
  });
  const thread = prior?.codex_thread_id
    ? codex.resumeThread(prior.codex_thread_id, threadOptions)
    : codex.startThread(threadOptions);

  const abort = new AbortController();
  const handle: LiveRun = { interrupt: async () => abort.abort() };
  live.set(session_id, handle);

  // persist `working` only once the thread + live handle exist, so a throw
  // during Codex construction never leaves the record stuck in `working`
  record.status = 'working';
  record.updated_at_ms = Date.now();
  await saveSession(iii, record);

  const transcript: AgentMessage[] = [];
  const pendingResults: FunctionResultMessage[] = [];
  const started = new Map<string, { function_id: string; started_at: number }>();
  let usage: SessionRecord['usage'] = null;
  let resultText = '';
  let stopReason = 'end';
  let isError = false;

  try {
    const input = payload.images?.length
      ? [
          { type: 'text' as const, text: prompt },
          ...payload.images.map((path) => ({ type: 'local_image' as const, path })),
        ]
      : prompt;
    const { events } = await thread.runStreamed(input, {
      signal: abort.signal,
      ...(payload.output_schema ? { outputSchema: payload.output_schema } : {}),
    });
    for await (const event of events) {
      await emitRaw(session_id, event);
      if (event.type === 'thread.started') {
        record.codex_thread_id = event.thread_id;
        await saveSession(iii, record);
      } else if (event.type === 'item.started' || event.type === 'item.updated') {
        const item = event.item as CodexItem;
        if (isExecItem(item) && !started.has(item.id)) {
          const function_id = functionIdForItem(item);
          started.set(item.id, { function_id, started_at: Date.now() });
          await emit(session_id, {
            type: 'function_execution_start',
            function_call_id: item.id,
            function_id,
            args: argsForItem(item),
          });
        }
      } else if (event.type === 'item.completed') {
        const item = event.item as CodexItem;
        if (item.type === 'agent_message' || item.type === 'reasoning') {
          const block =
            item.type === 'agent_message'
              ? ({ type: 'text', text: item.text ?? '' } as const)
              : ({ type: 'thinking', text: item.text ?? '' } as const);
          const assistant = makeAssistantMessage([block], record.model, null);
          transcript.push(assistant);
          await emit(session_id, { type: 'message_complete', message: assistant });
          if (item.type === 'agent_message') resultText = item.text ?? '';
        } else if (isExecItem(item)) {
          const function_id = functionIdForItem(item);
          const begun = started.get(item.id);
          if (!begun) {
            await emit(session_id, {
              type: 'function_execution_start',
              function_call_id: item.id,
              function_id,
              args: argsForItem(item),
            });
          }
          const content = resultContentForItem(item);
          const fr = makeFunctionResult(item.id, function_id, content, isErrorItem(item));
          transcript.push(fr);
          pendingResults.push(fr);
          await emit(session_id, {
            type: 'function_execution_end',
            function_call_id: item.id,
            function_id,
            result: { content, details: null },
            is_error: fr.is_error,
            duration_ms: begun ? Date.now() - begun.started_at : 0,
          });
        }
      } else if (event.type === 'turn.completed') {
        usage = mapUsage(event.usage);
      } else if (event.type === 'turn.failed') {
        isError = true;
        stopReason = 'error';
        resultText = event.error?.message ?? 'turn failed';
      } else if (event.type === 'error') {
        isError = true;
        stopReason = 'error';
        resultText = event.message ?? 'stream error';
      }
    }
  } catch (err) {
    if (abort.signal.aborted) {
      stopReason = 'aborted';
    } else {
      isError = true;
      stopReason = 'error';
      resultText = String(err);
    }
  } finally {
    if (live.get(session_id) === handle) live.delete(session_id);
  }

  record.status = isError ? 'error' : 'done';
  record.turns += 1;
  record.usage = usage ?? record.usage;
  record.codex_thread_id = thread.id ?? record.codex_thread_id;
  record.updated_at_ms = Date.now();
  await saveSession(iii, record);

  if (transcript.length === 0) {
    transcript.push(
      makeAssistantMessage([{ type: 'text', text: resultText }], record.model, usage, stopReason),
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
    codex_thread_id: record.codex_thread_id,
    result: resultText,
    stop_reason: stopReason,
    is_error: isError,
    num_turns: record.turns,
    usage: record.usage,
  };
}

export function register(iii: ISdk, cfg: Config, emit: Emit, emitRaw: Emit): void {
  iii.registerFunction(
    'codex::run',
    async (payload: unknown) =>
      executeRun(iii, cfg, emit, emitRaw, RunPayloadSchema.parse(payload ?? {})),
    {
      description:
        'Run one Codex turn and wait for the result. Accepts `prompt` or a `messages` array plus a raw SDK `options` pass-through; streams raw Codex events onto codex::events, AgentEvent frames onto agent::events, and returns {session_id, result, usage}.',
      request_format: RUN_REQUEST_FORMAT,
    },
  );

  iii.registerFunction(
    'codex::start',
    async (payload: unknown) => {
      const parsed = RunPayloadSchema.parse(payload ?? {});
      const session_id = parsed.session_id ?? randomUUID();
      void executeRun(iii, cfg, emit, emitRaw, { ...parsed, session_id }).catch(async (err) => {
        console.error(`codex::start background run failed for ${session_id}: ${String(err)}`);
        // never leave the record stuck in `working`: a failure inside the
        // turn's own terminal save lands here, so mark it error best-effort
        await markSessionError(iii, session_id);
      });
      return { session_id, started: true };
    },
    {
      description:
        'Start a Codex turn and return immediately; watch codex::events / agent::events (group_id = session_id) for progress and turn_end.',
      request_format: RUN_REQUEST_FORMAT,
    },
  );

  iii.registerFunction(
    'codex::stop',
    async (payload: unknown) => {
      const { session_id } = SessionIdSchema.parse(payload ?? {});
      const run = live.get(session_id);
      if (!run) return { session_id, stopped: false, reason: 'no live run' };
      await run.interrupt();
      return { session_id, stopped: true };
    },
    {
      description: 'Interrupt a live Codex run for a session.',
      request_format: SESSION_ID_FORMAT,
    },
  );

  iii.registerFunction(
    'codex::status',
    async (payload: unknown) => {
      const { session_id } = SessionIdSchema.parse(payload ?? {});
      const record = await loadSession(iii, session_id);
      return { session_id, live: live.has(session_id), record };
    },
    {
      description: 'Point-in-time status of a Codex session.',
      request_format: SESSION_ID_FORMAT,
    },
  );

  iii.registerFunction(
    'codex::sessions::list',
    async () => ({ sessions: await listSessions(iii) }),
    {
      description: 'List every Codex session this worker has run.',
      request_format: { type: 'object', properties: {} },
    },
  );
}
