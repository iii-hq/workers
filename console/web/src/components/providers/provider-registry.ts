/**
 * Static provider metadata mirroring harness/src/auth-credentials/types.ts ENV_VAR_MAP.
 * Hardcoded defaults confirmed against each provider's config.ts in the harness.
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
  ['lmstudio', 'LMSTUDIO_API_KEY'],
  ['llamacpp', 'LLAMACPP_API_KEY'],
  ['opencode-zen', 'OPENCODE_ZEN_API_KEY'],
  ['opencode-go', 'OPENCODE_GO_API_KEY'],
] as const

export const ACTIVE_PROVIDERS = [
  'anthropic',
  'openai',
  'kimi',
  'lmstudio',
  'llamacpp',
] as const
export type ActiveProvider = (typeof ACTIVE_PROVIDERS)[number]

/**
 * Hardcoded defaults confirmed from each provider's config.ts:
 *   anthropic  -> https://api.anthropic.com/v1/messages           (default_max_tokens: 8192)
 *   openai     -> https://api.openai.com/v1/chat/completions      (default_max_tokens: 8192)
 *   kimi       -> https://api.moonshot.ai/v1/chat/completions     (default_max_tokens: 8192)
 *   lmstudio   -> http://localhost:1234/v1/chat/completions        (default_max_tokens: 8192)
 *   llamacpp   -> http://localhost:8080/v1/chat/completions        (default_max_tokens: 8192)
 */
export const PROVIDER_DEFAULTS: Record<
  ActiveProvider,
  { default_api_url: string; default_max_tokens: number }
> = {
  anthropic: {
    default_api_url: 'https://api.anthropic.com/v1/messages',
    default_max_tokens: 8192,
  },
  openai: {
    default_api_url: 'https://api.openai.com/v1/chat/completions',
    default_max_tokens: 8192,
  },
  kimi: {
    default_api_url: 'https://api.moonshot.ai/v1/chat/completions',
    default_max_tokens: 8192,
  },
  lmstudio: {
    default_api_url: 'http://localhost:1234/v1/chat/completions',
    default_max_tokens: 8192,
  },
  llamacpp: {
    default_api_url: 'http://localhost:8080/v1/chat/completions',
    default_max_tokens: 8192,
  },
}

export interface ProviderEntry {
  id: string
  envVar: string
  isActive: boolean
}

export const PROVIDERS: ProviderEntry[] = ENV_VAR_MAP.map(([id, envVar]) => ({
  id,
  envVar,
  isActive: (ACTIVE_PROVIDERS as readonly string[]).includes(id),
}))

/**
 * Documentation URLs for where to obtain an API key per active provider.
 * Shown as a discrete hint when the user opens the key editor with no key
 * set, and as a permanent "key console" link once a key is stored (so
 * operators rotating credentials don't have to leave the dialog to find
 * the provider's key page again).
 */
export const PROVIDER_DOCS: Record<ActiveProvider, string> = {
  anthropic: 'console.anthropic.com/settings/keys',
  openai: 'platform.openai.com/api-keys',
  kimi: 'platform.moonshot.cn/console/api-keys',
  lmstudio: 'lmstudio.ai/docs/local-server',
  llamacpp: 'github.com/ggml-org/llama.cpp/tree/master/tools/server',
}

/**
 * Human-readable labels for `PROVIDER_DOCS` links. Renders as the visible
 * link text instead of the bare domain, which truncated to unresolvable
 * fragments on narrow viewports.
 */
export const PROVIDER_DOCS_LABEL: Record<ActiveProvider, string> = {
  anthropic: 'anthropic console',
  openai: 'openai platform',
  kimi: 'moonshot platform',
  lmstudio: 'lm studio docs',
  llamacpp: 'llama.cpp server docs',
}

/**
 * Display label for providers whose internal id doesn't immediately
 * communicate the underlying service. `kimi` ≡ Moonshot AI; an operator
 * scanning the list shouldn't have to know that out of band.
 */
export const PROVIDER_DISPLAY: Partial<Record<ActiveProvider, string>> = {
  kimi: 'kimi (moonshot)',
}

/**
 * Per-provider placeholder for the api key input. Hardcoded `sk-...`
 * leaked OpenAI bias onto every provider; this gives operators a
 * format cue specific to the provider they opened.
 */
export const PROVIDER_KEY_PLACEHOLDER: Record<ActiveProvider, string> = {
  anthropic: 'sk-ant-api03-...',
  openai: 'sk-proj-... or sk-...',
  kimi: 'sk-...',
  lmstudio: 'any string (local server usually accepts anything)',
  llamacpp: 'any string (local server usually accepts anything)',
}

const LOCAL_PROVIDERS: ReadonlySet<ActiveProvider> = new Set([
  'lmstudio',
  'llamacpp',
])

const LOCAL_BASE_URL_ENV: Record<string, string> = {
  lmstudio: 'LMSTUDIO_BASE_URL',
  llamacpp: 'LLAMACPP_BASE_URL',
}

export function isLocalProvider(id: string): id is 'lmstudio' | 'llamacpp' {
  return LOCAL_PROVIDERS.has(id as ActiveProvider)
}

export function localBaseUrlEnv(id: string): string | undefined {
  return LOCAL_BASE_URL_ENV[id]
}
