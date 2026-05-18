import { getNumber, getSection, getString, getStringArray } from '../runtime/config.js';

export type TurnOrchestratorConfig = {
  sync_default_timeout_ms: number;
  system_default_skills: string[];
  policy_function_id: string;
};

export function loadOrchestratorConfig(cfg: Record<string, unknown>): TurnOrchestratorConfig {
  const section = getSection(cfg, 'turn_orchestrator');
  return {
    sync_default_timeout_ms: getNumber(cfg, 'sync_default_timeout_ms', 120_000),
    system_default_skills: getStringArray(cfg, 'system_default_skills', [
      'iii://iii-directory/index',
    ]),
    policy_function_id: getString(section, 'policy_function_id', 'policy::check_permissions'),
  };
}
