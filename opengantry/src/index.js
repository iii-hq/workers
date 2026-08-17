import { createMiddlewareHandler, isReservedGovernanceFunctionId } from './lib/middleware.js';
import { opengantryWorkerOptions } from './lib/worker-init.js';
import { loadSchema } from './lib/schemas.js';
import { createVerifyHandler } from './lib/verify-handler.js';
import { createWorkerState } from './lib/worker-state.js';

async function startWorker() {
  const url = process.env.III_URL;
  if (!url) {
    console.log('opengantry worker: III_URL not set — idle (use demo.mjs for offline harness)');
    return;
  }

  const state = createWorkerState();
  const { registerWorker } = await import('iii-sdk');
  const worker = registerWorker(url, opengantryWorkerOptions());

  const middleware = createMiddlewareHandler(state);

  state.forwardTrigger = async (function_id, payload) => worker.trigger({ function_id, payload });

  worker.registerFunction('gantry::middleware', middleware, {
    request_format: loadSchema('gantry__middleware.json'),
    response_format: loadSchema('gantry__middleware.response.json'),
  });

  worker.registerFunction('gantry::verify', createVerifyHandler(state), {
    request_format: loadSchema('gantry__verify.json'),
    response_format: loadSchema('gantry__verify.response.json'),
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
      request_format: loadSchema('gantry__on-function-registration.json'),
      response_format: loadSchema('gantry__on-function-registration.response.json'),
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
      request_format: loadSchema('gantry__on-trigger-registration.json'),
      response_format: loadSchema('gantry__on-trigger-registration.response.json'),
    },
  );

  worker.registerFunction('gantry::on-trigger-type-registration', async () => ({ denied: true }), {
    request_format: loadSchema('gantry__on-trigger-type-registration.json'),
    response_format: loadSchema('gantry__on-trigger-type-registration.response.json'),
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

export {
  createWorkerState,
  createVerifyHandler,
  createMiddlewareHandler,
  isReservedGovernanceFunctionId,
};
