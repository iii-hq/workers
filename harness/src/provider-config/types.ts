/**
 * Runtime provider overrides. Non-secret settings (base URL, max tokens)
 * that can be changed at runtime without restarting the harness. The
 * secret counterpart (API keys) lives in `auth-credentials`.
 */

export type ProviderOverrides = {
  default_api_url?: string;
  default_max_tokens?: number;
};
