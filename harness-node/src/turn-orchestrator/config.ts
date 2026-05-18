import { getNumber, getStringArray } from '../runtime/config.js';

export type TurnOrchestratorConfig = {
  sync_default_timeout_ms: number;
  sync_poll_interval_ms: number;
  system_default_skills: string[];
};

export function loadOrchestratorConfig(cfg: Record<string, unknown>): TurnOrchestratorConfig {
  return {
    sync_default_timeout_ms: getNumber(cfg, 'sync_default_timeout_ms', 120_000),
    sync_poll_interval_ms: getNumber(cfg, 'sync_poll_interval_ms', 50),
    system_default_skills: getStringArray(cfg, 'system_default_skills', [
      'iii://iii-directory/index',
    ]),
  };
}
