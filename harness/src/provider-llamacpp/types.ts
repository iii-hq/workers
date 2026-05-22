import type { Credential } from '../auth-credentials/types.js';

export type ChatCompletionsConfig = {
  url: string;
  provider_name: string;
  model: string;
  /** Empty string when llama-server is running without --api-key (the default). */
  api_key: string;
  /** Defaults to "Authorization". */
  auth_header_name?: string;
  /** Defaults to "Bearer ". */
  auth_value_prefix?: string;
  extra_headers?: Array<readonly [string, string]>;
  max_tokens: number;
};

export function configFromCredential(
  url: string,
  provider_name: string,
  model: string,
  cred: Credential | null,
  max_tokens: number,
): ChatCompletionsConfig {
  const api_key =
    cred === null ? '' : cred.type === 'api_key' ? cred.key : cred.access_token;
  return { url, provider_name, model, api_key, max_tokens };
}
