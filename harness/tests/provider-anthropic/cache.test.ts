import { describe, expect, it } from 'vitest';
import { applyMessagesCacheAnchor } from '../../src/provider-anthropic/cache.js';

type WireMsg = Record<string, unknown>;

function assistantTurn(content: Array<Record<string, unknown>>): WireMsg {
  return { role: 'assistant', content };
}

describe('applyMessagesCacheAnchor', () => {
  it('anchors cache_control on the last block of the last stable assistant turn', () => {
    const wire: WireMsg[] = [
      { role: 'user', content: [{ type: 'text', text: 'hi' }] },
      assistantTurn([{ type: 'text', text: 'answer' }]),
    ];
    applyMessagesCacheAnchor(wire);
    const content = (wire[1] as { content: Array<Record<string, unknown>> }).content;
    expect(content[0]?.cache_control).toEqual({ type: 'ephemeral' });
  });

  it('skips trailing thinking blocks (cache_control is rejected on them)', () => {
    const wire: WireMsg[] = [
      { role: 'user', content: [{ type: 'text', text: 'hi' }] },
      assistantTurn([
        { type: 'text', text: 'answer' },
        { type: 'thinking', thinking: 'why', signature: 'sig' },
      ]),
    ];
    applyMessagesCacheAnchor(wire);
    const content = (wire[1] as { content: Array<Record<string, unknown>> }).content;
    expect(content[1]?.cache_control).toBeUndefined();
    expect(content[0]?.cache_control).toEqual({ type: 'ephemeral' });
  });

  it('does nothing when the turn contains only thinking blocks', () => {
    const wire: WireMsg[] = [
      { role: 'user', content: [{ type: 'text', text: 'hi' }] },
      assistantTurn([{ type: 'thinking', thinking: 'why', signature: 'sig' }]),
    ];
    applyMessagesCacheAnchor(wire);
    const content = (wire[1] as { content: Array<Record<string, unknown>> }).content;
    expect(content[0]?.cache_control).toBeUndefined();
  });
});
