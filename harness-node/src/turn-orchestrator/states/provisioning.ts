import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { agentCallTool } from '../agent-call.js';
import type { TurnOrchestratorConfig } from '../config.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import { type DefaultSkillBody, type Mode, buildSystemPrompt, defaultSkillBody } from '../system-prompt.js';

function asMode(value: unknown): Mode | null {
  return value === 'plan' || value === 'ask' || value === 'agent' ? value : null;
}

const FETCH_TIMEOUT_MS = 10_000;

async function fetchSkill(iii: ISdk, id: string): Promise<string | null> {
  try {
    const resp = await iii.trigger<unknown, unknown>({
      function_id: 'directory::skills::get',
      payload: { id },
      timeoutMs: FETCH_TIMEOUT_MS,
    });
    if (typeof resp === 'string') return resp;
    if (resp && typeof resp === 'object') {
      const body = (resp as Record<string, unknown>).body;
      if (typeof body === 'string') return body;
    }
    return null;
  } catch (err) {
    logger.warn('directory::skills::get failed', { id, err: String(err) });
    return null;
  }
}

export async function handleProvisioning(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  rec: TurnStateRecord,
): Promise<void> {
  const request = await persistence.loadRunRequest(iii, rec.session_id);

  // The single tool LLMs see is `agent_call`.
  await persistence.saveFunctionSchemas(iii, rec.session_id, [agentCallTool()]);

  const overrideRaw = request.system_prompt;
  const override = typeof overrideRaw === 'string' && overrideRaw.length > 0 ? overrideRaw : null;
  const cwd = typeof request.cwd === 'string' ? (request.cwd as string) : null;
  const mode = asMode(request.mode);

  const bodies: DefaultSkillBody[] = [];
  for (const uri of cfg.system_default_skills) {
    const id = uri.startsWith('iii://') ? uri.slice('iii://'.length) : uri;
    const body = await fetchSkill(iii, id);
    bodies.push(defaultSkillBody(uri, body));
  }
  const prompt = buildSystemPrompt(bodies, cwd, override, mode);

  const updated = { ...request, system_prompt: prompt };
  await persistence.saveRunRequest(iii, rec.session_id, updated);

  transitionTo(rec, 'awaiting_assistant');
}
