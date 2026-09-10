import { z } from 'zod'
import { getIiiClient } from '@/lib/iii-client'

/**
 * Control plane for the optional `worktree` worker: typed wrappers over its
 * registry surface (list / claim / release / validate / get) plus parsers
 * and formatting helpers for the badge, the picker section, and the live
 * `worktree::landed` / `worktree::land-blocked` events. Everything here is
 * gated by the caller on worktree presence (`use-worktree-status`).
 */

export const WORKTREE_WORKER_NAME = 'worktree'

export const WORKTREE_LIST_FUNCTION_ID = 'worktree::list'
export const WORKTREE_GET_FUNCTION_ID = 'worktree::get'
export const WORKTREE_VALIDATE_FUNCTION_ID = 'worktree::validate'
export const WORKTREE_CLAIM_FUNCTION_ID = 'worktree::claim'
export const WORKTREE_RELEASE_FUNCTION_ID = 'worktree::release'

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

const validateResultSchema = z.object({
  valid: z.boolean(),
  managed: z.boolean(),
  worktree_id: z.string().nullable().optional(),
  reason: z.string().nullable().optional(),
})
export type WorktreeValidateResult = z.infer<typeof validateResultSchema>

const landedEventSchema = z.object({
  worktree_id: z.string(),
  repo_path: z.string(),
  branch: z.string(),
  target_branch: z.string(),
  merged_sha: z.string(),
  session_id: z.string().nullable().optional(),
})
export type WorktreeLandedEvent = z.infer<typeof landedEventSchema>

const landBlockedEventSchema = z.object({
  worktree_id: z.string(),
  repo_path: z.string(),
  target_branch: z.string(),
  reason: z.string(),
  code: z.string(),
  conflict_files: z.array(z.string()).nullable().optional(),
  exit_code: z.number().nullable().optional(),
  session_id: z.string().nullable().optional(),
})
export type WorktreeLandBlockedEvent = z.infer<typeof landBlockedEventSchema>

export function parseWorktreeInfo(payload: unknown): WorktreeInfo | null {
  const parsed = worktreeInfoSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

export function parseLandedEvent(payload: unknown): WorktreeLandedEvent | null {
  const parsed = landedEventSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

export function parseLandBlockedEvent(
  payload: unknown,
): WorktreeLandBlockedEvent | null {
  const parsed = landBlockedEventSchema.safeParse(payload)
  return parsed.success ? parsed.data : null
}

/** `wt_1f2e3d4c` -> `1f2e3d4c` (the id already reads as a short hash). */
export function shortWorktreeId(worktreeId: string): string {
  return worktreeId.startsWith('wt_') ? worktreeId.slice(3) : worktreeId
}

/**
 * Badge/status tone per DESIGN.md status semantics: accent = live/running,
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
 * Static tone -> text-color class map. Tailwind needs literal class names, so
 * every surface that tints by lifecycle tone shares this table instead of
 * rebuilding `text-${tone}` (which Tailwind cannot see) or a local copy.
 */
export const lifecycleToneClass: Record<
  ReturnType<typeof lifecycleTone>,
  string
> = {
  ink: 'text-ink-faint',
  accent: 'text-accent',
  warn: 'text-warn',
  alert: 'text-alert',
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

/** Transcript/announcer copy for a land-blocked event. */
export function formatLandBlockedNotice(evt: WorktreeLandBlockedEvent): string {
  const base = `land of worktree ${shortWorktreeId(evt.worktree_id)} onto ${evt.target_branch} blocked`
  switch (evt.reason) {
    case 'rebase_conflict': {
      const files = evt.conflict_files ?? []
      const preview = files.slice(0, 3).join(', ')
      const suffix = files.length > 3 ? ` and ${files.length - 3} more` : ''
      return files.length > 0
        ? `${base} — rebase conflicts in ${preview}${suffix}; resolve in the worktree, then land again`
        : `${base} — rebase conflicts; resolve in the worktree, then land again`
    }
    case 'test_failed':
      return `${base} — tests failed${
        evt.exit_code != null ? ` (exit ${evt.exit_code})` : ''
      }`
    case 'dirty':
      return `${base} — the worktree has uncommitted changes`
    case 'target_moved_exhausted':
      return `${base} — ${evt.target_branch} kept moving; retry the land`
    case 'target_checked_out_dirty':
      return `${base} — ${evt.target_branch} is checked out with local changes`
    default:
      return `${base} — ${evt.reason} (${evt.code})`
  }
}

export function formatLandedNotice(evt: WorktreeLandedEvent): string {
  return `worktree ${evt.branch} landed onto ${evt.target_branch} (${evt.merged_sha.slice(0, 8)})`
}

export async function listWorktrees(): Promise<WorktreeInfo[]> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(WORKTREE_LIST_FUNCTION_ID, {
    include_status: true,
  })
  const parsed = listResultSchema.safeParse(res)
  if (!parsed.success) return []
  return (parsed.data.worktrees ?? [])
    .map(parseWorktreeInfo)
    .filter((w): w is WorktreeInfo => w !== null)
}

export async function getWorktree(
  worktreeId: string,
): Promise<WorktreeInfo | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(WORKTREE_GET_FUNCTION_ID, {
    worktree_id: worktreeId,
  })
  return parseWorktreeInfo(res)
}

export async function validateWorktreePath(
  path: string,
): Promise<WorktreeValidateResult | null> {
  const client = await getIiiClient()
  const res = await client.trigger<unknown>(WORKTREE_VALIDATE_FUNCTION_ID, {
    path,
  })
  const parsed = validateResultSchema.safeParse(res)
  return parsed.success ? parsed.data : null
}

export async function claimWorktree(
  worktreeId: string,
  sessionId: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(WORKTREE_CLAIM_FUNCTION_ID, {
    worktree_id: worktreeId,
    session_id: sessionId,
  })
}

export async function releaseWorktree(
  worktreeId: string,
  sessionId: string,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(WORKTREE_RELEASE_FUNCTION_ID, {
    worktree_id: worktreeId,
    session_id: sessionId,
  })
}
