import { describe, expect, it } from 'vitest';
import { loadOrchestratorConfig } from '../../src/turn-orchestrator/config.js';

describe('loadOrchestratorConfig', () => {
  it('applies defaults for system skills', () => {
    const cfg = loadOrchestratorConfig({});
    expect(cfg.system_default_skills).toEqual(['iii://iii-directory/index']);
  });

  it('reads system_default_skills from config', () => {
    const cfg = loadOrchestratorConfig({
      system_default_skills: ['skill-a'],
    });
    expect(cfg.system_default_skills).toEqual(['skill-a']);
  });
});
