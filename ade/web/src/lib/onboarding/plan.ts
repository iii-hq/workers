/**
 * The wizard's decisions, kept free of I/O so they can be tested and shown
 * before anything runs: which providers to recommend from what was found on
 * this machine, and the exact list of engine actions a choice turns into.
 *
 * Every action is a `PlanStep` the user reads before pressing the button,
 * and the same steps become the activity log while they run. They read as
 * short sentences for someone exploring iii: the one thing they always name
 * is a worker being added, since each one brings new behavior to the project.
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
  DEVICE_PROVIDERS,
  type DeviceProvider,
  JUDGE_HUB_WORKER,
  JUDGE_OPTIONS,
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
  /**
   * The provider declares no credential env var: it signs in by itself (OAuth,
   * device flow, a local CLI) and the router holds no key for it, so it
   * reports `configured: false` even when its models are usable.
   */
  ownsAuthentication?: boolean
}

/** Its models are usable: it has some, and a credential or its own sign-in. */
export function servesUsableModels(provider: ProviderState): boolean {
  return (
    provider.modelCount > 0 &&
    (provider.configured || provider.ownsAuthentication === true)
  )
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

/** Signs in with a device flow from the ADE (GitHub Copilot). */
export interface DeviceChoice extends BaseChoice {
  kind: 'device'
  provider: DeviceProvider
}

/** A provider worker from the registry the wizard has no recipe for. */
export interface RegistryChoice extends BaseChoice {
  kind: 'registry'
  description: string | null
  version: string | null
}

export type ProviderChoice =
  | SubscriptionChoice
  | KeyChoice
  | DeviceChoice
  | RegistryChoice

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
      const title =
        REGISTRY_TITLES[providerId] ??
        providerId
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
        // Registry descriptions are written for worker authors; the wizard
        // only says what happens next.
        reason: 'Set it up after it is added.',
        modelCount: 0,
      }
    })
}

/** Names a slug cannot spell: brand casing and punctuation. */
const REGISTRY_TITLES: Record<string, string> = {
  llamacpp: 'llama.cpp',
  'llama-cpp': 'llama.cpp',
  'opencode-go': 'OpenCode Go',
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

  const devices: DeviceChoice[] = DEVICE_PROVIDERS.map((provider) => {
    const state = byProvider.get(provider.providerId)
    const ready = state !== undefined && servesUsableModels(state)
    return {
      kind: 'device',
      provider,
      providerId: provider.providerId,
      worker: provider.worker,
      title: provider.title,
      ready,
      installed: state?.available === true,
      recommended: false,
      reason: ready
        ? `Connected — models from ${provider.plan}.`
        : `Sign in with GitHub in your browser — uses ${provider.plan}, no API key.`,
      modelCount: state?.modelCount ?? 0,
    }
  })

  const rank = (choice: ProviderChoice) =>
    choice.ready ? 0 : choice.recommended ? 1 : 2
  // Any other running provider that already serves usable models (Copilot,
  // llama.cpp): the wizard has no recipe for it but shows it as connected.
  const known = new Set(
    [...subscriptions, ...keys, ...devices].map((choice) => choice.providerId),
  )
  const others: RegistryChoice[] = providers
    .filter(
      (state) =>
        !known.has(state.id) && state.available && servesUsableModels(state),
    )
    .map((state) => ({
      kind: 'registry',
      providerId: state.id,
      worker: `provider-${state.id}`,
      title: state.title,
      description: null,
      version: null,
      ready: true,
      installed: true,
      recommended: false,
      reason: 'Connected.',
      modelCount: state.modelCount,
    }))

  // Stable: catalog order inside each rank.
  return [...subscriptions, ...keys, ...devices, ...others]
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
  // The provider reads only the sign-in; the CLI program may be absent (a
  // desktop app signs in to the same file).
  if (tool?.signed_in) {
    return `${provider.title} is signed in on this machine — uses ${provider.plan}, no API key.`
  }
  if (!tool?.installed) {
    return `${provider.title} is not signed in on this machine.`
  }
  return tool.sign_in_note
    ? `${provider.title} is installed, but ${tool.sign_in_note}.`
    : `${provider.title} is installed but not signed in. Sign in with the ${provider.title} CLI, then scan again.`
}

function keyReason(
  provider: KeyProvider,
  detection: KeyDetection | null,
  ready: boolean,
): string {
  if (ready) return 'Connected.'
  if (detection?.stored) return 'Your key is already saved on this machine.'
  const source = preferredSource(detection)
  if (source)
    return `Found your ${provider.title} key in ${sourceLabel(source)}.`
  return 'Needs an API key.'
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
  /** Unchecked in setup: take the worker out of the project again. */
  | { kind: 'remove-workers'; workers: string[] }
  | {
      kind: 'store-secret'
      name: string
      input: KeyInput
      /** Whose key it is, as the user knows it (`Anthropic`). */
      owner: string
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
      /** What the value is for, as the user knows it (`Anthropic`, `Laya`). */
      owner: string
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
  /** Connected providers the person unchecked: removed after everything else. */
  removals: readonly ProviderChoice[] = [],
): PlanStep[] {
  const pending = selections.filter(({ choice }) => !choice.ready)
  const removing = removeStep(
    removals.map((choice) => choice.worker),
    installedWorkers,
  )
  if (pending.length === 0) return removing
  const why: Record<string, string> = {}
  const keyed = pending.filter(
    (selection) => selection.choice.kind === 'key' && selection.key,
  )
  // llm-router depends on it, so it is normally running already; a project
  // set up before that gets it here, without a question of its own.
  if (keyed.length > 0 && !installedWorkers.has(SECRETS_WORKER)) {
    why[SECRETS_WORKER] = SECRETS_WHY
  }
  for (const { choice } of pending) {
    if (choice.installed || installedWorkers.has(choice.worker)) continue
    why[choice.worker] =
      choice.kind === 'subscription'
        ? `Lets agents use ${choice.title} models, from ${choice.provider.plan}.`
        : choice.kind === 'key'
          ? `Lets agents use ${choice.title} models.`
          : `Adds ${choice.title}; finish its sign-in or key in the model picker.`
  }

  const steps: PlanStep[] = []
  const workers = Object.keys(why)
  if (workers.length > 0) steps.push({ kind: 'add-workers', workers, why })
  for (const { choice, key } of keyed) {
    if (choice.kind !== 'key' || !key) continue
    steps.push(
      keyStep(
        choice.provider.envVar,
        choice.title,
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
      owner: choice.title,
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
  return [...steps, ...removing]
}

/** One `remove-workers` step for the named workers that are running. */
function removeStep(
  workers: readonly string[],
  installedWorkers: ReadonlySet<string>,
): PlanStep[] {
  const running = [...new Set(workers)].filter((worker) =>
    installedWorkers.has(worker),
  )
  return running.length > 0
    ? [{ kind: 'remove-workers', workers: running }]
    : []
}

/** Why the secrets worker is added, when a key needs it. */
const SECRETS_WHY = 'Keeps your API keys safe, outside every file you commit.'

/**
 * Put the key where `key` says for `consumers` — or, when it is already
 * there, make sure they may read it. An import's value never passes through
 * the browser.
 */
function keyStep(
  name: string,
  owner: string,
  key: KeyInput,
  consumers: string[],
  envFile: string = DEFAULT_ENV_FILE,
): PlanStep {
  return {
    kind: 'store-secret',
    name,
    input: key,
    owner,
    consumers,
    ...(keyStore(key) === 'env' ? { envFile } : {}),
  }
}

/**
 * The option Judge answers with now: the hub's `provider` when its worker is
 * installed, else the first installed option. More than one judge worker can
 * be running (a key shared with the OpenAI provider makes that common), so the
 * first installed one is not necessarily the one in use.
 */
export function activeJudge(
  installed: ReadonlySet<string>,
  provider: string | null,
): JudgeOption | undefined {
  const running = JUDGE_OPTIONS.filter((option) => installed.has(option.worker))
  return running.find((option) => option.id === provider) ?? running[0]
}

/**
 * The workers setting Judge up with `option` adds, each with what it brings:
 * the secrets worker (only for a hosted judge's key), the `judge` hub, and
 * the option's own worker — whichever is not running yet.
 */
export function judgeWorkers(
  option: JudgeOption,
  installedWorkers: ReadonlySet<string>,
  withKey = option.envVar !== undefined,
): Record<string, string> {
  const why: Record<string, string> = {}
  if (option.envVar && withKey && !installedWorkers.has(SECRETS_WORKER)) {
    why[SECRETS_WORKER] = SECRETS_WHY
  }
  if (!installedWorkers.has(JUDGE_HUB_WORKER)) {
    why[JUDGE_HUB_WORKER] =
      'Answers the small decisions agents make along the way.'
  }
  if (!installedWorkers.has(option.worker)) {
    why[option.worker] = `Runs ${option.title}, the model behind those answers.`
  }
  return why
}

export interface JudgeSelection {
  option: JudgeOption
  /** A hosted option not running yet needs one. */
  key?: KeyInput
}

/**
 * The actions that leave Judge answering with exactly the checked options:
 * add the hub and every checked strategy not running yet (keys behind
 * references), make the first checked one the hub's default, prove it
 * answers, then remove the strategies that were unchecked — and the hub
 * with them when nothing is left to answer.
 */
export function judgePlan(
  selections: readonly JudgeSelection[],
  installedWorkers: ReadonlySet<string>,
  {
    removals = [],
    currentDefault = null,
  }: {
    /** Running strategies the user unchecked. */
    removals?: readonly JudgeOption[]
    /** The hub's `provider` setting now, when it is running. */
    currentDefault?: string | null
  } = {},
): PlanStep[] {
  const hubRunning = installedWorkers.has(JUDGE_HUB_WORKER)
  if (selections.length === 0) {
    return removeStep(
      [
        ...removals.map((option) => option.worker),
        ...(removals.length > 0 ? [JUDGE_HUB_WORKER] : []),
      ],
      installedWorkers,
    )
  }
  const why: Record<string, string> = {}
  for (const { option, key } of selections) {
    Object.assign(why, judgeWorkers(option, installedWorkers, Boolean(key)))
  }
  const steps: PlanStep[] = []
  const workers = Object.keys(why)
  if (workers.length > 0) steps.push({ kind: 'add-workers', workers, why })
  for (const { option, key } of selections) {
    if (!option.envVar || !key || installedWorkers.has(option.worker)) continue
    steps.push(
      keyStep(option.envVar, option.keyOwner ?? option.title, key, [
        option.worker,
      ]),
    )
    steps.push({
      kind: 'set-config',
      configuration: option.worker,
      path: ['api_key'],
      value: keyReference(option.envVar, key),
      owner: option.title,
    })
  }
  const primary = selections[0].option
  const changesDefault = !hubRunning || currentDefault !== primary.id
  if (changesDefault) {
    steps.push({
      kind: 'set-config',
      configuration: JUDGE_HUB_WORKER,
      path: ['provider'],
      value: primary.id,
      owner: primary.title,
    })
  }
  if (workers.length > 0 || changesDefault) {
    steps.push({
      kind: 'check-judge',
      title: primary.title,
      hosted: primary.envVar !== undefined,
    })
  }
  return [
    ...steps,
    ...removeStep(
      removals.map((option) => option.worker),
      installedWorkers,
    ),
  ]
}

/**
 * One short sentence per step, as the plan preview and the activity log show
 * it. Plain words for someone exploring iii: the worker being added is named,
 * function ids, references and configuration entries are not.
 */
export function describeStep(step: PlanStep): string {
  switch (step.kind) {
    case 'add-workers':
      return step.workers.length === 1
        ? `Add the ${step.workers[0]} worker`
        : `Add ${step.workers.length} workers: ${joinNames(step.workers)}`
    case 'remove-workers':
      return step.workers.length === 1
        ? `Remove the ${step.workers[0]} worker`
        : `Remove ${step.workers.length} workers: ${joinNames(step.workers)}`
    case 'store-secret': {
      const envFile = step.envFile ?? DEFAULT_ENV_FILE
      if (step.input.mode === 'stored') {
        return `Use your ${step.owner} key already saved on this machine`
      }
      if (step.input.mode === 'env') {
        return `Use your ${step.owner} key from this project’s ${envFile}`
      }
      return keyStore(step.input) === 'env'
        ? `Save your ${step.owner} key in this project’s ${envFile}`
        : `Store your ${step.owner} key encrypted on this machine`
    }
    case 'set-config':
      return step.path[step.path.length - 1] === 'api_key'
        ? `Connect ${step.owner} with that key`
        : `Have Judge answer with ${step.owner}`
    case 'wait-models':
      return `Check that ${step.title} models are ready`
    case 'check-judge':
      return `Check that ${step.title} answers`
  }
}

/**
 * The engine operation behind a step, as the setup log prints it under the
 * sentence: the command a terminal would show.
 */
export function stepDetail(step: PlanStep): string {
  switch (step.kind) {
    case 'add-workers':
      return `compose::add ${step.workers.join(' ')}`
    case 'remove-workers':
      return `compose::remove ${step.workers.join(' ')}`
    case 'store-secret':
      return `secrets::${step.input.mode === 'paste' ? 'set' : 'access'} ${step.name}`
    case 'set-config':
      return `configuration::set ${step.configuration} ${step.path.join('.')}`
    case 'wait-models':
      return `router::models::list provider=${step.providerId}`
    case 'check-judge':
      return 'judge::models::list'
  }
}

/** `a`, `a and b`, `a, b and c`. */
function joinNames(names: readonly string[]): string {
  if (names.length <= 1) return names.join('')
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
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
