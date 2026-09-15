import { registerWorker } from 'iii-sdk';
import { type AgentGuildOptions, createOperations } from './operations.js';
import {
  passportRequest,
  passportResponse,
  preflightRequest,
  preflightResponse,
} from './schemas.js';
import { record } from './validation.js';

export type { AgentGuildOptions } from './operations.js';
export { createOperations } from './operations.js';

/** Explicit startup only. Importing this module does not connect or make a Guild request. */
export function startWorker(address?: string, options: AgentGuildOptions = {}) {
  const operations = createOperations(options);
  const stopped = new AbortController();
  const iii = registerWorker(address, {
    workerName: 'agent-guild',
    workerDescription:
      'Optional public endpoint observations and supplied public passport verification. No delegation guard or payment.',
    enableMetricsReporting: false,
    otel: { enabled: false },
  });
  iii.registerFunction(
    'agent-guild::preflight',
    (input) => operations.preflight(withoutEngineCaller(input), stopped.signal),
    {
      description:
        'Actively observe one selected public HTTP(S) endpoint through Guild; return six measured statuses and unknowns, never authorization or proof of independent ownership.',
      metadata: { mcp: { expose: true } },
      request_format: preflightRequest,
      response_format: preflightResponse,
    },
  );
  iii.registerFunction(
    'agent-guild::verify_passport',
    (input) => operations.verifyPassport(withoutEngineCaller(input), stopped.signal),
    {
      description:
        'Send a supplied PUBLIC passport unchanged to Guild with independently expected issuer/subject checked locally. Verification records passport_verified even for unsuccessful verification. Origin/integrity only, no endpoint ownership inference.',
      metadata: { mcp: { expose: true } },
      request_format: passportRequest,
      response_format: passportResponse,
    },
  );
  return {
    client: iii,
    async shutdown() {
      stopped.abort();
      await iii.shutdown();
    },
  };
}

/** Remove only iii's bounded top-level transport field, never credential claims.
 * The engine overwrites this field. It is ignored, not used for authorization.
 * Missing metadata is accepted for direct/local calls; malformed metadata stays
 * in the input so the strict operation guard rejects it without a request.
 */
export function withoutEngineCaller(input: unknown): unknown {
  if (!record(input)) return input;
  const fields = Object.getOwnPropertyDescriptors(input);
  const caller = fields._caller_worker_id;
  if (!caller) return input;
  if (
    !caller.enumerable ||
    !('value' in caller) ||
    typeof caller.value !== 'string' ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(caller.value)
  )
    return input;
  delete fields._caller_worker_id;
  return Object.defineProperties({}, fields);
}
