import { describe, expect, it } from 'vitest';
import { type Model, parseCapability, supportsModel } from '../../src/models-catalog/types.js';

const m = (overrides: Partial<Model>): Model => ({
  id: 'm',
  provider: 'p',
  api: 'a',
  display_name: 'M',
  context_window: 100,
  ...overrides,
});

describe('parseCapability', () => {
  it('parses known capability strings', () => {
    expect(parseCapability('thinking')).toEqual({ type: 'thinking' });
    expect(parseCapability('thinking:xhigh')).toEqual({
      type: 'thinking_level',
      level: 'xhigh',
    });
    expect(parseCapability('tools')).toEqual({ type: 'tools' });
  });

  it('returns null for unknown', () => {
    expect(parseCapability('weird')).toBeNull();
  });
});

describe('supportsModel', () => {
  it('thinking_level:xhigh maps to supports_xhigh', () => {
    expect(
      supportsModel(m({ supports_xhigh: true, supports_thinking: false }), {
        type: 'thinking_level',
        level: 'xhigh',
      }),
    ).toBe(true);
  });

  it('thinking_level:medium maps to supports_thinking', () => {
    expect(
      supportsModel(m({ supports_thinking: true, supports_xhigh: false }), {
        type: 'thinking_level',
        level: 'medium',
      }),
    ).toBe(true);
  });

  it('tools / vision / cache map to their flags', () => {
    expect(supportsModel(m({ supports_tools: true }), { type: 'tools' })).toBe(true);
    expect(supportsModel(m({ supports_vision: true }), { type: 'vision' })).toBe(true);
    expect(supportsModel(m({ supports_cache: true }), { type: 'cache' })).toBe(true);
  });
});
