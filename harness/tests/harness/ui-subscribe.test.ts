import { describe, expect, it } from 'vitest';
import { FanoutState } from '../../src/harness/ui-subscribe.js';

describe('FanoutState', () => {
  it('tracks model subscribers', () => {
    const s = new FanoutState();
    s.subscribeModels('b1');
    expect(s.modelSubscribers()).toEqual(['b1']);
    s.unsubscribeModels('b1');
    expect(s.modelSubscribers()).toEqual([]);
  });

  it('tracks multiple model subscribers independently', () => {
    const s = new FanoutState();
    s.subscribeModels('b1');
    s.subscribeModels('b2');
    expect(s.modelSubscribers().sort()).toEqual(['b1', 'b2']);
    s.unsubscribeModels('b1');
    expect(s.modelSubscribers()).toEqual(['b2']);
  });
});
