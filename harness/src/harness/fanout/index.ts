import type { ISdk } from '../../runtime/iii.js';
import type { FanoutState } from '../ui-subscribe.js';
import { spawnAgentEventsPump } from './agent-events.js';
import { spawnSessionsPoll } from './sessions-poll.js';
import { spawnTracesChangedPump } from './traces-changed.js';

export type FanoutPumps = {
  shutdown(): Promise<void>;
};

export function spawnPumps(iii: ISdk, state: FanoutState): FanoutPumps {
  const agentEventsTrigger = spawnAgentEventsPump(iii, state);
  const stopSessions = spawnSessionsPoll(iii, state);
  const stopTraces = spawnTracesChangedPump(iii, state);
  return {
    async shutdown() {
      try {
        agentEventsTrigger?.unregister();
      } catch {
        // ignore
      }
      stopSessions();
      stopTraces();
    },
  };
}
