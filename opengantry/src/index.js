/**
 * Worker boot. Registers gantry::* on the engine, then injects this
 * connection's call() as forwardCall so middleware can replay a
 * call after the policy checks. forwardCall is required: there is no
 * fail-open default that would skip the governed listener.
 */
import { registerWorker } from 'iii-sdk';

import { formatFor, FUNCTION_FORMATS } from './formats.js';
import { onFunctionRegistration, onObserverRegistration } from './registration-hooks.js';
import { createGantryRuntime } from './runtime.js';
import { createVerdictEmitter, ON_VERDICT_FN } from './verdict-events.js';

const HANDLERS = {
  'gantry::middleware': null,
  'gantry::verify': null,
  'gantry::on-function-registration': onFunctionRegistration,
  'gantry::on-observer-registration': onObserverRegistration,
  [ON_VERDICT_FN]: null,
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
    call: (request) => worker.call(request),
    isObserved: () => worker.isObserved(ON_VERDICT_FN),
  });
  const { middleware, verify } = createGantryRuntime({
    forwardCall: (function_id, payload) => worker.call({ function_id, payload }),
    emitVerdict: verdictEmitter.emit.bind(verdictEmitter),
  });
  HANDLERS['gantry::middleware'] = middleware;
  HANDLERS['gantry::verify'] = verify;
  HANDLERS[ON_VERDICT_FN] = verdictEmitter.handler;

  for (const [functionId, formats] of Object.entries(FUNCTION_FORMATS)) {
    worker.registerFunction(functionId, HANDLERS[functionId], {
      ...(formats.description ? { description: formats.description } : {}),
      request_format: formatFor(formats.request),
      response_format: formatFor(formats.response),
      ...(formats.private ? { private: true } : {}),
    });
  }

  console.log(`opengantry worker registered (verify, middleware, RBAC hooks) → ${url}`);
}

startWorker().catch((err) => {
  console.error(err);
  process.exit(1);
});
