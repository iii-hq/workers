/**
 * Load run request, build the provisioned RunRequest, and register the FSM step.
 */

import type { ISdk } from '../../runtime/iii.js';
import { agentTriggerTool } from '../agent-trigger.js';
import { runTransition } from '../run-transition.js';
import type { RunRequest } from '../run-request.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { buildSystemPrompt } from '../system-prompt.js';
import { transitionTo, type TurnStateRecord } from '../state.js';
import { createProvisioningPorts, type ProvisioningPorts } from './ports.js';

export type ProvisioningOutcome = {
  kind: 'ready';
  runRequest: RunRequest;
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

  return {
    kind: 'ready',
    runRequest: {
      ...request,
      system_prompt: prompt,
      function_schemas: [agentTriggerTool()],
    },
  };
}

export async function applyProvisioningOutcome(
  ports: ProvisioningPorts,
  rec: TurnStateRecord,
  outcome: ProvisioningOutcome,
): Promise<void> {
  await ports.saveRunRequest(rec.session_id, outcome.runRequest);
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
