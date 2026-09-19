import { afterEach, it, vi } from 'vitest';
import {
  checkConfigurationContract,
  configurationCases,
} from '../../crates/node-core/tests/configuration-contract.mjs';

afterEach(() => vi.unstubAllEnvs());
it.each(configurationCases)('$title', async (scenario) => {
  vi.stubEnv('III_CONFIG_NAME', scenario.env);
  vi.resetModules();
  const { loadConfig, toRuntime } = await import('../src/config.js');
  const config = await import('../src/configuration.js');
  const seed = await loadConfig('/nonexistent/config.yaml');
  await checkConfigurationContract({
    ...scenario,
    register: (iii) => config.registerVscodeConfig(iii, seed),
    fetch: config.fetchRuntime,
    bind: config.bindConfigTrigger,
    expectedId: scenario.env?.trim() || 'vscode',
    formId: 'vscode',
    value: toRuntime(seed),
  });
});
