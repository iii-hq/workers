import { describe, expect, it } from 'vitest';
import { isReasoningModel, reasoningEffortFor } from '../../src/provider-openai/reasoning.js';

describe('isReasoningModel', () => {
  it.each([
    'gpt-5',
    'gpt-5-mini',
    'gpt-5.2',
    'o1-preview',
    'o3-mini',
    'o4-mini',
  ])('detects %s by id pattern', (id) => {
    expect(isReasoningModel(id)).toBe(true);
  });

  it.each(['gpt-4o', 'gpt-4.1', 'chatgpt-4o-latest'])('rejects %s by id pattern', (id) => {
    expect(isReasoningModel(id)).toBe(false);
  });

  it('catalog flag wins over the id pattern', () => {
    expect(isReasoningModel('gpt-4o', true)).toBe(true);
    expect(isReasoningModel('gpt-5', false)).toBe(false);
  });
});

describe('reasoningEffortFor', () => {
  it('defaults to medium when no level is given', () => {
    expect(reasoningEffortFor(undefined, 'gpt-5')).toBe('medium');
    expect(reasoningEffortFor(undefined, 'o3-mini')).toBe('medium');
  });

  it('passes supported levels through', () => {
    expect(reasoningEffortFor('high', 'gpt-5')).toBe('high');
    expect(reasoningEffortFor('low', 'gpt-5.1')).toBe('low');
  });

  it('gpt-5-pro is clamped to high regardless of level', () => {
    expect(reasoningEffortFor('low', 'gpt-5-pro')).toBe('high');
    expect(reasoningEffortFor(undefined, 'gpt-5-pro')).toBe('high');
    expect(reasoningEffortFor('xhigh', 'gpt-5-pro')).toBe('high');
  });

  it('xhigh only on gpt-5.2+ families', () => {
    expect(reasoningEffortFor('xhigh', 'gpt-5.2')).toBe('xhigh');
    expect(reasoningEffortFor('xhigh', 'gpt-5.3-codex')).toBe('xhigh');
    expect(reasoningEffortFor('xhigh', 'gpt-5.1')).toBe('high');
    expect(reasoningEffortFor('xhigh', 'gpt-5')).toBe('high');
  });

  it("maps 'off' to the lowest supported effort", () => {
    expect(reasoningEffortFor('off', 'gpt-5.1')).toBe('none');
    expect(reasoningEffortFor('off', 'gpt-5')).toBe('minimal');
    expect(reasoningEffortFor('off', 'o3')).toBe('low');
  });

  it("maps 'max' to xhigh where supported", () => {
    expect(reasoningEffortFor('max', 'gpt-5.2')).toBe('xhigh');
    expect(reasoningEffortFor('max', 'o3')).toBe('high');
  });

  it('returns undefined for chat-tuned variants', () => {
    expect(reasoningEffortFor('high', 'gpt-5-chat-latest')).toBeUndefined();
    expect(reasoningEffortFor(undefined, 'gpt-5.2-chat-latest')).toBeUndefined();
  });

  it('returns undefined for non-reasoning families', () => {
    expect(reasoningEffortFor('high', 'gpt-4o')).toBeUndefined();
  });

  it.each([
    'o1',
    'o1-mini',
    'o1-preview',
    'o1-pro',
  ])('returns undefined for %s (o1 family rejects reasoning_effort)', (id) => {
    expect(reasoningEffortFor(undefined, id)).toBeUndefined();
    expect(reasoningEffortFor('high', id)).toBeUndefined();
  });

  it('falls back to medium for an unrecognized level on families that support it', () => {
    expect(reasoningEffortFor('turbo', 'gpt-5')).toBe('medium');
    expect(reasoningEffortFor('turbo', 'o3')).toBe('medium');
  });

  it('falls back to the only supported effort for an unrecognized level on single-effort families', () => {
    expect(reasoningEffortFor('turbo', 'gpt-5-pro')).toBe('high');
  });
});
