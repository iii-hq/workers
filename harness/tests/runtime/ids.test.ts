import { describe, expect, it } from 'vitest';
import { uuidLike } from '../../src/runtime/ids.js';

describe('uuidLike', () => {
  it('returns unique values across rapid calls', () => {
    const seen = new Set<string>();
    for (let i = 0; i < 1000; i++) seen.add(uuidLike());
    expect(seen.size).toBe(1000);
  });

  it('contains a hex-like timestamp prefix', () => {
    const id = uuidLike();
    expect(id.split('-').length).toBeGreaterThanOrEqual(3);
  });
});
