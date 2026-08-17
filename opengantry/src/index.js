import { createMiddlewareHandler, isReservedGovernanceFunctionId } from './middleware.js';
import { createVerifyHandler, createWorkerState } from './verify.js';
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

  const state = createWorkerState();
  const { registerWorker } = await import('iii-sdk');
  const worker = registerWorker(url, opengantryWorkerOptions());

  const middleware = createMiddlewareHandler(state);

  state.forwardTrigger = async (function_id, payload) => worker.trigger({ function_id, payload });

  worker.registerFunction('gantry::middleware', middleware, {
    request_format: MIDDLEWARE_REQUEST_FORMAT,
    response_format: MIDDLEWARE_RESPONSE_FORMAT,
  });

  worker.registerFunction('gantry::verify', createVerifyHandler(state), {
    request_format: VERIFY_REQUEST_FORMAT,
    response_format: VERIFY_RESPONSE_FORMAT,
  });

  worker.registerFunction(
    'gantry::on-function-registration',
    async (input) => {
      if (isReservedGovernanceFunctionId(input.function_id)) {
        throw new Error(`reserved namespace: ${input.function_id}`);
      }
      return { function_id: input.function_id };
    },
    {
      request_format: ON_FUNCTION_REGISTRATION_REQUEST_FORMAT,
      response_format: ON_FUNCTION_REGISTRATION_RESPONSE_FORMAT,
    },
  );

  worker.registerFunction(
    'gantry::on-trigger-registration',
    async (input) => {
      if (input.function_id.startsWith('gantry::')) {
        throw new Error('cannot bind trigger to gantry namespace');
      }
      return input;
    },
    {
      request_format: ON_TRIGGER_REGISTRATION_REQUEST_FORMAT,
      response_format: ON_TRIGGER_REGISTRATION_RESPONSE_FORMAT,
    },
  );

  worker.registerFunction('gantry::on-trigger-type-registration', async () => ({ denied: true }), {
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
