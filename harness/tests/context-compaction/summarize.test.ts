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
// ISdk mock factory
//
// The summariser now runs through one `router::complete` call — no channel.
// `onComplete` receives the payload and returns the final AssistantMessage
// (or throws to fail the call, feeding the retry logic).
// ---------------------------------------------------------------------------

function doneMessage(summary = 'the summary'): AssistantMessage {
  return {
    role: 'assistant',
    content: [{ type: 'text', text: summary }],
    stop_reason: 'end',
    model: 'm',
    provider: 'p',
    timestamp: 0,
  };
}

type MockTriggerReq = { function_id: string; payload: Record<string, unknown> };

/**
 * Build a minimal ISdk mock.
 *
 * @param onComplete - Called with the `router::complete` payload; returns the
 *   AssistantMessage wrapped as `{ message }`, or throws to fail the call.
 * @param extraHandlers - per function_id response overrides.
 */
function buildMock(
  onComplete: (payload: Record<string, unknown>) => AssistantMessage = () => doneMessage(),
  extraHandlers: Record<string, (payload: Record<string, unknown>) => unknown> = {},
) {
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

    if (function_id === 'router::complete') {
      return { message: onComplete(payload), provider: payload.provider, model: payload.model };
    }

    return undefined;
  });

  const iii = { trigger } as unknown as import('iii-sdk').ISdk;

  return { iii, trigger };
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
      content: [{ type: 'image', media_type: 'image/png', data: 'abc' } as unknown as ContentBlock],
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

    const { iii, trigger } = buildMock(() => doneMessage('my summary'), {
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

    // The summariser ran through router::complete
    const completeCall = trigger.mock.calls.find(
      ([req]) => req.function_id === 'router::complete',
    );
    expect(completeCall).toBeDefined();

    // Compaction was appended with session_id and non-empty summary
    expect(compactPayloads).toHaveLength(1);
    const cp = compactPayloads[0] as Record<string, unknown>;
    expect(cp.session_id).toBe('sess-1');
    expect(typeof cp.summary).toBe('string');
    expect((cp.summary as string).length).toBeGreaterThan(0);
  });

  it('returns "empty" when summariser produces no text content', async () => {
    const { iii } = buildMock(() => ({ ...doneMessage(), content: [] }));

    const result = await summarizeAndAppend(iii, 'sess-empty', { mode: 'async' }, testModel);
    expect(result).toBe('empty');
  });

  it('returns "compact" when router::complete rejects', async () => {
    const { iii } = buildMock(() => {
      throw new Error('router unreachable');
    });

    const result = await summarizeAndAppend(iii, 'sess-fail', { mode: 'async' }, testModel);
    expect(typeof result === 'object' && 'kind' in result ? result.kind : result).toBe('compact');
    if (typeof result === 'object' && 'kind' in result && result.kind === 'compact') {
      expect(result.reason).toContain('router unreachable');
    }
  });

  it('retries router::complete once and succeeds when the first attempt fails', async () => {
    // Transient provider failures (429, network blip, 5xx) should not
    // surface to /compact when a single retry would succeed.
    let attempts = 0;
    const { iii } = buildMock(() => {
      attempts += 1;
      if (attempts === 1) {
        throw new Error('transient provider failure');
      }
      return doneMessage('recovered summary');
    });

    const result = await summarizeAndAppend(iii, 'sess-retry', { mode: 'async' }, testModel);
    expect(attempts).toBe(2);
    expect(typeof result === 'object' && 'kind' in result ? result.kind : result).toBe('ok');
  }, 10_000);

  it('returns "compact" when both stream attempts fail', async () => {
    // Generic permanent failure (no auth/4xx markers) — still retries once
    // then surfaces the failure.
    let attempts = 0;
    const { iii } = buildMock(() => {
      attempts += 1;
      throw new Error('permanent provider failure');
    });

    const result = await summarizeAndAppend(iii, 'sess-fail-twice', { mode: 'async' }, testModel);
    expect(attempts).toBe(2);
    expect(typeof result === 'object' && 'kind' in result ? result.kind : result).toBe('compact');
    if (typeof result === 'object' && 'kind' in result && result.kind === 'compact') {
      expect(result.reason).toContain('permanent provider failure');
    }
  }, 10_000);

  it('skips retry on non-retryable errors (401/auth/malformed)', async () => {
    // Auth and 4xx errors won't fix on retry. Skip the retry so the
    // compaction lease releases ~1s sooner.
    let attempts = 0;
    const { iii } = buildMock(() => {
      attempts += 1;
      throw new Error('401 unauthorized: invalid_api_key');
    });

    const result = await summarizeAndAppend(iii, 'sess-auth-fail', { mode: 'async' }, testModel);
    expect(attempts).toBe(1); // no retry
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
    const { iii } = buildMock(() => ({
      role: 'assistant',
      content: [
        {
          type: 'text',
          text: '{"type":"error","error":{"type":"not_found_error","message":"model: gpt-5-mini"}}',
        },
      ],
      stop_reason: 'error',
      error_message: 'model: gpt-5-mini',
      model: 'gpt-5-mini',
      provider: 'anthropic',
      timestamp: 0,
    }));

    const result = await summarizeAndAppend(
      iii,
      'sess-provider-error',
      { mode: 'async' },
      testModel,
    );
    expect(typeof result === 'object' && 'kind' in result ? result.kind : result).toBe('compact');
    if (typeof result === 'object' && 'kind' in result && result.kind === 'compact') {
      expect(result.reason.toLowerCase()).toMatch(/not.found|gpt-5-mini/);
    }
  });

  it('pins the session provider/model on the router::complete call', async () => {
    // /compact uses the session's own provider/model, pinned explicitly so
    // the router executes on exactly the provider the session streams on.
    const { iii, trigger } = buildMock();

    const openAiModel = {
      providerID: 'openai',
      modelID: 'gpt-5-mini',
      modelLimit: { context: 200_000, input: 200_000, output: 4_096 },
    };
    await summarizeAndAppend(iii, 'sess-openai', { mode: 'async' }, openAiModel);
    const call = trigger.mock.calls.find(([req]) => req.function_id === 'router::complete');
    expect(call?.[0].payload).toEqual(
      expect.objectContaining({ provider: 'openai', model: 'gpt-5-mini' }),
    );
  });

  it('pins kimi sessions to the kimi provider, not anthropic', async () => {
    const { iii, trigger } = buildMock();

    const kimiModel = {
      providerID: 'kimi',
      modelID: 'kimi-k2.5',
      modelLimit: { context: 256_000, input: 256_000, output: 16_384 },
    };
    await summarizeAndAppend(iii, 'sess-kimi', { mode: 'async' }, kimiModel);
    const call = trigger.mock.calls.find(([req]) => req.function_id === 'router::complete');
    expect(call?.[0].payload).toEqual(
      expect.objectContaining({ provider: 'kimi', model: 'kimi-k2.5' }),
    );
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
        return doneMessage('updated summary');
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

    const { iii } = buildMock(() => doneMessage('sync summary'), {
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
