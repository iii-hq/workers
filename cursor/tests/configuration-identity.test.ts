import { InvocationError } from 'iii-sdk';
import { afterEach, it, vi } from 'vitest';
import {
  checkConfigurationContract,
  configurationCases,
} from '../../crates/node-core/tests/configuration-contract.mjs';
import { defaultConfig } from '../src/config.js';
import { fetchRuntime, registerCursorConfig } from '../src/configuration.js';

afterEach(() => {
  vi.unstubAllEnvs();
  vi.useRealTimers();
});
it.each(configurationCases)('$title', async (scenario) => {
  vi.stubEnv('III_CONFIG_NAME', scenario.env);
  vi.useFakeTimers();
  const seed = defaultConfig();
  const task = checkConfigurationContract({
    ...scenario,
    makeError: (body) => new InvocationError(body),
    register: (iii) => registerCursorConfig(iii, seed),
    fetch: fetchRuntime,
    expectedId: scenario.env?.trim() || 'cursor',
    formId: 'cursor',
    value: seed,
  });
  await vi.runAllTimersAsync();
  await task;
});
