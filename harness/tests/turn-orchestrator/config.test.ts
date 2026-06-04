import { describe, expect, it } from 'vitest';
import { loadOrchestratorConfig } from '../../src/turn-orchestrator/config.js';

describe('loadOrchestratorConfig', () => {
  it('defaults system_default_skills to empty when no config is supplied', () => {
    // The code-level fallback is intentionally empty; the running engine
    // supplies the actual list via config.yaml's system_default_skills.
    const cfg = loadOrchestratorConfig({});
    expect(cfg.system_default_skills).toEqual([]);
  });

  it('reads system_default_skills from config', () => {
    const cfg = loadOrchestratorConfig({
      system_default_skills: ['skill-a'],
    });
    expect(cfg.system_default_skills).toEqual(['skill-a']);
  });
});
