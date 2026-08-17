/** iii-sdk init for the OpenGantry control-plane worker only. */
import { envFlag } from './env-flag.js';

export function opengantryWorkerOptions() {
  return {
    workerName: 'opengantry',
    workerDescription: 'OpenGantry governance (verify, middleware, RBAC hooks)',
    otel: { enabled: envFlag('OTEL_ENABLED') },
  };
}
