import { describe, expect, it } from 'vitest';
import { loadOrchestratorConfig } from '../../src/turn-orchestrator/config.js';

describe('loadOrchestratorConfig', () => {
  it('exposes policy_function_id with a sane default', () => {
    const cfg = loadOrchestratorConfig({});
    expect(cfg.policy_function_id).toBe('policy::check_permissions');
  });

  it('lets turn_orchestrator.policy_function_id override the default', () => {
    const cfg = loadOrchestratorConfig({
      turn_orchestrator: { policy_function_id: 'custom::policy' },
    });
    expect(cfg.policy_function_id).toBe('custom::policy');
  });
});
