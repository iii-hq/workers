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
    keysUrl: 'https://typesafe.ai',
    recommended: true,
  },
  {
    id: 'laya',
    worker: 'judge-laya',
    title: 'Laya',
    summary: 'Small ModernBERT decision checkpoints that run on this machine.',
    runs: 'Local · CPU is enough · downloads its checkpoints once',
  },
  {
    id: 'decider',
    worker: 'judge-decider',
    title: 'Decider',
    summary: 'A 4B decision model served by llama.cpp on this machine.',
    runs: 'Local · a GPU is recommended · downloads a GGUF model once',
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
  /** The starter task offered once setup is done. */
  starter: { label: string; prompt: string }
}

export const PILLARS: readonly Pillar[] = [
  {
    id: 'extensible',
    title: 'Extensible',
    line: 'The harness builds tools for itself — a kanban board for its own work, a stories view to review your UI.',
    starter: {
      label: 'Give the agents a kanban board',
      prompt:
        'Add the kanban worker to this project and open its board. Then create three tickets that break down what we should build first, and explain how agents pick tickets up from the board.',
    },
  },
  {
    id: 'discoverable',
    title: 'Discoverable',
    line: 'It runs inside your backend: it knows every function your workers register and can call them directly.',
    starter: {
      label: 'Show me what my backend can do',
      prompt:
        'List the workers registered in this project and the functions each one exposes. Then call one read-only function to show me how calling my backend directly works.',
    },
  },
  {
    id: 'optimized',
    title: 'Optimized',
    line: 'Judge finds the function or skill that matters for each step, so prompts stay small and precise.',
    starter: {
      label: 'Find the right function for a job',
      prompt:
        'Search this project for the function that best fits "store a value and read it back later", explain why it was chosen over the alternatives, and use it once.',
    },
  },
  {
    id: 'composable',
    title: 'Composable',
    line: 'Most things already exist in the registry at workers.iii.dev — add a worker instead of building it.',
    starter: {
      label: 'Find a worker in the registry',
      prompt:
        'Browse the workers registry for workers that would be useful in this project, recommend three with one line each on why, and add the one I pick.',
    },
  },
  {
    id: 'reactive',
    title: 'Reactive',
    line: 'Agents subscribe to triggers and wake the moment something happens — no polling loops.',
    starter: {
      label: 'React to an event',
      prompt:
        'Set up a trigger that wakes you whenever a value changes in the state worker under scope "demo". Then write a value there so we can watch the trigger fire, and tell me what you received.',
    },
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
