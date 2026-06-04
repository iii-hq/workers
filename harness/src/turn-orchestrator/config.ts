import { getStringArray } from '../runtime/config.js';

export type TurnOrchestratorConfig = {
  system_default_skills: string[];
};

export function loadOrchestratorConfig(cfg: Record<string, unknown>): TurnOrchestratorConfig {
  return {
    system_default_skills: getStringArray(cfg, 'system_default_skills', []),
  };
}
