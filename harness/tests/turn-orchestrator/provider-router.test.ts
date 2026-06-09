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

  it('routes llamacpp when provider=llamacpp', () => {
    expect(decide({ provider: 'llamacpp', model: 'Meta-Llama-3.1-8B' }).provider).toBe('llamacpp');
    // No heuristic — bare model id without explicit provider does not
    // route to llamacpp (same posture as lmstudio; user-controlled ids
    // overlap with HF-style ids from other services).
    expect(decide({ model: 'Meta-Llama-3.1-8B' }).provider).toBe('anthropic');
  });

  it('maps targetFunctionId for llamacpp', () => {
    expect(targetFunctionId({ provider: 'llamacpp', model: 'm' })).toBe(
      'provider::llamacpp::stream',
    );
  });

  it('routes lmstudio when provider=lmstudio (no model-name heuristic)', () => {
    expect(decide({ provider: 'lmstudio', model: 'qwen/qwen3-4b-2507' }).provider).toBe('lmstudio');
    expect(
      decide({
        provider: 'lmstudio',
        model: 'lmstudio-community/Meta-Llama-3.1-8B-Instruct-GGUF',
      }).provider,
    ).toBe('lmstudio');
    // No heuristic — a bare HF-style model id without an explicit
    // provider MUST default to anthropic, not be magically routed to
    // lmstudio because the namespace looks like a HF org/repo. This
    // is the regression that the route's comment warns about
    // ("model IDs are user-controlled … overlap with HF-style IDs").
    const ambiguousIds = [
      'qwen/qwen3-4b-2507',
      'google/gemma-2-9b-it',
      'google/gemma-3-e4b',
      'meta-llama/Llama-3-70B',
      'mistralai/Mistral-7B-v0.3',
      'TheBloke/Llama-2-7B-GGUF',
      'lmstudio-community/Meta-Llama-3.1-8B-Instruct-GGUF',
      'deepseek-ai/DeepSeek-R1',
    ];
    for (const model of ambiguousIds) {
      expect(decide({ model }).provider, `model=${model}`).toBe('anthropic');
    }
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
    expect(targetFunctionId({ provider: 'lmstudio', model: 'm' })).toBe(
      'provider::lmstudio::stream',
    );
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

  it('carries thinking_level when provided and omits it when absent', () => {
    const decision = { provider: 'anthropic', model: 'claude' } as const;
    const writer = { channel_id: 'c', access_key: 'k', direction: 'write' } as const;
    const withLevel = buildInput(decision, writer, 'sys', [], [], 'high');
    expect(withLevel.thinking_level).toBe('high');
    const withoutLevel = buildInput(decision, writer, 'sys', [], []);
    expect(withoutLevel).not.toHaveProperty('thinking_level');
  });

  it('carries model_meta when provided and omits it when absent', () => {
    const decision = { provider: 'anthropic', model: 'claude-sonnet-4-6' } as const;
    const writer = { channel_id: 'c', access_key: 'k', direction: 'write' } as const;
    const model_meta = {
      id: 'claude-sonnet-4-6',
      provider: 'anthropic',
      api: 'anthropic-messages',
      display_name: 'Claude Sonnet 4.6',
      context_window: 1_000_000,
      max_output_tokens: 64_000,
    };
    const withMeta = buildInput(decision, writer, 'sys', [], [], undefined, model_meta);
    expect(withMeta.model_meta).toEqual(model_meta);
    const withoutMeta = buildInput(decision, writer, 'sys', [], []);
    expect(withoutMeta).not.toHaveProperty('model_meta');
  });

  it('carries resolution_key when provided and omits it when absent', () => {
    const decision = { provider: 'anthropic', model: 'claude' } as const;
    const writer = { channel_id: 'c', access_key: 'k', direction: 'write' } as const;
    const withKey = buildInput(
      decision,
      writer,
      'sys',
      [],
      [],
      undefined,
      undefined,
      1738000000000,
    );
    expect(withKey.resolution_key).toBe(1738000000000);
    const withoutKey = buildInput(decision, writer, 'sys', [], []);
    expect(withoutKey).not.toHaveProperty('resolution_key');
  });
});
