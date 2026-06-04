import type { ISdk } from '../../runtime/iii.js';
import type { FanoutState } from '../ui-subscribe.js';
import { spawnSessionsPoll } from './sessions-poll.js';

export type FanoutPumps = {
  shutdown(): Promise<void>;
};

export function spawnPumps(iii: ISdk, state: FanoutState): FanoutPumps {
  const stopSessions = spawnSessionsPoll(iii, state);
  return {
    async shutdown() {
      stopSessions();
    },
  };
}
