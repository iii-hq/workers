/**
 * Load run request, fetch skills, build the provisioned RunRequest, and register the FSM step.
 */

import type { ISdk } from '../../runtime/iii.js';
import { agentTriggerTool } from '../agent-trigger.js';
import type { TurnOrchestratorConfig } from '../config.js';
import { runTransition } from '../run-transition.js';
import type { RunRequest } from '../run-request.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { buildSystemPrompt } from '../system-prompt.js';
import { transitionTo, type TurnStateRecord } from '../state.js';
import { loadDefaultSkillBodies } from './load-skills.js';
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

  const [skillsIndex, bodies] = await Promise.all([
    ports.fetchSkillsIndex(),
    loadDefaultSkillBodies(ports, ports.defaultSkillUris),
  ]);
  const prompt = buildSystemPrompt(bodies, { override, mode: request.mode, skillsIndex });

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

export async function runProvisioning(ports: ProvisioningPorts, rec: TurnStateRecord): Promise<void> {
  const outcome = await processProvisioning(ports, rec);
  await applyProvisioningOutcome(ports, rec, outcome);
}

export async function handleProvisioning(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  rec: TurnStateRecord,
): Promise<void> {
  const ports = createProvisioningPorts(iii, cfg);
  await runProvisioning(ports, rec);
}

export function register(iii: ISdk, cfg: TurnOrchestratorConfig): void {
  iii.registerFunction(
    'turn::provisioning',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(
        iii,
        'provisioning',
        (i, rec) => handleProvisioning(i, cfg, rec),
        parsed,
      );
    },
    {
      description:
        'Run one durable FSM transition for session in state provisioning: build the system prompt, attach the agent_trigger function schema, advance to assistant_streaming.',
    },
  );
}
