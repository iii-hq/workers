/**
 * Credential types. Wire-identical to
 * `auth-credentials/src/lib.rs::Credential` and friends.
 */

export type ApiKeyCredential = {
  type: 'api_key';
  key: string;
};

export type OAuthCredential = {
  type: 'oauth';
  access_token: string;
  refresh_token?: string;
  expires_at?: number;
  scopes?: string[];
  provider_extra?: unknown;
};

export type Credential = ApiKeyCredential | OAuthCredential;

export type CredentialType = 'ApiKey' | 'OAuth';

export type AuthSource = 'stored' | 'runtime' | 'environment' | 'fallback';

export type AuthStatus = {
  configured: boolean;
  source?: AuthSource;
  label?: string;
};

export type EnvKeyMatch = {
  provider: string;
  env_var: string;
  key_prefix: string;
};

/**
 * Per-provider environment variable map. Order matches
 * `auth-credentials/src/lib.rs::env_var_map`.
 */
export const ENV_VAR_MAP: ReadonlyArray<readonly [string, string]> = [
  ['anthropic', 'ANTHROPIC_API_KEY'],
  ['openai', 'OPENAI_API_KEY'],
  ['openai-codex', 'OPENAI_API_KEY'],
  ['azure-openai', 'AZURE_OPENAI_API_KEY'],
  ['google', 'GOOGLE_API_KEY'],
  ['google-vertex', 'GOOGLE_APPLICATION_CREDENTIALS'],
  ['amazon-bedrock', 'AWS_ACCESS_KEY_ID'],
  ['mistral', 'MISTRAL_API_KEY'],
  ['groq', 'GROQ_API_KEY'],
  ['cerebras', 'CEREBRAS_API_KEY'],
  ['xai', 'XAI_API_KEY'],
  ['deepseek', 'DEEPSEEK_API_KEY'],
  ['openrouter', 'OPENROUTER_API_KEY'],
  ['vercel-ai-gateway', 'VERCEL_AI_GATEWAY_API_KEY'],
  ['zai', 'ZAI_API_KEY'],
  ['minimax', 'MINIMAX_API_KEY'],
  ['huggingface', 'HF_TOKEN'],
  ['fireworks', 'FIREWORKS_API_KEY'],
  ['kimi', 'MOONSHOT_API_KEY'],
  ['kimi-coding', 'MOONSHOT_API_KEY'],
  ['opencode-zen', 'OPENCODE_ZEN_API_KEY'],
  ['opencode-go', 'OPENCODE_GO_API_KEY'],
] as const;

export function envVarFor(provider: string): string | undefined {
  return ENV_VAR_MAP.find(([p]) => p === provider)?.[1];
}
