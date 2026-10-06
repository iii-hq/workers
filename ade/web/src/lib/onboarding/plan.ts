/**
 * The wizard's decisions, kept free of I/O so they can be tested and shown
 * before anything runs: which providers to recommend from what was found on
 * this machine, and the exact list of engine actions a choice turns into.
 *
 * Transparency is the point of the plan: every worker the wizard adds, every
 * secret it stores and every configuration value it writes is a `PlanStep`
 * the user reads before pressing the button, and the same steps become the
 * activity log while they run.
 */

import {
  DEFAULT_ENV_FILE,
  type KeyDetection,
  type KeyInput,
  type KeySource,
  type KeySourceKind,
  keyReference,
  keyStore,
  preferredSource,
  sourceLabel,
} from '@/lib/secrets'
import {
  JUDGE_HUB_WORKER,
  type JudgeOption,
  KEY_PROVIDERS,
  type KeyProvider,
  ROUTER_CONFIGURATION,
  SECRETS_WORKER,
  SUBSCRIPTION_PROVIDERS,
  type SubscriptionProvider,
} from './catalog'

export type { KeyDetection, KeyInput, KeySource, KeySourceKind }
export { preferredSource, sourceLabel }

/** One CLI found (or not) by `console::onboarding::scan`. */
export interface ToolScan {
  id: string
  name: string
  installed: boolean
  binary_path?: string | null
  version?: string | null
  signed_in: boolean
  credentials_path?: string | null
  sign_in_note?: string | null
  provider_worker: string
}

/** What the router reports for one provider, joined with its model count. */
export interface ProviderState {
  id: string
  title: string
  configured: boolean
  available: boolean
  modelCount: number
  credentialSource?: string
  credentialRef?: string
  credentialError?: string
}

interface BaseChoice {
  providerId: string
  worker: string
  title: string
  /** The router already serves models from it. */
  ready: boolean
  /** The worker is running (the router knows the provider). */
  installed: boolean
  recommended: boolean
  /** One sentence: why it is (or is not) recommended. */
  reason: string
  modelCount: number
}

export interface SubscriptionChoice extends BaseChoice {
  kind: 'subscription'
  provider: SubscriptionProvider
  tool: ToolScan | null
  /** Signed in locally, so adding the worker is all it takes. */
  usable: boolean
}

export interface KeyChoice extends BaseChoice {
  kind: 'key'
  provider: KeyProvider
  detection: KeyDetection | null
  credentialError?: string
}

/** A provider worker from the registry the wizard has no recipe for. */
export interface RegistryChoice extends BaseChoice {
  kind: 'registry'
  description: string | null
  version: string | null
}

export type ProviderChoice = SubscriptionChoice | KeyChoice | RegistryChoice

/** The registry row a `RegistryChoice` is built from. */
export interface RegistryProviderRow {
  name: string
  description: string | null
  version: string | null
}

/**
 * Registry provider workers that are neither in the wizard's catalog nor
 * already running: offered as "add the worker, configure it afterwards".
 */
export function registryChoices(
  rows: readonly RegistryProviderRow[],
  installedWorkers: ReadonlySet<string>,
  known: readonly ProviderChoice[],
): RegistryChoice[] {
  const knownWorkers = new Set(known.map((choice) => choice.worker))
  return rows
    .filter(
      (row) => !knownWorkers.has(row.name) && !installedWorkers.has(row.name),
    )
    .map((row) => {
      const providerId = row.name.replace(/^provider-/, '')
      const title = providerId
        .split(/[-_]/)
        .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
        .join(' ')
      return {
        kind: 'registry',
        providerId,
        worker: row.name,
        title,
        description: row.description,
        version: row.version,
        ready: false,
        installed: false,
        recommended: false,
        reason:
          firstSentence(row.description) ??
          'A provider worker from the registry.',
        modelCount: 0,
      }
    })
}

function firstSentence(text: string | null): string | null {
  if (!text) return null
  const end = text.search(/[.;](\s|$)/)
  return (end > 0 ? text.slice(0, end) : text).trim() || null
}

export interface ChoiceInputs {
  tools: readonly ToolScan[]
  providers: readonly ProviderState[]
  detections: readonly KeyDetection[]
}

/**
 * Every provider the wizard offers, recommended ones first. Subscription
 * providers lead when their CLI is signed in on this machine — no key to
 * find or paste; key providers are recommended when a key was found.
 */
export function providerChoices({
  tools,
  providers,
  detections,
}: ChoiceInputs): ProviderChoice[] {
  const byProvider = new Map(providers.map((entry) => [entry.id, entry]))
  const byName = new Map(detections.map((entry) => [entry.name, entry]))

  const subscriptions: SubscriptionChoice[] = SUBSCRIPTION_PROVIDERS.map(
    (provider) => {
      const tool = tools.find((entry) => entry.id === provider.toolId) ?? null
      const state = byProvider.get(provider.providerId)
      const ready = (state?.modelCount ?? 0) > 0
      const usable = tool?.signed_in === true
      return {
        kind: 'subscription',
        provider,
        tool,
        providerId: provider.providerId,
        worker: provider.worker,
        title: provider.title,
        ready,
        installed: state?.available === true,
        usable,
        recommended: usable || ready,
        reason: subscriptionReason(provider, tool, ready),
        modelCount: state?.modelCount ?? 0,
      }
    },
  )

  const keys: KeyChoice[] = KEY_PROVIDERS.map((provider) => {
    const state = byProvider.get(provider.providerId)
    const detection = byName.get(provider.envVar) ?? null
    const ready = (state?.modelCount ?? 0) > 0 && state?.configured !== false
    const found =
      detection !== null && (detection.stored || detection.sources.length > 0)
    return {
      kind: 'key',
      provider,
      detection,
      providerId: provider.providerId,
      worker: provider.worker,
      title: provider.title,
      ready,
      installed: state?.available === true,
      recommended: ready || found,
      reason: keyReason(provider, detection, ready),
      modelCount: state?.modelCount ?? 0,
      credentialError: state?.credentialError,
    }
  })

  const rank = (choice: ProviderChoice) =>
    choice.ready ? 0 : choice.recommended ? 1 : 2
  // Stable: catalog order inside each rank.
  return [...subscriptions, ...keys]
    .map((choice, index) => ({ choice, index }))
    .sort((a, b) => rank(a.choice) - rank(b.choice) || a.index - b.index)
    .map(({ choice }) => choice)
}

function subscriptionReason(
  provider: SubscriptionProvider,
  tool: ToolScan | null,
  ready: boolean,
): string {
  if (ready) return `Connected — models from ${provider.plan}.`
  if (!tool?.installed) {
    return `${provider.title} was not found on this machine.`
  }
  if (!tool.signed_in) {
    return tool.sign_in_note
      ? `${provider.title} is installed, but ${tool.sign_in_note}.`
      : `${provider.title} is installed but not signed in. Sign in with the ${provider.title} CLI, then scan again.`
  }
  return `${provider.title} is signed in on this machine — uses ${provider.plan}, no API key.`
}

function keyReason(
  provider: KeyProvider,
  detection: KeyDetection | null,
  ready: boolean,
): string {
  if (ready) return 'Connected.'
  if (detection?.stored) {
    return `${provider.envVar} is already in the secrets store.`
  }
  const source = preferredSource(detection)
  if (source) return `Found ${provider.envVar} in ${sourceLabel(source)}.`
  return `Needs an API key (${provider.envVar}).`
}

/* ------------------------------------------------------------------ */
/*  Plans                                                              */
/* ------------------------------------------------------------------ */

export type PlanStep =
  | {
      kind: 'add-workers'
      /** Registry names, in the order they are explained. */
      workers: string[]
      why: Record<string, string>
    }
  | {
      kind: 'store-secret'
      name: string
      input: KeyInput
      /** Human description of where the value comes from. */
      from: string
      consumers: string[]
      /** The secrets worker's env file, by name, for an env store key. */
      envFile?: string
    }
  | {
      kind: 'set-config'
      /** Configuration family (`llm-router`, `judge`, `judge-typesafe`). */
      configuration: string
      /** Dotted path inside the entry value. */
      path: string[]
      value: string
    }
  | { kind: 'wait-models'; providerId: string; title: string }
  /** Ask the judge hub for its models: proves the strategy answers, key included. */
  | { kind: 'check-judge'; title: string; hosted: boolean }

export interface ProviderSelection {
  choice: ProviderChoice
  /** Required for a key provider that is not ready. */
  key?: KeyInput
}

/**
 * The actions that connect the selected providers, in execution order: one
 * `compose::add` for every missing worker (the secrets worker first when a
 * key is involved), then each key into the secrets store and its reference
 * into the router's configuration, then the wait for models.
 */
export function connectPlan(
  selections: readonly ProviderSelection[],
  installedWorkers: ReadonlySet<string>,
  /** The secrets worker's env file, by name. */
  envFile: string = DEFAULT_ENV_FILE,
): PlanStep[] {
  const pending = selections.filter(({ choice }) => !choice.ready)
  if (pending.length === 0) return []
  const why: Record<string, string> = {}
  const keyed = pending.filter(
    (selection) => selection.choice.kind === 'key' && selection.key,
  )
  // llm-router depends on it, so it is normally running already; a project
  // set up before that gets it here, without a question of its own.
  if (keyed.length > 0 && !installedWorkers.has(SECRETS_WORKER)) {
    why[SECRETS_WORKER] =
      'Keeps API keys encrypted or in this project’s .env, outside every file you commit.'
  }
  for (const { choice } of pending) {
    if (choice.installed || installedWorkers.has(choice.worker)) continue
    why[choice.worker] =
      choice.kind === 'subscription'
        ? `Serves ${choice.title} models from ${choice.provider.plan}.`
        : choice.kind === 'key'
          ? `Serves ${choice.title} models through llm-router.`
          : `Adds ${choice.title}; finish its sign-in or key in the model picker.`
  }

  const steps: PlanStep[] = []
  const workers = Object.keys(why)
  if (workers.length > 0) steps.push({ kind: 'add-workers', workers, why })
  for (const { choice, key } of keyed) {
    if (choice.kind !== 'key' || !key) continue
    steps.push(
      ...keyReferenceSteps(
        choice.provider.envVar,
        key,
        ['llm-router'],
        envFile,
      ),
    )
    steps.push({
      kind: 'set-config',
      configuration: ROUTER_CONFIGURATION,
      path: ['providers', choice.providerId, 'api_key'],
      value: keyReference(choice.provider.envVar, key),
    })
  }
  // A registry provider the wizard has no recipe for may need a sign-in
  // first; its models are not waited for.
  for (const { choice } of pending) {
    if (choice.kind === 'registry') continue
    steps.push({
      kind: 'wait-models',
      providerId: choice.providerId,
      title: choice.title,
    })
  }
  return steps
}

/**
 * Put the key where `key` says for `consumers` — or, when it is already
 * there, make sure they may read it. An import's value never passes through
 * the browser.
 */
function keyReferenceSteps(
  name: string,
  key: KeyInput,
  consumers: string[],
  envFile: string = DEFAULT_ENV_FILE,
): PlanStep[] {
  return [
    {
      kind: 'store-secret',
      name,
      input: key,
      from:
        key.mode === 'paste'
          ? 'the key you pasted'
          : key.mode === 'stored'
            ? 'the secrets store'
            : key.mode === 'env'
              ? `this project’s ${envFile}`
              : sourceLabel(key.source),
      consumers,
      ...(keyStore(key) === 'env' ? { envFile } : {}),
    },
  ]
}

/** The actions that set Judge up with one strategy. */
export function judgePlan(
  option: JudgeOption,
  key: KeyInput | undefined,
  installedWorkers: ReadonlySet<string>,
): PlanStep[] {
  const why: Record<string, string> = {}
  if (option.envVar && key && !installedWorkers.has(SECRETS_WORKER)) {
    why[SECRETS_WORKER] =
      'Stores API keys encrypted, outside every file you commit.'
  }
  if (!installedWorkers.has(JUDGE_HUB_WORKER)) {
    why[JUDGE_HUB_WORKER] =
      'The hub harness, function search and the browser ask for decisions.'
  }
  if (!installedWorkers.has(option.worker)) {
    why[option.worker] = `Answers those decisions with ${option.title}.`
  }
  const steps: PlanStep[] = []
  const workers = Object.keys(why)
  if (workers.length > 0) steps.push({ kind: 'add-workers', workers, why })
  if (option.envVar && key) {
    steps.push(...keyReferenceSteps(option.envVar, key, [option.worker]))
    steps.push({
      kind: 'set-config',
      configuration: option.worker,
      path: ['api_key'],
      value: keyReference(option.envVar, key),
    })
  }
  steps.push({
    kind: 'set-config',
    configuration: JUDGE_HUB_WORKER,
    path: ['provider'],
    value: option.id,
  })
  steps.push({
    kind: 'check-judge',
    title: option.title,
    hosted: option.envVar !== undefined,
  })
  return steps
}

/** One line per step, as the plan preview and the activity log show it. */
export function describeStep(step: PlanStep): {
  title: string
  detail: string
} {
  switch (step.kind) {
    case 'add-workers':
      return {
        title:
          step.workers.length === 1
            ? `Add the ${step.workers[0]} worker`
            : `Add ${step.workers.length} workers`,
        detail: `compose::add ${step.workers.join(' ')}`,
      }
    case 'store-secret': {
      const reference = keyReference(step.name, step.input)
      const readers = step.consumers.join(', ')
      if (step.input.mode === 'stored' || step.input.mode === 'env') {
        return {
          title:
            step.input.mode === 'env'
              ? `Let ${readers} read ${step.name} from ${step.from}`
              : `Let ${readers} read ${step.name}`,
          detail: `secrets::access ${step.name} → ${reference}`,
        }
      }
      const call = `secrets::${step.input.mode === 'import' ? 'import' : 'set'}`
      return keyStore(step.input) === 'env'
        ? {
            title: `Write ${step.name} to this project’s ${step.envFile ?? DEFAULT_ENV_FILE}, from ${step.from}`,
            detail: `${call} ${step.name} store=env → ${reference}`,
          }
        : {
            title: `Store ${step.name} encrypted, from ${step.from}`,
            detail: `${call} ${step.name} → ${reference}`,
          }
    }
    case 'set-config':
      return {
        title: `Point ${step.configuration} at ${step.value}`,
        detail: `${step.configuration} · ${step.path.join('.')} = ${step.value}`,
      }
    case 'wait-models':
      return {
        title: `Wait for ${step.title} models`,
        detail: `router::models::list provider=${step.providerId}`,
      }
    case 'check-judge':
      return {
        title: `Ask ${step.title} for its models`,
        detail: 'judge::models::list',
      }
  }
}

/** Set `path` inside a JSON object value, creating objects on the way. */
export function setPath(
  value: unknown,
  path: readonly string[],
  leaf: unknown,
): Record<string, unknown> {
  const root: Record<string, unknown> =
    value && typeof value === 'object' && !Array.isArray(value)
      ? { ...(value as Record<string, unknown>) }
      : {}
  let cursor = root
  path.forEach((segment, index) => {
    if (index === path.length - 1) {
      cursor[segment] = leaf
      return
    }
    const next = cursor[segment]
    const copy =
      next && typeof next === 'object' && !Array.isArray(next)
        ? { ...(next as Record<string, unknown>) }
        : {}
    cursor[segment] = copy
    cursor = copy
  })
  return root
}
