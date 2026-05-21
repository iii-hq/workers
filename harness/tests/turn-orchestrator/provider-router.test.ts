import { describe, expect, it } from 'vitest';
import {
  buildInput,
  decide,
  targetFunctionId,
} from '../../src/turn-orchestrator/provider-router.js';

describe('decide', () => {
  it('routes anthropic when provider=anthropic', () => {
    expect(decide({ provider: 'anthropic', model: 'claude' }).provider).toBe('anthropic');
  });

  it('routes openai when provider=openai', () => {
    expect(decide({ provider: 'openai', model: 'gpt-5' }).provider).toBe('openai');
  });

  it('routes kimi when provider=kimi', () => {
    expect(decide({ provider: 'kimi', model: 'kimi-k2-0905-preview' }).provider).toBe('kimi');
  });

  it('falls back to model heuristic when provider missing', () => {
    expect(decide({ model: 'gpt-5' }).provider).toBe('openai');
    expect(decide({ model: 'claude-opus-4-7' }).provider).toBe('anthropic');
    expect(decide({ model: 'kimi-k2-0905-preview' }).provider).toBe('kimi');
    expect(decide({ model: 'kimi-k2-turbo-preview' }).provider).toBe('kimi');
    expect(decide({ model: 'kimi-k2.6' }).provider).toBe('kimi');
    expect(decide({ model: 'moonshot-v1-128k' }).provider).toBe('kimi');
    expect(decide({ model: 'moonshot-v1-8k-vision-preview' }).provider).toBe('kimi');
  });
});

describe('targetFunctionId', () => {
  it('maps decisions to provider stream function ids', () => {
    expect(targetFunctionId({ provider: 'anthropic', model: 'm' })).toBe(
      'provider::anthropic::stream',
    );
    expect(targetFunctionId({ provider: 'openai', model: 'm' })).toBe('provider::openai::stream');
    expect(targetFunctionId({ provider: 'kimi', model: 'm' })).toBe('provider::kimi::stream');
  });
});

describe('buildInput', () => {
  it('roundtrips the canonical fields', () => {
    const input = buildInput(
      { provider: 'anthropic', model: 'claude' },
      { channel_id: 'c', access_key: 'k', direction: 'write' },
      'sys',
      [{ role: 'user', content: [{ type: 'text', text: 'hi' }], timestamp: 0 }],
      [{ name: 'agent_trigger', description: 'd', parameters: {} }],
    );
    expect(input.model).toBe('claude');
    expect(input.tools).toHaveLength(1);
    expect(input.writer_ref.channel_id).toBe('c');
  });
});
