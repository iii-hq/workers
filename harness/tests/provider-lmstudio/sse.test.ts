import { describe, expect, it } from 'vitest';
import {
  buildPartial,
  classifyLmstudioError,
  emptyPartial,
  extractErrorMessage,
  formatLmstudioError,
  handleChunk,
  isLoadFailureMessage,
  mapFinishReason,
  mergeUsage,
  syntheticErrorEvent,
} from '../../src/provider-lmstudio/sse.js';

describe('mergeUsage (lmstudio)', () => {
  it('extracts chat-completions cached_tokens', () => {
    const u = { input: 0, output: 0, cache_read: 0, cache_write: 0 };
    mergeUsage(
      {
        prompt_tokens: 1500,
        completion_tokens: 200,
        prompt_tokens_details: { cached_tokens: 1200 },
      },
      u,
    );
    expect(u.input).toBe(1500);
    expect(u.output).toBe(200);
    expect(u.cache_read).toBe(1200);
  });
});

describe('mapFinishReason (lmstudio)', () => {
  it('maps known finish reasons', () => {
    expect(mapFinishReason('stop')).toBe('end');
    expect(mapFinishReason('length')).toBe('length');
    expect(mapFinishReason('tool_calls')).toBe('function_call');
    expect(mapFinishReason('function_call')).toBe('function_call');
  });

  it('falls back to `end` for unknown or empty finish reasons', () => {
    // LM Studio GGUF backends sometimes emit non-standard finish reasons
    // (e.g. content_filter from upstream OpenAI-compat layers). Pin the
    // fallback so a future refactor doesn't accidentally introduce a new
    // StopReason variant for unknown strings.
    expect(mapFinishReason('content_filter')).toBe('end');
    expect(mapFinishReason('')).toBe('end');
    expect(mapFinishReason('unrecognised_string')).toBe('end');
  });
});

describe('handleChunk (lmstudio)', () => {
  it('emits text_start on first content delta then text_delta', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { choices: [{ delta: { content: 'hi' } }] },
      state,
      'qwen/qwen3-4b-2507',
      'lmstudio',
    );
    expect(events.map((e) => e.type)).toEqual(['text_start', 'text_delta']);
    expect(state.text).toBe('hi');
  });

  it('accumulates tool_call arguments across chunks', () => {
    const state = emptyPartial();
    handleChunk(
      {
        choices: [
          {
            delta: {
              tool_calls: [
                { index: 0, id: 'tc1', function: { name: 'shell::exec', arguments: '{"x' } },
              ],
            },
          },
        ],
      },
      state,
      'qwen/qwen3-4b-2507',
      'lmstudio',
    );
    handleChunk(
      {
        choices: [{ delta: { tool_calls: [{ index: 0, function: { arguments: '":1}' } }] } }],
      },
      state,
      'qwen/qwen3-4b-2507',
      'lmstudio',
    );
    expect(state.tool_calls[0]?.id).toBe('tc1');
    expect(state.tool_calls[0]?.function_id).toBe('shell::exec');
    expect(state.tool_calls[0]?.args_json).toBe('{"x":1}');
  });

  it('records finish_reason as stop_reason on the partial state', () => {
    const state = emptyPartial();
    handleChunk(
      { choices: [{ finish_reason: 'tool_calls' }] },
      state,
      'qwen/qwen3-4b-2507',
      'lmstudio',
    );
    const partial = buildPartial(state, 'qwen/qwen3-4b-2507', 'lmstudio');
    expect(partial.stop_reason).toBe('function_call');
  });
});

describe('classifyLmstudioError', () => {
  it('maps 401/403 to auth_expired (corporate proxy / authenticated deploy)', () => {
    expect(classifyLmstudioError('unauthorized', 401)).toBe('auth_expired');
    expect(classifyLmstudioError('forbidden', 403)).toBe('auth_expired');
  });

  it('maps 429 to rate_limited', () => {
    expect(classifyLmstudioError('too many requests', 429)).toBe('rate_limited');
  });

  it('maps 5xx to transient', () => {
    expect(classifyLmstudioError('bad gateway', 502)).toBe('transient');
  });

  it('maps "no model loaded" 4xx to transient', () => {
    expect(
      classifyLmstudioError('No model is loaded. Please load a model first.', 404),
    ).toBe('transient');
    expect(classifyLmstudioError('model not found: foo/bar', 400)).toBe('transient');
  });

  it('maps "model has crashed" 4xx to transient (post-JIT-load crash)', () => {
    // Regression: this was the exact wire message zai-org/glm-4.7-flash
    // emitted on lmstudio when its lazy load aborted. Previously
    // classified as `permanent`, blocking any retry/recovery path.
    expect(
      classifyLmstudioError(
        'The model has crashed without additional information. (Exit code: null)',
        400,
      ),
    ).toBe('transient');
  });

  it('maps model_load_failed type marker to transient', () => {
    expect(
      classifyLmstudioError("{\"type\":\"model_load_failed\",\"message\":\"...\"}", 400),
    ).toBe('transient');
  });

  it('maps "Failed to load LLM" to transient', () => {
    expect(
      classifyLmstudioError(`Failed to load LLM 'foo/bar': Error: Failed to load model.`, 500),
    ).toBe('transient');
  });

  it('maps context length messages to context_overflow', () => {
    expect(classifyLmstudioError('context length exceeded', 400)).toBe('context_overflow');
  });

  it('defaults to permanent', () => {
    expect(classifyLmstudioError('something else', 400)).toBe('permanent');
  });
});

describe('isLoadFailureMessage', () => {
  it('matches all known load-failure shapes', () => {
    for (const msg of [
      'No model is loaded',
      'model not found',
      'please load a model',
      'The model has crashed',
      'model_load_failed',
      'Failed to load LLM foo',
      'Failed to load model',
      'Exit code: null',
    ]) {
      expect(isLoadFailureMessage(msg), `expected hit for: ${msg}`).toBe(true);
    }
  });

  it('does not match unrelated errors', () => {
    for (const msg of [
      'context length exceeded',
      'rate limited',
      'unauthorized',
      'template render failed',
    ]) {
      expect(isLoadFailureMessage(msg), `expected miss for: ${msg}`).toBe(false);
    }
  });
});

describe('formatLmstudioError', () => {
  it('prepends a load hint when the message looks like a load failure', () => {
    const out = formatLmstudioError(
      'The model has crashed without additional information. (Exit code: null)',
    );
    expect(out).toContain('LM Studio could not load the model');
    expect(out).toContain('provider::lmstudio::load_model');
    expect(out).toContain('pick a different model');
    // Original wire message must still be present after the hint.
    expect(out).toContain('The model has crashed without additional information');
  });

  it('passes through unrelated errors verbatim (no hint)', () => {
    const msg = 'context length exceeded';
    expect(formatLmstudioError(msg)).toBe(msg);
  });
});

describe('syntheticErrorEvent applies the load-failure hint', () => {
  it('puts the formatted message into error_message ONLY (not into content)', () => {
    // Security-hardened contract: the formatted text now lives only in
    // `error_message` (which the UI's translate layer surfaces via
    // `stop-reason`). Pre-fix this same string was ALSO injected as a
    // `text` ContentBlock — a malicious LM Studio backend or MITM
    // could then smuggle "tool approved" lines or markup into the
    // assistant content stream, where it would be persisted and
    // re-fed to the next provider call as trusted context. Keeping
    // content empty closes that prompt-injection sink.
    const ev = syntheticErrorEvent(
      'The model has crashed without additional information. (Exit code: null)',
      'zai-org/glm-4.7-flash',
      'lmstudio',
      'transient',
    );
    expect(ev.type).toBe('error');
    if (ev.type !== 'error') throw new Error('unreachable');
    const final = ev.error;
    expect(final.error_message).toContain('LM Studio could not load the model');
    expect(final.error_message).toContain('Original error: The model has crashed');
    expect(final.content).toEqual([]);
  });

  it('leaves non-load-failure errors unmodified', () => {
    const ev = syntheticErrorEvent('rate limited', 'm', 'lmstudio', 'rate_limited');
    if (ev.type !== 'error') throw new Error('unreachable');
    expect(ev.error.error_message).toBe('rate limited');
    expect(ev.error.content).toEqual([]);
  });
});

describe('extractErrorMessage', () => {
  it('returns the inner message from an object-shaped error', () => {
    expect(
      extractErrorMessage({
        error: { message: 'No user query found in messages.', type: 'template_error' },
      }),
    ).toBe('No user query found in messages.');
  });

  it('returns a string-shaped error verbatim', () => {
    expect(extractErrorMessage({ error: 'plain text error from server' })).toBe(
      'plain text error from server',
    );
  });

  it('falls back to .error.error_message when nested under that alias', () => {
    expect(
      extractErrorMessage({ error: { error_message: 'OOM during generation' } }),
    ).toBe('OOM during generation');
  });

  it('falls back to .error.detail (used by some llama.cpp builds)', () => {
    expect(extractErrorMessage({ error: { detail: 'context exceeded' } })).toBe(
      'context exceeded',
    );
  });

  it('returns null when no error field is present', () => {
    expect(extractErrorMessage({ choices: [{ delta: { content: 'hi' } }] })).toBeNull();
  });

  it('returns null for empty/non-stringy error payloads', () => {
    expect(extractErrorMessage({ error: '' })).toBeNull();
    expect(extractErrorMessage({ error: {} })).toBeNull();
    expect(extractErrorMessage({ error: null })).toBeNull();
  });
});

describe('handleChunk — SSE error chunks', () => {
  // LM Studio commits to HTTP 200 + SSE streaming, then on template render
  // failure / mid-stream OOM / model unload it sends a JSON chunk shaped
  // `data: {"error":{"message":"..."}}` with no `choices` and closes the
  // body. Pre-fix we silently ignored those (no choices → no events) and
  // the user saw "stream closed mid-response" instead of the real reason.
  it('surfaces an error event when the chunk carries an object-shaped error', () => {
    const state = emptyPartial();
    const events = handleChunk(
      {
        error: {
          message: 'Error rendering prompt with jinja template: "No user query found in messages."',
          type: 'invalid_request_error',
        },
      },
      state,
      'qwen/qwen3.6-35b-a3b',
      'lmstudio',
    );
    expect(events).toHaveLength(1);
    expect(events[0]?.type).toBe('error');
    const err = events[0] as Extract<(typeof events)[number], { type: 'error' }>;
    expect(err.error.stop_reason).toBe('error');
    expect(err.error.error_message).toMatch(/No user query found in messages/);
    expect(state.stop_reason).toBe('error');
    expect(state.saw_finish_reason).toBe(true);
  });

  it('classifies "No user query" errors as transient (qwen template constraint, fixable)', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { error: { message: 'No user query found in messages.' } },
      state,
      'qwen/qwen3-4b-2507',
      'lmstudio',
    );
    const err = events[0] as Extract<(typeof events)[number], { type: 'error' }>;
    // classifyLmstudioError doesn't have a specific regex for "No user
    // query" — falls through to 'permanent'. That's fine: the UI's
    // stop-reason surfacing still shows the real message. Pin the
    // value (rather than `toBeDefined`) so a future reclassification
    // (e.g. accidentally routing it through auth_expired) is caught.
    expect(err.error.error_kind).toBe('permanent');
  });

  it('classifies context-overflow error chunks as context_overflow', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { error: { message: 'Trying to keep the first 8192 tokens, context length exceeded' } },
      state,
      'm',
      'lmstudio',
    );
    const err = events[0] as Extract<(typeof events)[number], { type: 'error' }>;
    expect(err.error.error_kind).toBe('context_overflow');
  });

  it('continues normally when the chunk has choices and no error', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { choices: [{ delta: { content: 'hi' } }] },
      state,
      'm',
      'lmstudio',
    );
    expect(events.some((e) => e.type === 'error')).toBe(false);
    expect(state.text).toBe('hi');
  });
});

describe('handleChunk — thinking-mode reasoning_content (qwen3 / glm / deepseek-r1)', () => {
  // LM Studio surfaces reasoning tokens for thinking-capable GGUFs on the
  // standard OpenAI-compat `delta.reasoning_content` field — same shape
  // as Moonshot Kimi K2. We persist them as a thinking ContentBlock so
  // wire-messages can echo them back when the round-trip needs it.

  it('accumulates delta.reasoning_content and emits thinking_start + thinking_delta', () => {
    const state = emptyPartial();
    const events = handleChunk(
      { choices: [{ delta: { reasoning_content: 'reasoning…' } }] },
      state,
      'qwen/qwen3-thinking',
      'lmstudio',
    );
    expect(state.reasoning_text).toBe('reasoning…');
    expect(events.map((e) => e.type)).toEqual(['thinking_start', 'thinking_delta']);
  });

  it('persists reasoning_text as a leading thinking ContentBlock', () => {
    const state = emptyPartial();
    handleChunk(
      { choices: [{ delta: { reasoning_content: 'planning' } }] },
      state,
      'm',
      'lmstudio',
    );
    handleChunk(
      { choices: [{ delta: { content: 'answer' } }] },
      state,
      'm',
      'lmstudio',
    );
    const partial = buildPartial(state, 'm', 'lmstudio');
    expect(partial.content[0]).toEqual({ type: 'thinking', text: 'planning' });
    expect(partial.content[1]).toEqual({ type: 'text', text: 'answer' });
  });

  it('carries the thinking block alongside tool_calls in one turn', () => {
    const state = emptyPartial();
    handleChunk(
      {
        choices: [
          {
            delta: {
              reasoning_content: 'I should list files',
              tool_calls: [
                {
                  index: 0,
                  id: 'tc-1',
                  function: { name: 'shell::ls', arguments: '{"path":"/"}' },
                },
              ],
            },
          },
        ],
      },
      state,
      'qwen/qwen3-thinking',
      'lmstudio',
    );
    const partial = buildPartial(state, 'qwen/qwen3-thinking', 'lmstudio');
    const thinking = partial.content.find((c) => c.type === 'thinking') as
      | { type: 'thinking'; text: string }
      | undefined;
    expect(thinking?.text).toBe('I should list files');
    expect(partial.content.some((c) => c.type === 'function_call')).toBe(true);
  });
});
