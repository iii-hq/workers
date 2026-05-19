import { describe, expect, it } from 'vitest';
import { FN_RESOLVE, pendingKey } from '../../src/approval-gate/types.js';

describe('pendingKey', () => {
  it('joins session and call ids', () => {
    expect(pendingKey('s1', 'tc-1')).toBe('s1/tc-1');
  });
  it('rejects ids that contain "/"', () => {
    expect(() => pendingKey('a/b', 'tc')).toThrow();
    expect(() => pendingKey('a', 'b/c')).toThrow();
  });
});

describe('approval-gate function constants', () => {
  it('exposes active approval-gate iii function ids', () => {
    expect(FN_RESOLVE).toBe('approval::resolve');
  });
});
