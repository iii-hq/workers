export interface ConfigurationScenario {
  title: string;
  env?: string;
  stored?: unknown;
  error?: unknown;
  legacy?: boolean;
  getError?: unknown;
  malformed?: { response: unknown };
  registerError?: unknown;
  probeUpgrade?: boolean;
}
export const configurationCases: ConfigurationScenario[];
export function checkConfigurationContract<Client>(options: ConfigurationScenario & {
  register: (iii: Client) => Promise<void>;
  fetch: (iii: Client) => Promise<unknown>;
  bind?: (iii: Client, onChange: () => Promise<void>) => unknown;
  expectedId: string;
  formId: string;
  value: unknown;
  makeError?: (body: { code: string; message: string; function_id?: string }) => unknown;
}): Promise<void>;
