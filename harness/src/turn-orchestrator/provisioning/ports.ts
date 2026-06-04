/**
 * Typed dependency ports for provisioning.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { RunRequest } from '../run-request.js';
import { createTurnStore } from '../state-runtime/store.js';

export type ProvisioningPorts = {
  loadRunRequest(session_id: string): Promise<RunRequest>;
  saveRunRequest(session_id: string, request: RunRequest): Promise<void>;
};

export function createProvisioningPorts(iii: ISdk): ProvisioningPorts {
  const store = createTurnStore(iii);

  return {
    loadRunRequest(session_id) {
      return store.loadRunRequest(session_id);
    },

    saveRunRequest(session_id, request) {
      return store.saveRunRequest(session_id, request);
    },
  };
}
