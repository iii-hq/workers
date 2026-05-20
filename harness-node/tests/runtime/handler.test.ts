import { describe, expect, it } from 'vitest';
import { optionalString, requireString, unwrapBody } from '../../src/runtime/handler.js';

describe('unwrapBody', () => {
  it('peels the body envelope when present', () => {
    expect(unwrapBody({ body: { x: 1 } })).toEqual({ x: 1 });
  });

  it('returns raw object when no body field', () => {
    expect(unwrapBody({ x: 1 })).toEqual({ x: 1 });
  });

  it('returns {} for non-object input', () => {
    expect(unwrapBody(null)).toEqual({});
    expect(unwrapBody('hi')).toEqual({});
    expect(unwrapBody(42)).toEqual({});
  });
});

describe('requireString', () => {
  it('returns the value when present and non-empty', () => {
    expect(requireString({ k: 'v' }, 'k')).toBe('v');
  });

  it('throws on missing/empty/non-string', () => {
    expect(() => requireString({}, 'k')).toThrow(/missing required string/);
    expect(() => requireString({ k: '' }, 'k')).toThrow();
    expect(() => requireString({ k: 42 }, 'k')).toThrow();
  });
});

describe('optionalString', () => {
  it('returns undefined for missing or non-string', () => {
    expect(optionalString({}, 'k')).toBeUndefined();
    expect(optionalString({ k: 42 }, 'k')).toBeUndefined();
  });
  it('returns the value when string', () => {
    expect(optionalString({ k: '' }, 'k')).toBe('');
    expect(optionalString({ k: 'v' }, 'k')).toBe('v');
  });
});
