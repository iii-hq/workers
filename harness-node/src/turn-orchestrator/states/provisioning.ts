/**
 * `provisioning`. First FSM step after `run::start`: materialize tool schemas,
 * assemble the system prompt, persist the enriched run request, then advance.
 *
 * **Incoming**: provisioning `TurnStateRecord` (`rec.session_id`) plus persisted
 * run request from `persistence.loadRunRequest` (written by `run::start`).
 * **Outgoing**: `transitionTo(rec, 'awaiting_assistant')`; side effects:
 * `saveFunctionSchemas` (agent_trigger), `saveRunRequest` (built prompt),
 * directory skill fetches for default skills + index.
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { agentTriggerTool } from '../agent-trigger.js';
import type { TurnOrchestratorConfig } from '../config.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import {
  type DefaultSkillBody,
  type Mode,
  buildSystemPrompt,
  defaultSkillBody,
} from '../system-prompt.js';

type RunRequest = {
  provider: string;
  model: string;
  mode: Mode | null;
  system_prompt: string;
  image: string;
  idle_timeout_secs: number;
};

const FETCH_TIMEOUT_MS = 10_000;

function parseMode(value: unknown): Mode | null {
  return value === 'plan' || value === 'ask' || value === 'agent' ? value : null;
}

export function parseRunRequest(raw: Record<string, unknown>): RunRequest {
  return {
    provider: typeof raw.provider === 'string' ? raw.provider : '',
    model: typeof raw.model === 'string' ? raw.model : '',
    mode: parseMode(raw.mode),
    system_prompt: typeof raw.system_prompt === 'string' ? raw.system_prompt : '',
    image: typeof raw.image === 'string' ? raw.image : 'python',
    idle_timeout_secs: typeof raw.idle_timeout_secs === 'number' ? raw.idle_timeout_secs : 300,
  };
}

export function parseDirectoryBody(resp: unknown): string | null {
  if (typeof resp === 'string') return resp;
  if (resp && typeof resp === 'object') {
    const body = (resp as { body?: unknown }).body;
    if (typeof body === 'string') return body;
  }
  return null;
}

async function fetchSkill(iii: ISdk, id: string): Promise<string | null> {
  try {
    const resp = await iii.trigger<unknown, unknown>({
      function_id: 'directory::skills::get',
      payload: { id },
      timeoutMs: FETCH_TIMEOUT_MS,
    });
    return parseDirectoryBody(resp);
  } catch (err) {
    logger.warn('directory::skills::get failed', { id, err: String(err) });
    return null;
  }
}

async function fetchDefaultSkills(iii: ISdk, uris: readonly string[]): Promise<DefaultSkillBody[]> {
  const bodies: DefaultSkillBody[] = [];
  for (const uri of uris) {
    const id = uri.startsWith('iii://') ? uri.slice('iii://'.length) : uri;
    const body = await fetchSkill(iii, id);
    bodies.push(defaultSkillBody(uri, body));
  }
  return bodies;
}

async function fetchSkillsIndex(iii: ISdk): Promise<string | null> {
  try {
    const resp = await iii.trigger<unknown, unknown>({
      function_id: 'directory::skills::index',
      payload: {},
      timeoutMs: FETCH_TIMEOUT_MS,
    });
    const body = parseDirectoryBody(resp);
    return body && body.length > 0 ? body : null;
  } catch (err) {
    logger.warn('directory::skills::index failed', { err: String(err) });
    return null;
  }
}

export async function handleProvisioning(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  rec: TurnStateRecord,
): Promise<void> {
  const request = parseRunRequest(await persistence.loadRunRequest(iii, rec.session_id));

  // The single tool LLMs see is `agent_trigger`.
  await persistence.saveFunctionSchemas(iii, rec.session_id, [agentTriggerTool()]);

  const override = request.system_prompt.length > 0 ? request.system_prompt : null;

  const [skillsIndex, bodies] = await Promise.all([
    fetchSkillsIndex(iii),
    fetchDefaultSkills(iii, cfg.system_default_skills),
  ]);
  const prompt = buildSystemPrompt(bodies, null, override, request.mode, skillsIndex);

  const updated: RunRequest = { ...request, system_prompt: prompt };
  await persistence.saveRunRequest(iii, rec.session_id, updated);

  transitionTo(rec, 'awaiting_assistant');
}
