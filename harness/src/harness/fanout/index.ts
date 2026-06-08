import type { ISdk } from '../../runtime/iii.js';
import type { FanoutState } from '../ui-subscribe.js';
import { spawnModelsChanged } from './models-changed.js';

export type FanoutPumps = {
  shutdown(): Promise<void>;
};

export function spawnPumps(iii: ISdk, state: FanoutState): FanoutPumps {
  const stopModels = spawnModelsChanged(iii, state);
  return {
    async shutdown() {
      stopModels();
    },
  };
}
