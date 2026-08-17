/** Operator-opt-in governance bypass (unsafe for production). */
import { envFlag } from './env-flag.js';

export function isBypassMode() {
  return envFlag('GANTRY_BYPASS_MODE');
}
