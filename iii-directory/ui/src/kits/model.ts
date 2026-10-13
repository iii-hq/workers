/**
 * Pure review logic for kit plans: effective decisions, what still needs a
 * decision, the primary button's wording, the update review's change
 * groups, and the human phrasing of owners, workers and permissions. No
 * React here — `model.test.ts` covers it.
 */

import { frontmatterBody, readFrontmatterField, readFrontmatterStringList } from '../page/frontmatter'
import type { Collision, Decision, KitFunction, Plan, PlanFile, PlanWorker } from './types'

/** A caller's choice for one file: the decision, plus edited text for `merged`. */
export interface FileChoice {
  decision: Decision
  content?: string
}

export type Choices = Record<string, FileChoice>

/** The decision a file will be applied with (the user's, else the plan's default). */
export function effectiveDecision(file: PlanFile, choices: Choices): Decision | null {
  return choices[file.path]?.decision ?? file.default
}

/** Files that cannot be applied yet: no decision, or a merge result that
 * still carries conflict markers. */
export function unresolvedFiles(plan: Plan, choices: Choices): PlanFile[] {
  return plan.files.filter((file) => {
    const decision = effectiveDecision(file, choices)
    if (decision === null) return true
    if (decision === 'merged' && file.merge === 'conflicts') {
      const content = choices[file.path]?.content
      return content === undefined || hasConflictMarkers(content)
    }
    return false
  })
}

export function hasConflictMarkers(text: string): boolean {
  let open = false
  for (const line of text.split('\n')) {
    if (line.startsWith('<<<<<<<')) open = true
    else if (open && line.startsWith('>>>>>>>')) return true
  }
  return false
}

/** The `decisions` argument for `directory::kits::apply`: only what differs
 * from the plan, plus every merged text. */
export function decisionsPayload(plan: Plan, choices: Choices): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const file of plan.files) {
    const choice = choices[file.path]
    if (!choice) continue
    if (choice.decision === 'merged' && choice.content !== undefined) {
      out[file.path] = { choice: 'merged', content: choice.content }
    } else if (choice.decision !== file.default) {
      out[file.path] = choice.decision
    }
  }
  return out
}

/** Collisions the install will act on: existing files the kit overwrites. */
export function replacedFiles(plan: Plan, choices: Choices): PlanFile[] {
  return plan.files.filter((f) => f.collision && effectiveDecision(f, choices) === 'overwrite')
}

function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`
}

/** What the primary button says: the consequence, not just the verb. */
export function primaryLabel(plan: Plan, choices: Choices, removeWorkers: readonly string[] = []): string {
  const pending = unresolvedFiles(plan, choices).length
  if (plan.kind === 'install') {
    const replaced = replacedFiles(plan, choices)
    const profiles = replaced.filter((f) => f.kind === 'agent').length
    const files = replaced.length - profiles
    const parts: string[] = []
    if (profiles > 0) parts.push(plural(profiles, 'profile'))
    if (files > 0) parts.push(plural(files, 'file'))
    return parts.length > 0 ? `Install and replace ${parts.join(' and ')}` : 'Install'
  }
  if (plan.kind === 'update') {
    if (pending > 0) return `Update · ${plural(pending, 'conflict')} pending`
    return `Update to ${plan.to ?? 'the new version'}`
  }
  const removing = plan.files.filter((f) => effectiveDecision(f, choices) === 'remove').length
  const parts = [`Remove ${plan.kit}`]
  if (removing > 0) parts.push(plural(removing, 'file'))
  if (removeWorkers.length > 0) parts.push(plural(removeWorkers.length, 'worker'))
  return parts.length > 1 ? `${parts[0]} · ${parts.slice(1).join(' · ')}` : parts[0]
}

/** One line on who owns a file the kit wants to write. */
export function ownerLabel(collision: Collision): string {
  switch (collision.owner) {
    case 'worker':
      return `came from worker ${collision.worker ?? '?'}${collision.version ? ` ${collision.version}` : ''}${
        collision.modified ? ' (edited here)' : ''
      }`
    case 'kit':
      return `belongs to kit ${collision.kit ?? '?'}${collision.version ? ` ${collision.version}` : ''}`
    case 'local':
      return 'created in this project — not from any package'
    case 'global':
      return 'your user-global profile (~/.iii/agents)'
    case 'builtin':
      return 'built into iii-directory'
  }
}

/** How strongly a collision should read: a local file is the one at risk. */
export function collisionTone(collision: Collision): 'alert' | 'warn' {
  return collision.owner === 'local' ? 'alert' : 'warn'
}

/** The heading of the install warnings block. */
export function collisionHeadline(plan: Plan): string {
  const collided = plan.files.filter((f) => f.collision)
  const profiles = collided.filter((f) => f.kind === 'agent').length
  const files = collided.length - profiles
  const parts: string[] = []
  if (profiles > 0) parts.push(`${plural(profiles, 'existing profile')}`)
  if (files > 0) parts.push(`${plural(files, 'existing file')}`)
  return `${parts.join(' and ')} will be replaced`
}

/** Permission phrasing for one worker change. */
export function workerLine(w: PlanWorker): string {
  switch (w.action) {
    case 'add':
      return `New worker ${w.name} ${w.range ?? ''} → ${w.to ?? 'latest'}`.replace(/\s+/g, ' ')
    case 'update':
      return `${w.name} ${w.installed ?? '?'} → ${w.to ?? '?'} (the kit asks ${w.range})`
    case 'redeclare':
      return `${w.name} range ${w.range_from ?? w.declared ?? '?'} → ${w.range} (${w.installed ?? '?'} already satisfies it)`
    case 'remove':
      return `${w.name} is no longer part of the kit`
    case 'none':
      return w.path
        ? `${w.name} runs from a local path; the kit asks ${w.range}`
        : `${w.name} ${w.installed ?? ''} satisfies ${w.range ?? ''}`.trim()
  }
}

/** Worker status word for the contents column / sidebar. */
export function workerStatusWord(w: PlanWorker): string {
  switch (w.action) {
    case 'add':
      return 'new'
    case 'update':
      return 'updates'
    case 'redeclare':
      return 'range'
    case 'remove':
      return 'dropped'
    case 'none':
      return w.path ? 'local' : 'ok'
  }
}

/** Preloaded functions grouped by the worker that provides them. */
export function functionsByWorker(functions: readonly KitFunction[]): { worker: string; functions: KitFunction[] }[] {
  const groups = new Map<string, KitFunction[]>()
  for (const f of functions) {
    const key = f.worker ?? (f.status === 'unknown' ? '' : (f.id.split('::')[0] ?? ''))
    groups.set(key, [...(groups.get(key) ?? []), f])
  }
  return [...groups.entries()]
    .map(([worker, fns]) => ({ worker, functions: fns.sort((a, b) => a.id.localeCompare(b.id)) }))
    .sort((a, b) => (a.worker === '' ? 1 : b.worker === '' ? -1 : a.worker.localeCompare(b.worker)))
}

export function functionsSummary(functions: readonly KitFunction[]): string {
  const workers = new Set(functions.map((f) => f.worker).filter(Boolean))
  return `The profiles preload ${plural(functions.length, 'function')} from ${plural(workers.size, 'worker')}`
}

/** A section of the update review's change list, in the order the review
 * walks: profiles, workers, skills, permissions. */
export interface ChangeGroup {
  id: string
  label: string
  items: ChangeItem[]
}

export type ChangeItem =
  | { key: string; type: 'file'; file: PlanFile }
  | { key: string; type: 'worker'; worker: PlanWorker }
  | { key: 'permissions'; type: 'permissions' }

export function updateGroups(plan: Plan): ChangeGroup[] {
  const agents = plan.files.filter((f) => f.kind === 'agent')
  const skills = plan.files.filter((f) => f.kind === 'skill')
  const file = (f: PlanFile): ChangeItem => ({ key: f.path, type: 'file', file: f })
  const worker = (w: PlanWorker): ChangeItem => ({ key: `worker:${w.name}`, type: 'worker', worker: w })
  const groups: ChangeGroup[] = [
    { id: 'agents-new', label: 'New profiles', items: agents.filter((f) => f.change === 'added').map(file) },
    {
      id: 'agents-changed',
      label: 'Changed profiles',
      items: agents.filter((f) => f.change === 'modified' || f.change === 'kept').map(file),
    },
    { id: 'agents-removed', label: 'Removed profiles', items: agents.filter((f) => f.change === 'removed').map(file) },
    { id: 'workers-new', label: 'New workers', items: plan.workers.filter((w) => w.action === 'add').map(worker) },
    {
      id: 'workers-changed',
      label: 'Worker ranges',
      items: plan.workers
        .filter(
          (w) => w.action === 'update' || w.action === 'redeclare' || (w.range_from && w.range && w.action === 'none'),
        )
        .map(worker),
    },
    {
      id: 'workers-removed',
      label: 'Workers dropped',
      items: plan.workers.filter((w) => w.action === 'remove').map(worker),
    },
    { id: 'skills-new', label: 'New skills', items: skills.filter((f) => f.change === 'added').map(file) },
    {
      id: 'skills-changed',
      label: 'Changed skills',
      items: skills.filter((f) => f.change === 'modified' || f.change === 'kept').map(file),
    },
    { id: 'skills-removed', label: 'Removed skills', items: skills.filter((f) => f.change === 'removed').map(file) },
  ]
  const nonEmpty = groups.filter((g) => g.items.length > 0)
  if (hasPermissions(plan)) {
    nonEmpty.push({ id: 'permissions', label: 'Permissions', items: [{ key: 'permissions', type: 'permissions' }] })
  }
  return nonEmpty
}

export function hasPermissions(plan: Plan): boolean {
  const c = plan.capabilities
  return (
    (c.workers_added?.length ?? 0) > 0 ||
    (c.functions_added?.length ?? 0) > 0 ||
    (c.models_changed?.length ?? 0) > 0 ||
    plan.workers.some((w) => w.action === 'add' || w.action === 'update')
  )
}

/** Short permission tally for the update sidebar (`+1 worker · +3 functions`). */
export function permissionTally(plan: Plan): string {
  const parts: string[] = []
  const workers = plan.capabilities.workers_added?.length ?? 0
  const functions = plan.capabilities.functions_added?.length ?? 0
  const models = plan.capabilities.models_changed?.length ?? 0
  if (workers) parts.push(`+${plural(workers, 'worker')}`)
  if (functions) parts.push(`+${plural(functions, 'function')}`)
  if (models) parts.push(`~${plural(models, 'model')}`)
  return parts.join(' · ') || 'no new capabilities'
}

export function changeCount(plan: Plan): number {
  return updateGroups(plan)
    .filter((g) => g.id !== 'permissions')
    .reduce((n, g) => n + g.items.length, 0)
}

/** Skill rows grouped by their first directory, for the contents column. */
export function skillFolders(files: readonly PlanFile[]): { folder: string; files: PlanFile[] }[] {
  const groups = new Map<string, PlanFile[]>()
  for (const f of files) {
    if (f.kind !== 'skill') continue
    const rest = f.source.replace(/^skills\//, '')
    const folder = rest.includes('/') ? rest.slice(0, rest.indexOf('/')) : ''
    groups.set(folder, [...(groups.get(folder) ?? []), f])
  }
  return [...groups.entries()]
    .map(([folder, list]) => ({ folder, files: list }))
    .sort((a, b) => (a.folder === '' ? -1 : b.folder === '' ? 1 : a.folder.localeCompare(b.folder)))
}

/** The skill's path inside the kit, without `skills/` and `.md`. */
export function skillLabel(file: PlanFile): string {
  return file.source.replace(/^skills\//, '').replace(/\.md$/, '')
}

/** Frontmatter of a profile as display fields, plus its body. */
export interface ProfileFields {
  name: string
  description: string
  logo: string
  model: string
  reasoning_effort: string
  extends: string
  icon: string
  color: string
  skills: string[]
  functions: string[]
  body: string
}

export function profileFields(content: string): ProfileFields {
  const field = (key: string) => readFrontmatterField(content, [key]).value
  return {
    name: field('name'),
    description: field('description'),
    logo: field('logo'),
    model: field('model'),
    reasoning_effort: field('reasoning_effort'),
    extends: field('extends'),
    icon: field('icon'),
    color: field('color'),
    skills: readFrontmatterStringList(content, 'skills').values,
    functions: readFrontmatterStringList(content, 'functions').values,
    body: frontmatterBody(content),
  }
}

/** Is this skill id served by the kit itself (`<kit>/…`)? */
export function isKitSkill(kit: string, skillId: string): boolean {
  return skillId.startsWith(`${kit}/`)
}

export function decisionLabel(decision: Decision, file?: PlanFile): string {
  switch (decision) {
    case 'overwrite':
      return file?.change === 'kept' ? "Take the kit's version" : 'Replace'
    case 'keep':
      return file?.change === 'removed' ? 'Keep as a local file' : 'Keep the current one'
    case 'kit':
      return "Use the kit's version"
    case 'mine':
      return file?.local === 'missing' ? 'Keep it deleted' : 'Keep mine'
    case 'merged':
      return 'Use the merged result'
    case 'remove':
      return 'Remove'
  }
}

/** `minor · published 2 days ago`-style meta for the update header. */
export function bumpWord(plan: Plan): string | null {
  if (plan.major) return 'major'
  return plan.bump ?? null
}

/** Removal: workers another installed kit still declares. */
export function sharedWorkers(plan: Plan): string[] {
  return plan.workers.filter((w) => (w.used_by?.length ?? 0) > 0).map((w) => w.name)
}

/** The step list shown while an apply runs, before the first event lands. */
export function initialSteps(plan: Plan) {
  const moving = plan.workers.filter(
    (w) => w.action === 'add' || w.action === 'update' || w.action === 'redeclare',
  ).length
  return [
    { id: 'workers', label: moving ? `Workers (${moving})` : 'Workers', state: 'pending' as const },
    { id: 'files', label: `Files (${plan.files.length})`, state: 'pending' as const },
    { id: 'lock', label: 'kits.lock', state: 'pending' as const },
  ]
}
