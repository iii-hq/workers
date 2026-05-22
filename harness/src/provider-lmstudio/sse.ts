// Kept separate from provider-openai / provider-kimi so LM Studio-specific
// quirks (cold-load timeouts, GGUF-specific finish reasons) can land without
// coupling the providers.

import type { AssistantMessage } from '../types/agent-message.js';
import type { ContentBlock } from '../types/content.js';
import type { AssistantMessageEvent, ErrorKind, StopReason, Usage } from '../types/stream-event.js';

type PartialToolCall = { id: string; function_id: string; args_json: string };

export type PartialState = {
  text: string;
  /**
   * Accumulated reasoning content from thinking-mode models served by
   * LM Studio (qwen3-thinking, glm-thinking, deepseek-r1, etc.). They
   * stream reasoning tokens on `delta.reasoning_content` using the same
   * convention as Moonshot's Kimi K2. Persisted as a `thinking`
   * ContentBlock so subsequent requests can echo it back via
   * `reasoning_content` — required by some templates when thinking is
   * enabled and the assistant turn carries tool_calls.
   */
  reasoning_text: string;
  tool_calls: PartialToolCall[];
  usage: Usage;
  stop_reason: StopReason;
  /**
   * Set true the first time we observe a chunk carrying a non-null
   * `finish_reason`. Used by stream.ts to distinguish a legitimate
   * stream end ("LM Studio said stop") from an abrupt EOF ("connection
   * dropped before LM Studio finished"). Without this, every silent
   * drop falls through as `stop_reason='end'` and the user sees a
   * truncated reply with no error indication.
   */
  saw_finish_reason: boolean;
};

export function emptyPartial(): PartialState {
  return {
    text: '',
    reasoning_text: '',
    tool_calls: [],
    usage: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
    stop_reason: 'end',
    saw_finish_reason: false,
  };
}

function buildContent(state: PartialState): ContentBlock[] {
  const out: ContentBlock[] = [];
  // Thinking goes FIRST so the persisted content order matches what the
  // model emitted: think → answer / tool_call. Wire-messages re-emits
  // it as `reasoning_content` on the next request.
  if (state.reasoning_text.length > 0) {
    out.push({ type: 'thinking', text: state.reasoning_text });
  }
  if (state.text.length > 0) out.push({ type: 'text', text: state.text });
  for (const tc of state.tool_calls) {
    if (tc.function_id.length === 0) continue;
    let args: unknown = {};
    if (tc.args_json.length > 0) {
      try {
        args = JSON.parse(tc.args_json);
      } catch {
        args = null;
      }
    }
    out.push({ type: 'function_call', id: tc.id, function_id: tc.function_id, arguments: args });
  }
  return out;
}

export function buildPartial(
  state: PartialState,
  model: string,
  provider: string,
): AssistantMessage {
  return {
    role: 'assistant',
    content: buildContent(state),
    stop_reason: state.stop_reason,
    error_message: null,
    error_kind: null,
    usage: state.usage,
    model,
    provider,
    timestamp: Date.now(),
  };
}

export function buildFinal(state: PartialState, model: string, provider: string): AssistantMessage {
  return buildPartial(state, model, provider);
}

export function mapFinishReason(s: string): StopReason {
  if (s === 'stop') return 'end';
  if (s === 'length') return 'length';
  if (s === 'tool_calls' || s === 'function_call') return 'function_call';
  return 'end';
}

export function mergeUsage(usage: Record<string, unknown>, into: Usage): void {
  const num = (k: string) => (typeof usage[k] === 'number' ? (usage[k] as number) : 0);
  into.input = (into.input ?? 0) + num('prompt_tokens') + num('input_tokens');
  into.output = (into.output ?? 0) + num('completion_tokens') + num('output_tokens');
  for (const parent of ['prompt_tokens_details', 'input_tokens_details']) {
    const d = usage[parent] as Record<string, unknown> | undefined;
    if (d && typeof d.cached_tokens === 'number') {
      into.cache_read = (into.cache_read ?? 0) + d.cached_tokens;
    }
  }
}

/**
 * Patterns that indicate an LM Studio model failed to load (either
 * because the user never loaded it, JIT-load crashed, or the GGUF
 * couldn't initialize). Centralized so the classifier and the
 * error-message formatter agree on what counts as a load failure, and
 * so the auto-load retry in `stream.ts` can key off the same set.
 */
const LOAD_FAILURE_PATTERN =
  /no model (is )?loaded|model not found|please load|model has crashed|model_load_failed|failed to load (?:llm|model)|exit code/i;

export function isLoadFailureMessage(message: string): boolean {
  return LOAD_FAILURE_PATTERN.test(message);
}

export function classifyLmstudioError(message: string, status?: number): ErrorKind {
  // Localhost LM Studio rarely returns 401/403, but corporate proxies and
  // authenticated deployments can — keep the mapping so downstream retry
  // logic does the right thing.
  if (status === 401 || status === 403) return 'auth_expired';
  if (status === 429) return 'rate_limited';
  if (status && status >= 500) return 'transient';
  // Load failures cover everything the user can fix by loading the
  // right model: "no model loaded", "model not found", "please load",
  // and the post-JIT crash shapes ("The model has crashed", "Failed to
  // load LLM …", "Exit code: null"). All map to transient so the
  // orchestrator retries and the UI doesn't show a hard dead-end.
  if (isLoadFailureMessage(message)) return 'transient';
  if (/context length|too many tokens/i.test(message)) return 'context_overflow';
  return 'permanent';
}

/**
 * Prepend a user-actionable hint when the message is a load-failure
 * shape. Keeps the original wire text intact (after the hint) so users
 * and logs still see exactly what LM Studio said.
 */
export function formatLmstudioError(message: string): string {
  if (!isLoadFailureMessage(message)) return message;
  return (
    `LM Studio could not load the model. Try loading it first via ` +
    `\`provider::lmstudio::load_model\`, or pick a different model in the picker. ` +
    `Original error: ${message}`
  );
}

export function syntheticErrorEvent(
  message: string,
  model: string,
  provider: string,
  error_kind: ErrorKind = 'transient',
): AssistantMessageEvent {
  const formatted = formatLmstudioError(message);
  // Carry the formatted text ONLY in `error_message` (which the UI's
  // translate layer routes through the `stop-reason` notice channel).
  // Do NOT inject it as a `text` ContentBlock — pre-fix that meant a
  // malicious LM Studio backend or MITM could inject "tool approved"
  // lines or attacker-controlled markup into the assistant message
  // stream, where it would be persisted and re-fed to the next
  // provider call as trusted context (a prompt-injection vector via
  // the local model server's error channel).
  const final: AssistantMessage = {
    role: 'assistant',
    content: [],
    stop_reason: 'error',
    error_message: formatted,
    error_kind,
    usage: null,
    model,
    provider,
    timestamp: Date.now(),
  };
  return { type: 'error', error: final };
}

/**
 * Extract a human-readable error message from an SSE error chunk. LM
 * Studio (and other OpenAI-compatible servers) send a JSON chunk shaped
 * `{"error": {"message": "...", ...}}` or `{"error": "..."}` when prompt
 * rendering or generation fails mid-stream after the HTTP 200 was
 * already committed. Returns null when the chunk has no error.
 */
export function extractErrorMessage(chunk: Record<string, unknown>): string | null {
  const err = chunk.error;
  if (!err) return null;
  if (typeof err === 'string' && err.length > 0) return err;
  if (typeof err === 'object') {
    const obj = err as Record<string, unknown>;
    if (typeof obj.message === 'string' && obj.message.length > 0) return obj.message;
    // Some servers nest the message under .error.error_message or .error.detail
    if (typeof obj.error_message === 'string' && obj.error_message.length > 0) {
      return obj.error_message;
    }
    if (typeof obj.detail === 'string' && obj.detail.length > 0) return obj.detail;
  }
  return null;
}

export function handleChunk(
  chunk: Record<string, unknown>,
  state: PartialState,
  model: string,
  provider: string,
): AssistantMessageEvent[] {
  const events: AssistantMessageEvent[] = [];

  // SSE error chunks: LM Studio surfaces template-render errors, OOM,
  // and other mid-stream failures as a JSON chunk with an `error` field
  // and no `choices`. Without this branch we'd ignore the chunk (no
  // choices) and later fall through with a generic "stream closed
  // mid-response" message — losing the specific server-side reason.
  const errMsg = extractErrorMessage(chunk);
  if (errMsg) {
    state.stop_reason = 'error';
    state.saw_finish_reason = true; // prevent the EOF-without-finish guard
    return [syntheticErrorEvent(errMsg, model, provider, classifyLmstudioError(errMsg))];
  }

  const usage = chunk.usage as Record<string, unknown> | undefined;
  if (usage) mergeUsage(usage, state.usage);
  const choices = chunk.choices;
  if (!Array.isArray(choices) || choices.length === 0) return events;
  const choice = choices[0] as Record<string, unknown>;
  const finish = typeof choice.finish_reason === 'string' ? choice.finish_reason : null;
  if (finish) {
    state.stop_reason = mapFinishReason(finish);
    state.saw_finish_reason = true;
  }
  const delta = choice.delta as Record<string, unknown> | undefined;
  if (!delta) return events;

  // Reasoning tokens — thinking-mode models served by LM Studio (qwen3,
  // glm, deepseek-r1, …) stream these on `delta.reasoning_content`
  // BEFORE any content/tool_calls. Surface as thinking_* events and
  // persist so the round-trip carries them back via `reasoning_content`.
  if (typeof delta.reasoning_content === 'string' && delta.reasoning_content.length > 0) {
    if (state.reasoning_text.length === 0) {
      events.push({ type: 'thinking_start', partial: buildPartial(state, model, provider) });
    }
    state.reasoning_text += delta.reasoning_content;
    events.push({
      type: 'thinking_delta',
      partial: buildPartial(state, model, provider),
      delta: delta.reasoning_content,
    });
  }

  if (typeof delta.content === 'string' && delta.content.length > 0) {
    if (state.text.length === 0) {
      events.push({ type: 'text_start', partial: buildPartial(state, model, provider) });
    }
    state.text += delta.content;
    events.push({
      type: 'text_delta',
      partial: buildPartial(state, model, provider),
      delta: delta.content,
    });
  }

  const tool_calls = delta.tool_calls;
  if (Array.isArray(tool_calls)) {
    for (const tc of tool_calls) {
      if (!tc || typeof tc !== 'object') continue;
      const tcObj = tc as Record<string, unknown>;
      const rawIndex = typeof tcObj.index === 'number' ? tcObj.index : 0;
      // Reject attacker-controlled indices that would force unbounded
      // allocation. Without this guard, a hostile (or buggy) LM Studio
      // backend can DoS the worker by sending `{"index": 1e9}` in a
      // delta.tool_calls entry — the while-loop below would allocate
      // billions of slots. 256 is well above any realistic tool-call
      // fan-out from a single model turn.
      if (!Number.isInteger(rawIndex) || rawIndex < 0 || rawIndex > 256) {
        continue;
      }
      const index = rawIndex;
      while (state.tool_calls.length <= index) {
        state.tool_calls.push({ id: '', function_id: '', args_json: '' });
      }
      const entry = state.tool_calls[index];
      if (!entry) continue;
      if (typeof tcObj.id === 'string' && tcObj.id.length > 0) entry.id = tcObj.id;
      const fn = tcObj.function as Record<string, unknown> | undefined;
      if (fn) {
        if (typeof fn.name === 'string' && fn.name.length > 0) entry.function_id = fn.name;
        if (typeof fn.arguments === 'string') {
          entry.args_json += fn.arguments;
          events.push({
            type: 'functioncall_delta',
            partial: buildPartial(state, model, provider),
            delta: fn.arguments,
          });
        }
      }
    }
  }
  return events;
}
