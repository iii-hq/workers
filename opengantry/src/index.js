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
import { createVerdictEmitter } from './verdict-events.js';

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
  const url = process.env.III_URL ?? process.env.III_ENGINE_URL;
  if (!url) {
    throw new Error('opengantry worker: III_URL or III_ENGINE_URL is required');
  }

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
      request_format: formatFor(formats.request),
      response_format: formatFor(formats.response),
    });
  }

  worker.registerTriggerType(
    {
      id: 'gantry::verdict',
      description: 'Emitted when gantry verify completes',
    },
    verdictEmitter.handler,
  );

  console.log(`opengantry worker registered (verify, middleware, RBAC hooks) → ${url}`);
}

startWorker().catch((err) => {
  console.error(err);
  process.exit(1);
});
