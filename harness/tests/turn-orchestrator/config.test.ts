import { describe, expect, it } from 'vitest';
import { loadOrchestratorConfig } from '../../src/turn-orchestrator/config.js';

describe('loadOrchestratorConfig', () => {
  it('applies defaults for sync timeout and system skills', () => {
    const cfg = loadOrchestratorConfig({});
    expect(cfg.sync_default_timeout_ms).toBe(120_000);
    expect(cfg.system_default_skills).toEqual(['iii://iii-directory/index']);
  });

  it('reads sync_default_timeout_ms and system_default_skills from config', () => {
    const cfg = loadOrchestratorConfig({
      sync_default_timeout_ms: 60_000,
      system_default_skills: ['skill-a'],
    });
    expect(cfg.sync_default_timeout_ms).toBe(60_000);
    expect(cfg.system_default_skills).toEqual(['skill-a']);
  });
});
