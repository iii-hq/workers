import { describe, expect, it, vi } from 'vitest';
import {
  estimateTokenCount,
  renderUserPrompt,
  summarizeAndAppend,
} from '../../src/context-compaction/summarize.js';
import type { AgentMessage, AssistantMessage } from '../../src/types/agent-message.js';
import type { ContentBlock } from '../../src/types/content.js';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const userMsg = (t: string): AgentMessage => ({
  role: 'user',
  content: [{ type: 'text', text: t }],
  timestamp: 0,
});

const assistantMsg = (t: string): AgentMessage => ({
  role: 'assistant',
  content: [{ type: 'text', text: t }],
  stop_reason: 'end',
  model: 'm',
  provider: 'p',
  timestamp: 0,
});

const withId = (entry_id: string, msg: AgentMessage) => ({ entry_id, message: msg });

// Builds entries large enough that selectWithEntryIds finds a non-empty head
// (needs > tailTurns=2 user turns to split off some turns into head).
const makeEntries = () => [
  withId('id-0', userMsg('user 0')),
  withId('id-1', assistantMsg('assistant 1')),
  withId('id-2', userMsg('user 2')),
  withId('id-3', assistantMsg('assistant 3')),
  withId('id-4', userMsg('user 4')),
  withId('id-5', assistantMsg('assistant 5')),
  withId('id-6', userMsg('user 6')),
  withId('id-7', assistantMsg('assistant 7')),
];

// Model stub passed to summarizeAndAppend
const testModel = {
  providerID: 'anthropic',
  modelID: 'claude-haiku-4-5',
  modelLimit: { context: 200_000, input: 200_000, output: 4_096 },
};

// ---------------------------------------------------------------------------
// Channel + ISdk mock factory
//
// streamAndCollect flow:
//   1. channel = await iii.createChannel()
//   2. channel.reader.onMessage(cb)   ← registers the callback
//   3. await iii.trigger(provider...) ← we fire cb here, inside the trigger mock
//   4. while(!terminal) drain loop
//
// We fire the channel message synchronously inside the provider trigger
// call so that `terminal` is set before the drain loop runs.
// ---------------------------------------------------------------------------

function makeDoneEvent(summary = 'the summary'): string {
  const msg: AssistantMessage = {
    role: 'assistant',
    content: [{ type: 'text', text: summary }],
    stop_reason: 'end',
    model: 'm',
    provider: 'p',
    timestamp: 0,
  };
  return JSON.stringify({ type: 'done', message: msg });
}

function makeEmptyDoneEvent(): string {
  const msg: AssistantMessage = {
    role: 'assistant',
    content: [],
    stop_reason: 'end',
    model: 'm',
    provider: 'p',
    timestamp: 0,
  };
  return JSON.stringify({ type: 'done', message: msg });
}

type MockTriggerReq = { function_id: string; payload: Record<string, unknown> };

/**
 * Build a minimal ISdk mock.
 *
 * @param onProviderTrigger - Called synchronously when a provider trigger fires.
 *   Return the raw JSON event string to deliver on the channel, or null to skip.
 * @param extraHandlers - per function_id response overrides.
 */
function buildMock(
  onProviderTrigger: (payload: Record<string, unknown>) => string | null = () => makeDoneEvent(),
  extraHandlers: Record<string, (payload: Record<string, unknown>) => unknown> = {},
) {
  // The channel message callback registered by streamAndCollect
  let channelCb: ((raw: string) => void) | null = null;

  const channel = {
    reader: {
      onMessage(cb: (raw: string) => void) {
        channelCb = cb;
      },
      stream: { resume: () => {} },
    },
    writerRef: 'mock-writer-ref',
  };

  const trigger = vi.fn(async (req: MockTriggerReq) => {
    const { function_id, payload } = req;

    // Check extra handlers first
    if (extraHandlers[function_id]) {
      return extraHandlers[function_id](payload);
    }

    // Default responses for session-tree ops
    if (function_id === 'session-tree::messages') {
      return {
        messages: makeEntries().map((e) => ({ entry_id: e.entry_id, message: e.message })),
      };
    }
    if (function_id === 'session-tree::compactions') {
      return { entries: [] };
    }
    if (function_id === 'session-tree::compact') {
      return undefined;
    }
    // state::set / state::get used by stampLastCompaction / acquireLease
    if (function_id === 'state::set' || function_id === 'state::get') {
      return undefined;
    }

    // Provider stream: fire channel callback synchronously so terminal is set
    if (function_id.startsWith('provider::')) {
      if (channelCb) {
        const event = onProviderTrigger(payload);
        if (event !== null) {
          channelCb(event);
        }
      }
      return undefined;
    }

    return undefined;
  });

  const createChannel = vi.fn(async () => channel);

  const iii = { trigger, createChannel } as unknown as import('iii-sdk').ISdk;

  return { iii, trigger, createChannel };
}

// ---------------------------------------------------------------------------
// renderUserPrompt
// ---------------------------------------------------------------------------

describe('renderUserPrompt', () => {
  it('includes role markers and text content', () => {
    const msgs: AgentMessage[] = [userMsg('hello'), assistantMsg('hi')];
    const out = renderUserPrompt(msgs);
    expect(out).toContain('[user]');
    expect(out).toContain('[assistant]');
    expect(out).toContain('hello');
    expect(out).toContain('hi');
    expect(out).toContain('<conversation>');
    expect(out).toContain('</conversation>');
  });

  it('renders function_call blocks with verbatim ids', () => {
    const msg: AgentMessage = {
      role: 'assistant',
      content: [
        {
          type: 'function_call',
          id: 'tc1',
          function_id: 'shell::run',
          arguments: { command: 'ls' },
        },
      ],
      stop_reason: 'function_call',
      model: 'm',
      provider: 'p',
      timestamp: 0,
    };
    const out = renderUserPrompt([msg]);
    expect(out).toContain('[tool_call] shell::run');
    expect(out).toContain('"command"');
  });

  it('skips non-text non-function_call blocks silently', () => {
    const msg: AgentMessage = {
      role: 'user',
      content: [
        { type: 'image', media_type: 'image/png', data: 'abc' } as unknown as ContentBlock,
      ],
      timestamp: 0,
    };
    const out = renderUserPrompt([msg]);
    expect(out).toContain('[user]');
    expect(out).not.toContain('image/png');
  });
});

// ---------------------------------------------------------------------------
// estimateTokenCount
// ---------------------------------------------------------------------------

describe('estimateTokenCount', () => {
  it('returns positive for non-empty messages', () => {
    expect(estimateTokenCount([userMsg('hello')])).toBeGreaterThan(0);
  });

  it('scales with message size', () => {
    expect(estimateTokenCount([userMsg('x'.repeat(4000))])).toBeGreaterThan(
      estimateTokenCount([userMsg('x')]),
    );
  });

  it('returns 0 for empty array', () => {
    expect(estimateTokenCount([])).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// summarizeAndAppend — async mode
// ---------------------------------------------------------------------------

describe('summarizeAndAppend async mode', () => {
  it('loads messages, calls summariser, appends compaction, returns ok', async () => {
    const compactPayloads: unknown[] = [];

    const { iii, trigger } = buildMock(() => makeDoneEvent('my summary'), {
      'session-tree::messages': () => ({
        messages: makeEntries().map((e) => ({ entry_id: e.entry_id, message: e.message })),
      }),
      'session-tree::compact': (p) => {
        compactPayloads.push(p);
        return undefined;
      },
    });

    const result = await summarizeAndAppend(iii, 'sess-1', { mode: 'async' }, testModel);

    // Returns ok shape
    expect(result).not.toBe('compact');
    expect(result).not.toBe('empty');
    const ok = result as { tail_start_id: string | null; tokens_before: number };
    expect(ok.tokens_before).toBeGreaterThan(0);

    // Provider trigger fired
    const providerCall = trigger.mock.calls.find(([req]) =>
      req.function_id.startsWith('provider::'),
    );
    expect(providerCall).toBeDefined();

    // Compaction was appended with session_id and non-empty summary
    expect(compactPayloads).toHaveLength(1);
    const cp = compactPayloads[0] as Record<string, unknown>;
    expect(cp.session_id).toBe('sess-1');
    expect(typeof cp.summary).toBe('string');
    expect((cp.summary as string).length).toBeGreaterThan(0);
  });

  it('returns "empty" when summariser produces no text content', async () => {
    const { iii } = buildMock(() => makeEmptyDoneEvent());

    const result = await summarizeAndAppend(iii, 'sess-empty', { mode: 'async' }, testModel);
    expect(result).toBe('empty');
  });

  it('returns "compact" when streamAndCollect throws', async () => {
    const { iii } = buildMock();
    // Make createChannel reject so streamAndCollect throws
    (iii as unknown as { createChannel: ReturnType<typeof vi.fn> }).createChannel = vi
      .fn()
      .mockRejectedValue(new Error('channel unavailable'));

    const result = await summarizeAndAppend(iii, 'sess-fail', { mode: 'async' }, testModel);
    expect(typeof result === 'object' && 'kind' in result ? result.kind : result).toBe('compact');
    if (typeof result === 'object' && 'kind' in result && result.kind === 'compact') {
      expect(result.reason).toContain('channel unavailable');
    }
  });

  it('retries streamAndCollect once and succeeds when the first attempt fails', async () => {
    // Transient provider failures (429, network blip, 5xx) should not
    // surface to /compact when a single retry would succeed.
    const { iii } = buildMock();
    let createChannelCalls = 0;
    const realCreateChannel = (
      iii as unknown as { createChannel: () => Promise<unknown> }
    ).createChannel;
    (iii as unknown as { createChannel: () => Promise<unknown> }).createChannel = vi
      .fn(async () => {
        createChannelCalls += 1;
        if (createChannelCalls === 1) {
          throw new Error('transient provider failure');
        }
        return realCreateChannel();
      });

    const result = await summarizeAndAppend(iii, 'sess-retry', { mode: 'async' }, testModel);
    expect(createChannelCalls).toBe(2);
    expect(typeof result === 'object' && 'kind' in result ? result.kind : result).toBe('ok');
  }, 10_000);

  it('returns "compact" when both stream attempts fail', async () => {
    // Generic permanent failure (no auth/4xx markers) — still retries once
    // then surfaces the failure.
    const { iii } = buildMock();
    let createChannelCalls = 0;
    (iii as unknown as { createChannel: () => Promise<unknown> }).createChannel = vi
      .fn(async () => {
        createChannelCalls += 1;
        throw new Error('permanent provider failure');
      });

    const result = await summarizeAndAppend(iii, 'sess-fail-twice', { mode: 'async' }, testModel);
    expect(createChannelCalls).toBe(2);
    expect(typeof result === 'object' && 'kind' in result ? result.kind : result).toBe('compact');
    if (typeof result === 'object' && 'kind' in result && result.kind === 'compact') {
      expect(result.reason).toContain('permanent provider failure');
    }
  }, 10_000);

  it('skips retry on non-retryable errors (401/auth/malformed)', async () => {
    // Auth and 4xx errors won't fix on retry. Skip the retry so the
    // compaction lease releases ~1s sooner.
    const { iii } = buildMock();
    let createChannelCalls = 0;
    (iii as unknown as { createChannel: () => Promise<unknown> }).createChannel = vi
      .fn(async () => {
        createChannelCalls += 1;
        throw new Error('401 unauthorized: invalid_api_key');
      });

    const result = await summarizeAndAppend(iii, 'sess-auth-fail', { mode: 'async' }, testModel);
    expect(createChannelCalls).toBe(1); // no retry
    expect(typeof result === 'object' && 'kind' in result ? result.kind : result).toBe('compact');
    if (typeof result === 'object' && 'kind' in result && result.kind === 'compact') {
      expect(result.reason).toContain('401');
    }
  });

  it('returns "compact" when the provider stream emits an error terminal event', async () => {
    // Regression: a not_found_error from Anthropic (e.g. wrong provider/model
    // combo) used to be silently stored as the compaction summary text,
    // showing "COMPACTED · N TOKENS" in the UI with the error JSON as
    // the summary. The error terminal must surface as a failure instead.
    const errorEvent = JSON.stringify({
      type: 'error',
      error: {
        role: 'assistant',
        content: [
          {
            type: 'text',
            text: '{"type":"error","error":{"type":"not_found_error","message":"model: gpt-5-mini"}}',
          },
        ],
        stop_reason: 'end',
        error_kind: 'NotFound',
        error_message: 'model: gpt-5-mini',
        model: 'gpt-5-mini',
        provider: 'anthropic',
        timestamp: 0,
      },
    });
    const { iii } = buildMock(() => errorEvent);

    const result = await summarizeAndAppend(iii, 'sess-provider-error', { mode: 'async' }, testModel);
    expect(typeof result === 'object' && 'kind' in result ? result.kind : result).toBe('compact');
    if (typeof result === 'object' && 'kind' in result && result.kind === 'compact') {
      expect(result.reason.toLowerCase()).toMatch(/not.found|gpt-5-mini/);
    }
  });

  it('routes the summariser to the session provider when env is unset', async () => {
    // Regression: summarizerProvider() default was 'anthropic'. Sessions on
    // OpenAI got Anthropic stream + OpenAI model → not_found_error.
    let calledFunctionId: string | null = null;
    const { iii } = buildMock();
    const wrapped = iii as unknown as { trigger: ReturnType<typeof vi.fn> };
    const originalTrigger = wrapped.trigger;
    wrapped.trigger = vi.fn(async (req: MockTriggerReq) => {
      if (req.function_id.startsWith('provider::')) {
        calledFunctionId = req.function_id;
      }
      return originalTrigger(req);
    });

    const openAiModel = {
      providerID: 'openai',
      modelID: 'gpt-5-mini',
      modelLimit: { context: 200_000, input: 200_000, output: 4_096 },
    };
    await summarizeAndAppend(iii, 'sess-openai', { mode: 'async' }, openAiModel);
    expect(calledFunctionId).toBe('provider::openai::stream');
  });
});

// ---------------------------------------------------------------------------
// summarizeAndAppend — anchored prompt (prior compaction present)
// ---------------------------------------------------------------------------

describe('summarizeAndAppend anchored prompt', () => {
  it('includes <previous-summary> in system prompt when a prior compaction exists', async () => {
    const PRIOR_SUMMARY = 'Prior compaction summary text here.';
    const capturedPrompts: string[] = [];

    const { iii } = buildMock(
      (payload) => {
        capturedPrompts.push(payload.system_prompt as string);
        return makeDoneEvent('updated summary');
      },
      {
        'session-tree::compactions': () => ({
          entries: [
            {
              id: 'comp-1',
              summary: PRIOR_SUMMARY,
              tokens_before: 1000,
              timestamp: Date.now() - 60_000,
            },
          ],
        }),
        'session-tree::compact': () => undefined,
      },
    );

    await summarizeAndAppend(iii, 'sess-anchored', { mode: 'async' }, testModel);

    expect(capturedPrompts).toHaveLength(1);
    expect(capturedPrompts[0]).toContain('<previous-summary>');
    expect(capturedPrompts[0]).toContain(PRIOR_SUMMARY);
    expect(capturedPrompts[0]).toContain('</previous-summary>');
  });
});

// ---------------------------------------------------------------------------
// summarizeAndAppend — sync mode (truncatedEntries supplied by caller)
// ---------------------------------------------------------------------------

describe('summarizeAndAppend sync mode', () => {
  it('uses caller-supplied truncatedEntries and does NOT call session-tree::messages', async () => {
    const messagesCallCount = { n: 0 };
    const compactPayloads: unknown[] = [];

    const { iii } = buildMock(() => makeDoneEvent('sync summary'), {
      'session-tree::messages': () => {
        messagesCallCount.n++;
        return { messages: [] };
      },
      'session-tree::compact': (p) => {
        compactPayloads.push(p);
        return undefined;
      },
    });

    const entries = makeEntries();
    const result = await summarizeAndAppend(
      iii,
      'sess-sync',
      { mode: 'sync', truncatedEntries: entries },
      testModel,
    );

    expect(messagesCallCount.n).toBe(0);
    expect(result).not.toBe('compact');
    expect(result).not.toBe('empty');
    expect(compactPayloads).toHaveLength(1);
  });
});
