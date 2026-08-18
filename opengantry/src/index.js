/**
 * Worker boot. Registers gantry::* on the engine, then injects this
 * connection's trigger() as forwardTrigger so middleware can replay a
 * call after the policy checks. forwardTrigger is required: there is no
 * fail-open default that would skip the governed listener.
 */
import { registerWorker } from 'iii-sdk';

import {
  MIDDLEWARE_REQUEST_FORMAT,
  MIDDLEWARE_RESPONSE_FORMAT,
  ON_FUNCTION_REGISTRATION_REQUEST_FORMAT,
  ON_FUNCTION_REGISTRATION_RESPONSE_FORMAT,
  ON_TRIGGER_REGISTRATION_REQUEST_FORMAT,
  ON_TRIGGER_REGISTRATION_RESPONSE_FORMAT,
  ON_TRIGGER_TYPE_REGISTRATION_REQUEST_FORMAT,
  ON_TRIGGER_TYPE_REGISTRATION_RESPONSE_FORMAT,
  VERIFY_REQUEST_FORMAT,
  VERIFY_RESPONSE_FORMAT,
} from './formats.js';
import {
  onFunctionRegistration,
  onTriggerRegistration,
  onTriggerTypeRegistration,
} from './registration-hooks.js';
import { createGantryRuntime } from './runtime.js';

function envFlag(name) {
  const value = process.env[name]?.trim().toLowerCase();
  return value === 'true' || value === '1' || value === 'yes' || value === 'on';
}

function opengantryWorkerOptions() {
  return {
    workerName: 'opengantry',
    workerDescription: 'OpenGantry governance (verify, middleware, RBAC hooks)',
    otel: { enabled: envFlag('OTEL_ENABLED') },
  };
}

async function startWorker() {
  const url = process.env.III_URL ?? process.env.III_ENGINE_URL;
  if (!url) {
    throw new Error('opengantry worker: III_URL or III_ENGINE_URL is required');
  }

  const worker = registerWorker(url, opengantryWorkerOptions());
  const { middleware, verify } = createGantryRuntime({
    forwardTrigger: (function_id, payload) => worker.trigger({ function_id, payload }),
  });

  worker.registerFunction('gantry::middleware', middleware, {
    request_format: MIDDLEWARE_REQUEST_FORMAT,
    response_format: MIDDLEWARE_RESPONSE_FORMAT,
  });

  worker.registerFunction('gantry::verify', verify, {
    request_format: VERIFY_REQUEST_FORMAT,
    response_format: VERIFY_RESPONSE_FORMAT,
  });

  worker.registerFunction('gantry::on-function-registration', onFunctionRegistration, {
    request_format: ON_FUNCTION_REGISTRATION_REQUEST_FORMAT,
    response_format: ON_FUNCTION_REGISTRATION_RESPONSE_FORMAT,
  });

  worker.registerFunction('gantry::on-trigger-registration', onTriggerRegistration, {
    request_format: ON_TRIGGER_REGISTRATION_REQUEST_FORMAT,
    response_format: ON_TRIGGER_REGISTRATION_RESPONSE_FORMAT,
  });

  worker.registerFunction('gantry::on-trigger-type-registration', onTriggerTypeRegistration, {
    request_format: ON_TRIGGER_TYPE_REGISTRATION_REQUEST_FORMAT,
    response_format: ON_TRIGGER_TYPE_REGISTRATION_RESPONSE_FORMAT,
  });

  worker.registerTriggerType(
    {
      id: 'gantry::verdict',
      description: 'Emitted when gantry verify completes',
    },
    {
      registerTrigger() {},
      unregisterTrigger() {},
    },
  );

  console.log(`opengantry worker registered (verify, middleware, RBAC hooks) → ${url}`);
}

startWorker().catch((err) => {
  console.error(err);
  process.exit(1);
});
