import { describe, expect, it, vi } from 'vitest';
import type { ISdk } from '../../src/runtime/iii.js';
import type { AssistantMessage } from '../../src/types/agent-message.js';
import type { ProviderStreamInput } from '../../src/types/provider.js';
import type { AssistantMessageEvent } from '../../src/types/stream-event.js';
import {
  formatProviderError,
  streamProviderTurn,
} from '../../src/turn-orchestrator/provider-stream.js';

function assistant(overrides: Partial<AssistantMessage> = {}): AssistantMessage {
  return {
    role: 'assistant',
    content: [{ type: 'text', text: 'hi' }],
    stop_reason: 'end',
    error_message: null,
    error_kind: null,
    usage: null,
    model: 'gpt-4o',
    provider: 'openai',
    timestamp: 1,
    ...overrides,
  };
}

/**
 * Fake iii: its channel delivers the given events (JSON-encoded) synchronously
 * on `stream.resume()`, and `trigger` resolves to null unless `triggerRejects`.
 */
function fakeIii(opts: {
  events?: unknown[];
  triggerRejects?: unknown;
  createChannelThrows?: unknown;
}): ISdk {
  return {
    createChannel: async () => {
      if (opts.createChannelThrows) throw opts.createChannelThrows;
      let deliver: ((m: string) => void) | null = null;
      return {
        writerRef: {},
        reader: {
          onMessage: (cb: (m: string) => void) => {
            deliver = cb;
          },
          stream: {
            resume: () => {
              for (const e of opts.events ?? []) deliver?.(JSON.stringify(e));
            },
          },
        },
      };
    },
    trigger: async () => {
      if (opts.triggerRejects) throw opts.triggerRejects;
      return null;
    },
  } as unknown as ISdk;
}

const baseParams = {
  session_id: 's1',
  targetFn: 'provider::openai::stream',
  buildInput: () => ({}) as unknown as ProviderStreamInput,
  onDelta: async () => {},
};

describe('streamProviderTurn', () => {
  it('returns the done frame as the final message', async () => {
    const finalMsg = assistant({ content: [{ type: 'text', text: 'done' }] });
    const iii = fakeIii({ events: [{ type: 'done', message: finalMsg }] });

    const result = await streamProviderTurn(iii, baseParams);

    expect(result.final).toEqual(finalMsg);
    expect(result.error).toBeNull();
  });

  it('invokes onDelta per partial and tracks the latest before done', async () => {
    const p1 = assistant({ content: [{ type: 'text', text: 'a' }] });
    const p2 = assistant({ content: [{ type: 'text', text: 'ab' }] });
    const finalMsg = assistant({ content: [{ type: 'text', text: 'abc' }] });
    const seen: AssistantMessageEvent[] = [];
    const iii = fakeIii({
      events: [
        { type: 'text_delta', partial: p1, delta: 'a' },
        { type: 'text_delta', partial: p2, delta: 'b' },
        { type: 'done', message: finalMsg },
      ],
    });

    const result = await streamProviderTurn(iii, {
      ...baseParams,
      onDelta: async (_partial, event) => {
        seen.push(event);
      },
    });

    expect(seen.map((e) => e.type)).toEqual(['text_delta', 'text_delta']);
    expect(result.final).toEqual(finalMsg);
    expect(result.error).toBeNull();
  });

  it('surfaces the error frame as the final message', async () => {
    const errMsg = assistant({ stop_reason: 'error', error_message: 'boom' });
    const iii = fakeIii({ events: [{ type: 'error', error: errMsg }] });

    const result = await streamProviderTurn(iii, baseParams);

    expect(result.final).toEqual(errMsg);
    expect(result.error).toBeNull();
  });

  it('returns a cleaned error when the provider trigger rejects', async () => {
    const iii = fakeIii({ triggerRejects: new Error('IIIInvocationError: upstream 500') });

    const result = await streamProviderTurn(iii, baseParams);

    expect(result.final).toBeNull();
    expect(result.error).toBe('upstream 500');
  });

  it('returns a create_channel error when the channel cannot be created', async () => {
    const iii = fakeIii({ createChannelThrows: new Error('channel unavailable') });

    const result = await streamProviderTurn(iii, baseParams);

    expect(result.final).toBeNull();
    expect(result.error).toContain('create_channel failed');
  });

  it('skips undecodable frames and still completes on done', async () => {
    const finalMsg = assistant({ content: [{ type: 'text', text: 'ok' }] });
    const onDelta = vi.fn(async () => {});
    const iii = {
      createChannel: async () => {
        let deliver: ((m: string) => void) | null = null;
        return {
          writerRef: {},
          reader: {
            onMessage: (cb: (m: string) => void) => {
              deliver = cb;
            },
            stream: {
              resume: () => {
                deliver?.('not json');
                deliver?.(JSON.stringify({ type: 'done', message: finalMsg }));
              },
            },
          },
        };
      },
      trigger: async () => null,
    } as unknown as ISdk;

    const result = await streamProviderTurn(iii, { ...baseParams, onDelta });

    expect(result.final).toEqual(finalMsg);
    expect(onDelta).not.toHaveBeenCalled();
  });
});

describe('formatProviderError', () => {
  it('strips iii invocation-error prefixes', () => {
    expect(formatProviderError(new Error('IIIInvocationError: nope'))).toBe('nope');
    expect(formatProviderError(new Error('invocation_failed: nope'))).toBe('nope');
    expect(formatProviderError('plain string')).toBe('plain string');
  });
});
