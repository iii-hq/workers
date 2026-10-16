/**
 * The registry kits the project's template asks the setup wizard to install
 * (`kits:` in `onboarding.yaml`, read with the example prompts through
 * `console::onboarding::prompts`). The wizard starts them in the background
 * as soon as it opens and they keep going after it closes: one
 * `directory::download-kit` with `apply: true` per kit, one at a time (the
 * Directory applies one plan at a time). A kit already installed answers
 * `up_to_date`; a plan that needs review (a profile with the same id, a
 * merge) waits in Directory → Kits.
 */

import { getIiiClient } from '@/lib/iii-client'
import { isMissingFunction } from '@/lib/secrets'
import { readableError } from './api'
import { PROMPTS_FUNCTION } from './prompts'

export type KitInstallState = 'installing' | 'installed' | 'review' | 'failed'

export interface KitInstall {
  /** Registry id, `<handle>/<name>` with an optional `@<version>`. */
  kit: string
  state: KitInstallState
  /** Why it needs review or failed, as the Directory said it. */
  message?: string
  /** The plan waiting for review in Directory → Kits. */
  planId?: string
}

const DOWNLOAD_KIT = 'directory::download-kit'
/** An apply waits for Compose to add the kit's workers (up to 15 min). */
export const KIT_INSTALL_TIMEOUT_MS = 20 * 60_000

/** The kits list of the backend's answer, read defensively. */
export function parseOnboardingKits(value: unknown): string[] {
  const list =
    value && typeof value === 'object'
      ? (value as { kits?: unknown }).kits
      : undefined
  if (!Array.isArray(list)) return []
  const kits = list
    .filter((kit): kit is string => typeof kit === 'string')
    .map((kit) => kit.trim())
    .filter((kit) => kit.length > 0)
  return [...new Set(kits)]
}

async function fetchOnboardingKits(): Promise<string[]> {
  const client = await getIiiClient()
  try {
    const result = await client.trigger<unknown>(
      PROMPTS_FUNCTION,
      {},
      { timeoutMs: 5_000 },
    )
    return parseOnboardingKits(result)
  } catch (error) {
    if (isMissingFunction(error)) return []
    throw error
  }
}

/**
 * Install one kit; `null` when this project's Directory has no kit support
 * (nothing to show then).
 */
export async function installKit(kit: string): Promise<KitInstall | null> {
  const client = await getIiiClient()
  let result: unknown
  try {
    result = await client.trigger<unknown>(
      DOWNLOAD_KIT,
      { kit, apply: true },
      { timeoutMs: KIT_INSTALL_TIMEOUT_MS },
    )
  } catch (error) {
    if (isMissingFunction(error)) return null
    return { kit, state: 'failed', message: readableError(error) }
  }
  const answer = (result ?? {}) as {
    status?: unknown
    message?: unknown
    plan?: { plan_id?: unknown }
  }
  const message =
    typeof answer.message === 'string' && answer.message.trim()
      ? answer.message.trim()
      : undefined
  switch (answer.status) {
    case 'applied':
    case 'up_to_date':
      return { kit, state: 'installed' }
    case 'planned':
    case 'replanned': {
      const planId = answer.plan?.plan_id
      return {
        kit,
        state: 'review',
        ...(message ? { message } : {}),
        ...(typeof planId === 'string' && planId ? { planId } : {}),
      }
    }
    default:
      return {
        kit,
        state: 'failed',
        message: message ?? 'The Directory gave no install status.',
      }
  }
}

/* ── one run per page load, followed by the wizard ───────────────────── */

let installs: readonly KitInstall[] = []
let started = false
let queue: Promise<void> = Promise.resolve()
const listeners = new Set<() => void>()

function publish(next: readonly KitInstall[]) {
  installs = next
  for (const listener of [...listeners]) listener()
}

function upsert(entry: KitInstall) {
  publish(
    installs.some((known) => known.kit === entry.kit)
      ? installs.map((known) => (known.kit === entry.kit ? entry : known))
      : [...installs, entry],
  )
}

function drop(kit: string) {
  publish(installs.filter((known) => known.kit !== kit))
}

async function settle(kit: string): Promise<void> {
  try {
    const outcome = await installKit(kit)
    if (outcome) upsert(outcome)
    else drop(kit)
  } catch (error) {
    upsert({ kit, state: 'failed', message: readableError(error) })
  }
}

/**
 * Read the project's kits and install them, once per page load: opening the
 * wizard again joins the run already under way. Never rejects.
 */
export function startOnboardingKits(): Promise<void> {
  if (started) return queue
  started = true
  queue = fetchOnboardingKits()
    .catch(() => [] as string[])
    .then(async (kits) => {
      for (const kit of kits) upsert({ kit, state: 'installing' })
      for (const kit of kits) await settle(kit)
    })
  return queue
}

/** Try one kit again (after a failure, or after its plan was reviewed). */
export function retryOnboardingKit(kit: string): Promise<void> {
  upsert({ kit, state: 'installing' })
  queue = queue.then(() => settle(kit))
  return queue
}

export function subscribeOnboardingKits(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function onboardingKitsSnapshot(): readonly KitInstall[] {
  return installs
}

/** Tests only: forget the run so the next start reads the kits again. */
export function resetOnboardingKitsForTest() {
  installs = []
  started = false
  queue = Promise.resolve()
  listeners.clear()
}
