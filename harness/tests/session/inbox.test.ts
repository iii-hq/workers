import { describe, expect, it } from 'vitest';
import { inboxKey } from '../../src/session/inbox/key.js';

describe('inboxKey', () => {
  it('namespaces by session and name', () => {
    expect(inboxKey('steering', 's1')).toBe('s1/steering');
  });
});
