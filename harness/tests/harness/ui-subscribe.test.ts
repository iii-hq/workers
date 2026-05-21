import { describe, expect, it } from 'vitest';
import { FanoutState } from '../../src/harness/ui-subscribe.js';

describe('FanoutState', () => {
  it('subscribes per-browser to specific session', () => {
    const s = new FanoutState();
    s.subscribe('b1', 's1');
    expect(s.subscribersFor('s1')).toEqual(['b1']);
    expect(s.subscribersFor('other')).toEqual([]);
  });

  it('null session subscribes to all sessions', () => {
    const s = new FanoutState();
    s.subscribe('b1', null);
    expect(s.subscribersFor('any-session')).toContain('b1');
    expect(s.allSubscribers()).toEqual(['b1']);
  });

  it('unsubscribe removes the targeted entry', () => {
    const s = new FanoutState();
    s.subscribe('b1', 's1');
    s.subscribe('b1', 's2');
    s.unsubscribe('b1', 's1');
    expect(s.subscribersFor('s1')).toEqual([]);
    expect(s.subscribersFor('s2')).toEqual(['b1']);
  });

  it('evicts a browser entirely', () => {
    const s = new FanoutState();
    s.subscribe('b1', null);
    s.evictBrowser('b1');
    expect(s.browserCount()).toBe(0);
  });
});
