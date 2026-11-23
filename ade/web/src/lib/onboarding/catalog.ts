/**
 * What the setup wizard knows before it asks the engine anything: the
 * subscription providers a local CLI sign-in unlocks and the API-key env var
 * each key provider declares.
 *
 * Worker names are registry slugs — what `compose::add` resolves. A
 * development checkout can point any of them at a local directory through
 * the console configuration's `onboarding.worker_sources` map (see
 * `workerSource`).
 */

/** A provider that turns a local CLI sign-in into models, no API key. */
export interface SubscriptionProvider {
  /** `ade::onboarding::scan` tool id. */
  toolId: 'claude-code' | 'codex'
  /** Provider id the worker declares to llm-router. */
  providerId: string
  worker: string
  title: string
  /** What the user pays with, in their words. */
  plan: string
  /** Installs the CLI when it is not on this machine. */
  install: string
  /** Signs the CLI in to the subscription. */
  signIn: string
  /** What to do once `signIn` is running, when it is not the whole story. */
  signInNote?: string
  /** Signs in from the ADE instead, when the provider rejects the CLI's sign-in. */
  deviceLogin?: DeviceProvider
}

export const SUBSCRIPTION_PROVIDERS: readonly SubscriptionProvider[] = [
  {
    toolId: 'claude-code',
    providerId: 'claude-code',
    worker: 'provider-claude-code',
    title: 'Claude Code',
    plan: 'your Claude Pro or Max plan',
    install: 'npm install -g @anthropic-ai/claude-code',
    signIn: 'claude',
    signInNote: 'then type /login',
  },
  {
    toolId: 'codex',
    providerId: 'openai-codex',
    worker: 'provider-openai-codex',
    title: 'Codex',
    plan: 'your ChatGPT plan',
    install: 'npm install -g @openai/codex',
    signIn: 'codex login',
    deviceLogin: {
      providerId: 'openai-codex',
      worker: 'provider-openai-codex',
      title: 'Codex',
      plan: 'your ChatGPT plan',
      site: 'ChatGPT',
      loginStart: 'provider::openai-codex::login::start',
      loginPoll: 'provider::openai-codex::login::poll',
      handle: 'login_id',
    },
  },
]

/**
 * A provider that signs in with a device flow from the ADE: the worker hands
 * out a code, the person enters it on `site`'s page, the ADE polls.
 */
export interface DeviceProvider {
  providerId: string
  worker: string
  title: string
  plan: string
  /** Where the code is entered, in the person's words. */
  site: string
  /** Returns `{ user_code, verification_uri }` and the `handle`. */
  loginStart: string
  /** Takes `{ [handle] }`, returns `{ status }`. */
  loginPoll: string
  /** What `loginStart` names the sign-in that `loginPoll` follows. */
  handle: 'device_code' | 'login_id'
}

export const DEVICE_PROVIDERS: readonly DeviceProvider[] = [
  {
    providerId: 'github-copilot',
    worker: 'provider-github-copilot',
    title: 'GitHub Copilot',
    plan: 'your GitHub Copilot plan',
    site: 'GitHub',
    loginStart: 'provider::github-copilot::login::start',
    loginPoll: 'provider::github-copilot::login::poll',
    handle: 'device_code',
  },
]

/** An API-key provider. `envVar` matches what the worker declares to the router. */
export interface KeyProvider {
  providerId: string
  worker: string
  title: string
  envVar: string
  /** Where to create a key. */
  keysUrl?: string
}

/**
 * Key providers the wizard can offer before their worker is installed — the
 * env var has to be known up front to look for an existing key. Ordered by
 * how often a new user already has a key.
 */
export const KEY_PROVIDERS: readonly KeyProvider[] = [
  {
    providerId: 'anthropic',
    worker: 'provider-anthropic',
    title: 'Anthropic',
    envVar: 'ANTHROPIC_API_KEY',
    keysUrl: 'https://console.anthropic.com/settings/keys',
  },
  {
    providerId: 'openai',
    worker: 'provider-openai',
    title: 'OpenAI',
    envVar: 'OPENAI_API_KEY',
    keysUrl: 'https://platform.openai.com/api-keys',
  },
  {
    providerId: 'openrouter',
    worker: 'provider-openrouter',
    title: 'OpenRouter',
    envVar: 'OPENROUTER_API_KEY',
    keysUrl: 'https://openrouter.ai/settings/keys',
  },
  {
    providerId: 'deepseek',
    worker: 'provider-deepseek',
    title: 'DeepSeek',
    envVar: 'DEEPSEEK_API_KEY',
    keysUrl: 'https://platform.deepseek.com/api_keys',
  },
  {
    providerId: 'xai',
    worker: 'provider-xai',
    title: 'xAI',
    envVar: 'XAI_API_KEY',
    keysUrl: 'https://console.x.ai',
  },
  {
    providerId: 'kimi',
    worker: 'provider-kimi',
    title: 'Kimi',
    envVar: 'MOONSHOT_API_KEY',
    keysUrl: 'https://platform.moonshot.ai/console/api-keys',
  },
  {
    providerId: 'zai',
    worker: 'provider-zai',
    title: 'Z.AI',
    envVar: 'ZAI_API_KEY',
    keysUrl: 'https://z.ai/manage-apikey/apikey-list',
  },
]

export { SECRETS_WORKER } from '@/lib/secrets'
/** The router entry provider references are written into. */
export const ROUTER_CONFIGURATION = 'llm-router'

/**
 * The guided tour that follows setup: the `learn-iii` worker's card walks
 * through the ADE stage by stage, with the models just connected. The tour
 * gate adds the worker when the tour comes due, then opens it.
 */
export const TOUR_WORKER = 'onboarding'
export const TOUR_PAGE = 'learn-iii'

/** The console configuration key that overrides worker sources (development). */
export const WORKER_SOURCES_KEY = 'onboarding'

/**
 * The `compose::add` input for a worker: its registry name, or what a
 * development checkout maps it to under `onboarding.worker_sources.<name>` in
 * the console configuration — a local directory, or a whole container object
 * (`{ worker, scripts: { run } }`) for a worker whose manifest has no start
 * script.
 */
export function workerSource(
  name: string,
  consoleConfig: Record<string, unknown> | null,
): string | Record<string, unknown> {
  const onboarding = consoleConfig?.[WORKER_SOURCES_KEY]
  if (!onboarding || typeof onboarding !== 'object') return name
  const sources = (onboarding as Record<string, unknown>).worker_sources
  if (!sources || typeof sources !== 'object') return name
  const source = (sources as Record<string, unknown>)[name]
  if (typeof source === 'string' && source.trim()) return source.trim()
  if (
    source &&
    typeof source === 'object' &&
    !Array.isArray(source) &&
    typeof (source as Record<string, unknown>).worker === 'string'
  ) {
    return source as Record<string, unknown>
  }
  return name
}

export { secretRef } from '@/lib/secrets'
