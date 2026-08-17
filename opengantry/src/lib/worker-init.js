/** iii-sdk init for the OpenGantry control-plane worker only. */
import { envFlag } from './env-flag.js';

export function opengantryWorkerOptions() {
  const bypass = envFlag('GANTRY_BYPASS_MODE');
  if (bypass) {
    console.warn(
      'opengantry: GANTRY_BYPASS_MODE is enabled — governance middleware is disabled (unsafe for production)',
    );
  }
  return {
    workerName: 'opengantry',
    workerDescription: bypass
      ? 'OpenGantry governance (BYPASS MODE — middleware disabled)'
      : 'OpenGantry governance (verify, middleware, RBAC hooks)',
    otel: { enabled: envFlag('OTEL_ENABLED') },
  };
}
