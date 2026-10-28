/**
 * Worker boot. Registers gantry::* on the engine, then injects this
 * connection's trigger() as forwardTrigger so middleware can replay a
 * call after the policy checks. forwardTrigger is required: there is no
 * fail-open default that would skip the governed listener.
 */
import { registerWorker } from 'iii-sdk';

import { formatFor, FUNCTION_FORMATS } from './formats.js';
import {
  onFunctionRegistration,
  onTriggerRegistration,
  onTriggerTypeRegistration,
} from './registration-hooks.js';
import { createGantryRuntime } from './runtime.js';
import { createVerdictEmitter, VERDICT_OBSERVABLE, VERDICT_TRIGGER } from './verdict-events.js';

const HANDLERS = {
  'gantry::middleware': null,
  'gantry::verify': null,
  'gantry::on-function-registration': onFunctionRegistration,
  'gantry::on-trigger-registration': onTriggerRegistration,
  'gantry::on-trigger-type-registration': onTriggerTypeRegistration,
};

function opengantryWorkerOptions() {
  return {
    workerName: 'opengantry',
    workerDescription: 'OpenGantry governance (verify, middleware, RBAC hooks)',
    otel: { enabled: process.env.OTEL_ENABLED === 'true' },
  };
}

async function startWorker() {
  const url = process.env.III_URL ?? process.env.III_ENGINE_URL ?? 'ws://127.0.0.1:49134';

  const worker = registerWorker(url, opengantryWorkerOptions());
  const verdictEmitter = createVerdictEmitter({
    trigger: (request) => worker.trigger(request),
  });
  const { middleware, verify } = createGantryRuntime({
    forwardTrigger: (function_id, payload) => worker.trigger({ function_id, payload }),
    emitVerdict: verdictEmitter.emit.bind(verdictEmitter),
  });
  HANDLERS['gantry::middleware'] = middleware;
  HANDLERS['gantry::verify'] = verify;

  for (const [functionId, formats] of Object.entries(FUNCTION_FORMATS)) {
    worker.registerFunction(functionId, HANDLERS[functionId], {
      ...(formats.description ? { description: formats.description } : {}),
      request_format: formatFor(formats.request),
      response_format: formatFor(formats.response),
    });
  }

  worker.registerTriggerType(
    {
      id: VERDICT_TRIGGER,
      description: 'Emitted when gantry verify completes',
    },
    verdictEmitter.handler,
  );

  // iii 0.25: the observable function next to the trigger type, sharing its
  // bindings (owner-delivered; a direct call is refused with FORBIDDEN).
  worker.registerFunction(VERDICT_OBSERVABLE, verdictEmitter.onVerdict, {
    description: `Emitted when gantry verify completes (same payload as ${VERDICT_TRIGGER}). Observe with iii.observe({ function_id: '${VERDICT_OBSERVABLE}', args: {} }, target) (iii 0.25); invoking it is refused with FORBIDDEN.`,
    observe: verdictEmitter.observe,
  });

  console.log(`opengantry worker registered (verify, middleware, RBAC hooks) → ${url}`);
}

startWorker().catch((err) => {
  console.error(err);
  process.exit(1);
});
