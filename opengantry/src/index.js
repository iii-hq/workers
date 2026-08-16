import {
  evaluateScope,
  verifyMission,
  mintVerdictToken,
  verifyVerdictToken,
  isPromoteClassFunctionId,
} from '@jeger-ai/opengantry/kernel';

import { createMiddlewareHandler, isReservedGovernanceFunctionId } from './lib/middleware.js';
import { VerifyCoalescer } from './lib/verify-coalescer.js';
import { opengantryWorkerOptions } from './lib/worker-init.js';
import { resolveVerifyRepoRoot } from './lib/repo-path.js';
import { loadSchema } from './lib/function-formats.js';
import { scanLocalWorkers, practicesFailedPayload } from './lib/iii-practices/scan.mjs';
import { resolveRepoRoot, loadHttpConnectorAllowlist } from './lib/iii-practices/allowlist.mjs';

const state = {
  leaseStores: new Map(),
  coalescer: new VerifyCoalescer(),
  forwardTrigger: async (function_id, payload) => ({ ok: true, function_id, payload }),
};

async function runVerify(data) {
  const repoRoot = resolveVerifyRepoRoot(data.repo_root);
  const { workers: httpAllowlist } = loadHttpConnectorAllowlist(resolveRepoRoot());
  const { findings, logs } = await scanLocalWorkers(repoRoot, { httpAllowlist });
  for (const line of logs) console.log(line);
  if (findings.length) return practicesFailedPayload(findings);
  return verifyMission({
    repoRoot,
    missionRelPath: data.mission_rel_path,
    options: data.options ?? { skipStaleEvidence: true },
  });
}

async function startWorker() {
  const url = process.env.III_URL;
  if (!url) {
    console.log('opengantry worker: III_URL not set — idle (use demo.mjs for offline harness)');
    return;
  }

  const { registerWorker } = await import('iii-sdk');
  const worker = registerWorker(url, opengantryWorkerOptions());

  const middleware = createMiddlewareHandler(state);

  state.forwardTrigger = async (function_id, payload) => worker.trigger({ function_id, payload });

  worker.registerFunction('gantry::middleware', middleware, {
    request_format: loadSchema('gantry__middleware.json'),
    response_format: loadSchema('gantry__middleware.response.json'),
  });

  worker.registerFunction(
    'gantry::verify',
    async (data) => {
      const repoRoot = data?.repo_root;
      const key = `${repoRoot}:${data?.msn_id ?? ''}`;
      return state.coalescer.run(key, () => runVerify(data));
    },
    {
      request_format: loadSchema('gantry__verify.json'),
      response_format: loadSchema('gantry__verify.response.json'),
    },
  );

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

  worker.registerFunction(
    'gantry::on-trigger-type-registration',
    async () => {
      throw new Error('trigger type registration denied');
    },
    {
      request_format: loadSchema('gantry__on-trigger-type-registration.json'),
      response_format: loadSchema('gantry__on-trigger-type-registration.response.json'),
    },
  );

  worker.registerTriggerType({
    id: 'gantry::verdict',
    description: 'Emitted when gantry verify completes',
  });

  console.log(`opengantry worker registered (verify, middleware, RBAC hooks) → ${url}`);
}

startWorker().catch((err) => {
  console.error(err);
  process.exit(1);
});

export {
  state,
  evaluateScope,
  mintVerdictToken,
  verifyVerdictToken,
  isPromoteClassFunctionId,
  runVerify,
};
