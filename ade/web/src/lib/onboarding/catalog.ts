/**
 * What the setup wizard knows before it asks the engine anything: the
 * subscription providers a local CLI sign-in unlocks, the API-key env var
 * each key provider declares, the judge strategies, and the five things the
 * harness should show a new user as early as possible.
 *
 * Worker names are registry slugs — what `compose::add` resolves. A
 * development checkout can point any of them at a local directory through
 * the console configuration's `onboarding.worker_sources` map (see
 * `workerSource`).
 */

/** A provider that turns a local CLI sign-in into models, no API key. */
export interface SubscriptionProvider {
  /** `console::onboarding::scan` tool id. */
  toolId: 'claude-code' | 'codex'
  /** Provider id the worker declares to llm-router. */
  providerId: string
  worker: string
  title: string
  /** What the user pays with, in their words. */
  plan: string
}

export const SUBSCRIPTION_PROVIDERS: readonly SubscriptionProvider[] = [
  {
    toolId: 'claude-code',
    providerId: 'claude-code',
    worker: 'provider-claude-code',
    title: 'Claude Code',
    plan: 'your Claude Pro or Max plan',
  },
  {
    toolId: 'codex',
    providerId: 'openai-codex',
    worker: 'provider-openai-codex',
    title: 'Codex',
    plan: 'your ChatGPT plan',
  },
]

/**
 * A provider that signs in with a device flow from the ADE: the worker hands
 * out a code, the person enters it on `provider`'s page, the ADE polls.
 */
export interface DeviceProvider {
  providerId: string
  worker: string
  title: string
  plan: string
  /** Returns `{ user_code, verification_uri, device_code }`. */
  loginStart: string
  /** Takes `{ device_code }`, returns `{ status }`. */
  loginPoll: string
}

export const DEVICE_PROVIDERS: readonly DeviceProvider[] = [
  {
    providerId: 'github-copilot',
    worker: 'provider-github-copilot',
    title: 'GitHub Copilot',
    plan: 'your GitHub Copilot plan',
    loginStart: 'provider::github-copilot::login::start',
    loginPoll: 'provider::github-copilot::login::poll',
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

/** One way the `judge` hub can answer. */
export interface JudgeOption {
  /** The hub's `provider` setting. */
  id: string
  worker: string
  title: string
  summary: string
  /** Where it runs and what it costs the machine. */
  runs: string
  /** A hosted judge needs a key; local judges download a model instead. */
  envVar?: string
  /** Who issues that key, as the user knows them (`TypeSafe`). */
  keyOwner?: string
  keysUrl?: string
  recommended?: boolean
}

export const JUDGE_OPTIONS: readonly JudgeOption[] = [
  {
    id: 'typesafe',
    worker: 'judge-typesafe',
    title: 'Jev by TypeSafe',
    summary:
      'A model trained only to make typed decisions. Fast and the most accurate option.',
    runs: 'Hosted · needs a TypeSafe API key',
    envVar: 'TYPESAFE_API_KEY',
    keyOwner: 'TypeSafe',
    keysUrl: 'https://typesafe.ai',
    recommended: true,
  },
  {
    id: 'laya',
    worker: 'judge-laya',
    title: 'Laya',
    summary: 'Small decision models that run on this machine.',
    runs: 'Runs on this machine · CPU is enough · downloads its models once',
  },
  {
    id: 'decider',
    worker: 'judge-decider',
    title: 'Decider',
    summary: 'A larger decision model that runs on this machine.',
    runs: 'Runs on this machine · a GPU is recommended · downloads its model once',
  },
  {
    id: 'clef',
    worker: 'judge-clef',
    title: 'Clef by Cloudflare',
    summary:
      "Cloudflare's decision model. Decides every question of a call in one pass, on this machine.",
    runs: 'Runs on this machine · a GPU is recommended · downloads its 6.5 GB model once',
  },
]

export const JUDGE_HUB_WORKER = 'judge'
export { SECRETS_WORKER } from '@/lib/secrets'
/** The router entry provider references are written into. */
export const ROUTER_CONFIGURATION = 'llm-router'

/** Where Judge already works for the harness once it is installed. */
export const JUDGE_USES: readonly { where: string; what: string }[] = [
  {
    where: 'Function search',
    what: 'picks the right function among everything your backend registers, instead of stuffing every schema into the prompt',
  },
  {
    where: 'Argument repair',
    what: 'fixes a malformed tool call before it fails, without another round trip to the model',
  },
  {
    where: 'Browser automation',
    what: 'chooses which element on a page an agent should act on',
  },
]

export type PillarId =
  | 'extensible'
  | 'discoverable'
  | 'optimized'
  | 'composable'
  | 'reactive'

export interface Pillar {
  id: PillarId
  title: string
  /** One line for the welcome step. */
  line: string
}

export const PILLARS: readonly Pillar[] = [
  {
    id: 'extensible',
    title: 'Extensible',
    line: 'The harness builds tools for itself — a kanban board for its own work, a stories view to review your UI.',
  },
  {
    id: 'discoverable',
    title: 'Discoverable',
    line: 'It runs inside your backend: it knows every function your workers register and can call them directly.',
  },
  {
    id: 'optimized',
    title: 'Optimized',
    line: 'Judge finds the function or skill that matters for each step, so prompts stay small and precise.',
  },
  {
    id: 'composable',
    title: 'Composable',
    line: 'Most things already exist in the registry at workers.iii.dev — add a worker instead of building it.',
  },
  {
    id: 'reactive',
    title: 'Reactive',
    line: 'Agents subscribe to triggers and wake the moment something happens — no polling loops.',
  },
]

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
