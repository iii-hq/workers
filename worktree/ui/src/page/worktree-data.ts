/**
 * Read-only RPC surface for the injected worktrees page: typed wrappers over
 * the `worktree` worker's registry, plus the parsers and view helpers the
 * graph and detail panel use.
 *
 * Ported from the console's `lib/worktrees.ts`: the function ids, the six
 * lifecycle trigger types, the payload shapes, and the tolerant zod parsing
 * are verbatim; only the transport changed — `listWorktrees` now takes the
 * tab's `host` and routes through `host.iii.trigger(...)` instead of a
 * console-internal iii client. Only the page's read surface travels here;
 * the mutating wrappers (claim / release / validate) stay in the console's
 * chat features, which keep their own copy.
 */

import type { Host } from '@iii-dev/console-ui'
import { z } from 'zod'

export const WORKTREE_LIST_FUNCTION_ID = 'worktree::list'

export const WORKTREE_CREATED_TRIGGER = 'worktree::created'
export const WORKTREE_CLAIMED_TRIGGER = 'worktree::claimed'
export const WORKTREE_RELEASED_TRIGGER = 'worktree::released'
export const WORKTREE_REMOVED_TRIGGER = 'worktree::removed'
export const WORKTREE_LANDED_TRIGGER = 'worktree::landed'
export const WORKTREE_LAND_BLOCKED_TRIGGER = 'worktree::land-blocked'

/** Every lifecycle trigger type the worker emits, in emission order. */
export const WORKTREE_LIFECYCLE_TRIGGERS = [
  WORKTREE_CREATED_TRIGGER,
  WORKTREE_CLAIMED_TRIGGER,
  WORKTREE_RELEASED_TRIGGER,
  WORKTREE_REMOVED_TRIGGER,
  WORKTREE_LANDED_TRIGGER,
  WORKTREE_LAND_BLOCKED_TRIGGER,
] as const

export const WORKTREE_LIFECYCLES = [
  'active',
  'claimed',
  'landing',
  'land-blocked',
  'orphaned',
] as const
export type WorktreeLifecycle = (typeof WORKTREE_LIFECYCLES)[number]

const lifecycleSchema = z.enum(WORKTREE_LIFECYCLES)

const worktreeStatusSchema = z.object({
  clean: z.boolean(),
  ahead: z.number(),
  behind: z.number(),
  staged: z.number(),
  unstaged: z.number(),
  untracked: z.number(),
  conflicted: z.number(),
  unpushed: z.number(),
  in_rebase: z.boolean(),
  diffstat: z.string().optional(),
  head_sha: z.string().optional(),
  // v0.2 workers; optional so older workers keep parsing.
  integrated: z.boolean().optional(),
  integration_reason: z.string().nullable().optional(),
})
export type WorktreeStatusInfo = z.infer<typeof worktreeStatusSchema>

/**
 * Label for an integrated worktree: "merged upstream", with the worker's
 * detection reason appended when it reports one.
 */
export function integrationLabel(status: WorktreeStatusInfo): string {
  return status.integration_reason
    ? `merged upstream (${status.integration_reason})`
    : 'merged upstream'
}

const worktreeInfoSchema = z.object({
  worktree_id: z.string(),
  repo_path: z.string(),
  repo_key: z.string().optional(),
  path: z.string(),
  branch: z.string(),
  base_ref: z.string().optional(),
  base_sha: z.string().optional(),
  lifecycle: lifecycleSchema,
  session_id: z.string().nullable().optional(),
  // v0.2 workers; optional so older workers keep parsing.
  dev_port: z.number().optional(),
  created_at: z.number().optional(),
  updated_at: z.number().optional(),
  status: worktreeStatusSchema.nullable().optional(),
})
export type WorktreeInfo = z.infer<typeof worktreeInfoSchema>

const listResultSchema = z.object({
  worktrees: z.array(z.unknown()).optional(),
})

export function parseWorktreeInfo(payload: unknown): WorktreeInfo | null {
  const parsed = worktreeInfoSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

/** `wt_1f2e3d4c` -> `1f2e3d4c` (the id already reads as a short hash). */
export function shortWorktreeId(worktreeId: string): string {
  return worktreeId.startsWith('wt_') ? worktreeId.slice(3) : worktreeId
}

/**
 * Badge/status tone per the console's status semantics: accent = live/running,
 * warn = warning, alert = error; everything else stays ink.
 */
export function lifecycleTone(
  lifecycle: WorktreeLifecycle,
): 'ink' | 'accent' | 'warn' | 'alert' {
  switch (lifecycle) {
    case 'landing':
      return 'accent'
    case 'land-blocked':
      return 'alert'
    case 'orphaned':
      return 'warn'
    default:
      return 'ink'
  }
}

/**
 * Static tone -> scoped class map. Injected CSS has no Tailwind, so every
 * surface that tints by lifecycle tone shares this table of `wt-tone-*`
 * classes (defined in styles.css) instead of a `text-${tone}` utility.
 */
export const lifecycleToneClass: Record<
  ReturnType<typeof lifecycleTone>,
  string
> = {
  ink: 'wt-tone-ink',
  accent: 'wt-tone-accent',
  warn: 'wt-tone-warn',
  alert: 'wt-tone-alert',
}

export interface WorktreeIndicators {
  dirty: boolean
  ahead: number
}

export function worktreeIndicators(
  status: WorktreeStatusInfo | null | undefined,
): WorktreeIndicators {
  if (!status) return { dirty: false, ahead: 0 }
  return { dirty: !status.clean, ahead: status.ahead }
}

/** Join truthy class names — the injected UI's stand-in for the console `cn`. */
export function cn(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(' ')
}

/**
 * `worktree::list` with git status, parsed tolerantly: unknown fields are
 * ignored and rows that fail the schema are dropped rather than blanking the
 * whole graph.
 */
export async function listWorktrees(host: Host): Promise<WorktreeInfo[]> {
  const res = await host.iii.trigger<unknown>(WORKTREE_LIST_FUNCTION_ID, {
    include_status: true,
  })
  const parsed = listResultSchema.safeParse(res)
  if (!parsed.success) return []
  return (parsed.data.worktrees ?? [])
    .map(parseWorktreeInfo)
    .filter((w): w is WorktreeInfo => w !== null)
}
