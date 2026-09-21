import { InvocationError } from 'iii-sdk';
import { test } from 'node:test';
import {
  checkConfigurationContract,
  configurationCases,
} from '../../crates/node-core/tests/configuration-contract.mjs';

for (const [index, scenario] of configurationCases.entries()) {
  test(scenario.title, async () => {
    const previous = process.env.III_CONFIG_NAME;
    try {
      if (scenario.env === undefined) delete process.env.III_CONFIG_NAME;
      else process.env.III_CONFIG_NAME = scenario.env;
      const config = await import(`../src/lib/configuration.mjs?identity=${index}`);
      await checkConfigurationContract({
        ...scenario,
        makeError: (body) => new InvocationError(body),
        register: config.registerConfig,
        fetch: config.fetchConfig,
        bind: config.bindConfigTrigger,
        expectedId: scenario.env?.trim() || 'openwiki',
        formId: 'openwiki',
        value: config.defaults(),
      });
    } finally {
      if (previous === undefined) delete process.env.III_CONFIG_NAME;
      else process.env.III_CONFIG_NAME = previous;
    }
  });
}
