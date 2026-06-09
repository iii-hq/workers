/**
 * Load run request, build the provisioned RunRequest, and register the FSM step.
 */

import type { Model } from '../../models-catalog/types.js';
import type { ISdk } from '../../runtime/iii.js';
import { agentTriggerTool } from '../agent-trigger.js';
import { decide } from '../provider-router.js';
import { runTransition } from '../run-transition.js';
import type { RunRequest } from '../run-request.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { buildSystemPrompt } from '../system-prompt.js';
import { transitionTo, type TurnStateRecord } from '../state.js';
import { createProvisioningPorts, type ProvisioningPorts } from './ports.js';

export type ProvisioningOutcome = {
  kind: 'ready';
  runRequest: RunRequest;
  /** Catalog entry for the turn's model; null when the catalog has no match. */
  model_meta: Model | null;
};

export async function processProvisioning(
  ports: ProvisioningPorts,
  rec: TurnStateRecord,
): Promise<ProvisioningOutcome> {
  const request = await ports.loadRunRequest(rec.session_id);

  const override = request.system_prompt.length > 0 ? request.system_prompt : null;
  const prompt = buildSystemPrompt({
    override,
    mode: request.mode,
    provider: request.provider,
    model: request.model,
  });

  // Resolve the model once for the whole turn. The decided provider (not the
  // raw request.provider, which may be blank) is what every later reader keys
  // on, so resolve against it. Best-effort: null is fine — readers fall back.
  const decision = decide({ provider: request.provider, model: request.model });
  const model_meta = request.model
    ? await ports.resolveModel(decision.provider, request.model)
    : null;

  return {
    kind: 'ready',
    runRequest: {
      ...request,
      system_prompt: prompt,
      function_schemas: [agentTriggerTool()],
    },
    model_meta,
  };
}

export async function applyProvisioningOutcome(
  ports: ProvisioningPorts,
  rec: TurnStateRecord,
  outcome: ProvisioningOutcome,
): Promise<void> {
  await ports.saveRunRequest(rec.session_id, outcome.runRequest);
  // Persist the resolved catalog entry on the durable record so preflight,
  // provider streaming, and turn-end compaction read it instead of re-fetching.
  if (outcome.model_meta) rec.model_meta = outcome.model_meta;
  transitionTo(rec, 'assistant_streaming');
}

export async function runProvisioning(
  ports: ProvisioningPorts,
  rec: TurnStateRecord,
): Promise<void> {
  const outcome = await processProvisioning(ports, rec);
  await applyProvisioningOutcome(ports, rec, outcome);
}

export async function handleProvisioning(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const ports = createProvisioningPorts(iii);
  await runProvisioning(ports, rec);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::provisioning',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'provisioning', (i, rec) => handleProvisioning(i, rec), parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state provisioning: build the system prompt, attach the agent_trigger function schema, advance to assistant_streaming.',
    },
  );
}
