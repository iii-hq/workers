import type { Credential } from '../runtime/provider-resolve.js';

export type ChatCompletionsConfig = {
  url: string;
  provider_name: string;
  model: string;
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
  cred: Credential,
  max_tokens: number,
): ChatCompletionsConfig {
  const api_key = cred.type === 'api_key' ? cred.key : cred.access_token;
  return { url, provider_name, model, api_key, max_tokens };
}
