import { describe, expect, it } from 'vitest'
import {
  changeCount,
  collisionHeadline,
  decisionsPayload,
  effectiveDecision,
  functionsByWorker,
  hasConflictMarkers,
  initialSteps,
  ownerLabel,
  permissionTally,
  primaryLabel,
  profileFields,
  replacedFiles,
  skillFolders,
  unresolvedFiles,
  updateGroups,
  workerLine,
} from './model'
import type { Plan, PlanFile, PlanWorker } from './types'

function file(over: Partial<PlanFile> & { path: string }): PlanFile {
  return {
    source: over.path.startsWith('skills/') ? `skills/${over.path.split('/').slice(3).join('/')}` : over.path,
    kind: over.path.startsWith('agents/') ? 'agent' : 'skill',
    id: over.path,
    change: 'added',
    local: 'absent',
    default: 'overwrite',
    options: ['overwrite'],
    ...over,
  }
}

function worker(over: Partial<PlanWorker> & { name: string }): PlanWorker {
  return { installed: null, action: 'none', registry_url: '', ...over }
}

function plan(over: Partial<Plan>): Plan {
  return {
    plan_id: 'kp_1',
    kind: 'install',
    kit: 'acme/team',
    from: null,
    to: '1.0.0',
    requested: 'latest',
    major: false,
    registry_url: '',
    workers: [],
    files: [],
    capabilities: {},
    functions: [],
    warnings: [],
    blocking: [],
    counts: {
      agents: 0,
      skills: 0,
      workers: 0,
      functions: 0,
      added: 0,
      modified: 0,
      removed: 0,
      unchanged: 0,
      collisions: 0,
      conflicts: 0,
      decisions_required: 0,
    },
    created_at: '',
    expires_at: '',
    review: '',
    ...over,
  }
}

const collided = file({
  path: 'agents/reviewer.md',
  local: 'occupied',
  collision: { owner: 'worker', worker: 'kanban', version: '0.1.18' },
  options: ['overwrite', 'keep'],
})
const local = file({
  path: 'agents/mine.md',
  local: 'occupied',
  collision: { owner: 'local' },
  options: ['overwrite', 'keep'],
})
const skill = file({ path: 'skills/acme/team/tickets/flow.md' })

describe('install review', () => {
  it('says what the primary button will do', () => {
    const p = plan({ files: [collided, local, skill] })
    expect(primaryLabel(p, {})).toBe('Install and replace 2 profiles')
    expect(primaryLabel(p, { 'agents/mine.md': { decision: 'keep' } })).toBe('Install and replace 1 profile')
    expect(
      primaryLabel(p, {
        'agents/mine.md': { decision: 'keep' },
        'agents/reviewer.md': { decision: 'keep' },
      }),
    ).toBe('Install')
    expect(replacedFiles(p, {}).map((f) => f.path)).toEqual(['agents/reviewer.md', 'agents/mine.md'])
  })

  it('words collisions by owner, the local one the strongest', () => {
    expect(ownerLabel(collided.collision!)).toBe('came from worker kanban 0.1.18')
    expect(ownerLabel(local.collision!)).toMatch(/created in this project/)
    expect(ownerLabel({ owner: 'builtin' })).toBe('built into iii-directory')
    expect(collisionHeadline(plan({ files: [collided, local, skill] }))).toBe('2 existing profiles will be replaced')
  })

  it('sends only the decisions that differ from the plan', () => {
    const p = plan({ files: [collided, local] })
    expect(decisionsPayload(p, { 'agents/reviewer.md': { decision: 'overwrite' } })).toEqual({})
    expect(decisionsPayload(p, { 'agents/mine.md': { decision: 'keep' } })).toEqual({ 'agents/mine.md': 'keep' })
  })

  it('groups skills by their first folder and describes workers', () => {
    const groups = skillFolders([
      file({ path: 'skills/acme/team/tickets/a.md', source: 'skills/tickets/a.md' }),
      file({ path: 'skills/acme/team/tickets/b.md', source: 'skills/tickets/b.md' }),
      file({ path: 'skills/acme/team/SKILL.md', source: 'skills/SKILL.md' }),
    ])
    expect(groups.map((g) => [g.folder, g.files.length])).toEqual([
      ['', 1],
      ['tickets', 2],
    ])
    expect(workerLine(worker({ name: 'web', action: 'add', range: '^1.2', to: '1.2.20' }))).toBe(
      'New worker web ^1.2 → 1.2.20',
    )
    expect(
      workerLine(worker({ name: 'kanban', action: 'update', range: '^1.6', installed: '1.5.2', to: '1.6.1' })),
    ).toBe('kanban 1.5.2 → 1.6.1 (the kit asks ^1.6)')
  })

  it('groups preloaded functions by worker, unknown last', () => {
    const groups = functionsByWorker([
      { id: 'kanban::b', worker: 'kanban', status: 'ok' },
      { id: 'harness::ask', status: 'unknown' },
      { id: 'kanban::a', worker: 'kanban', status: 'ok' },
    ])
    expect(groups.map((g) => [g.worker, g.functions.map((f) => f.id)])).toEqual([
      ['kanban', ['kanban::a', 'kanban::b']],
      ['', ['harness::ask']],
    ])
  })

  it('starts the step list with the workers it will touch', () => {
    const p = plan({ files: [skill], workers: [worker({ name: 'web', action: 'add', range: '^1' })] })
    expect(initialSteps(p).map((s) => s.label)).toEqual(['Workers (1)', 'Files (1)', 'kits.lock'])
  })
})

describe('update review', () => {
  const conflict = file({
    path: 'skills/acme/team/tickets/index.md',
    change: 'modified',
    local: 'edited',
    merge: 'conflicts',
    default: null,
    options: ['kit', 'mine', 'merged'],
  })
  const changedAgent = file({
    path: 'agents/planner.md',
    change: 'modified',
    local: 'intact',
    default: 'kit',
    options: ['kit'],
  })
  const removed = file({
    path: 'skills/acme/team/legacy/old.md',
    change: 'removed',
    local: 'intact',
    default: 'remove',
    options: ['remove', 'keep'],
  })
  const p = plan({
    kind: 'update',
    from: '1.0.0',
    to: '1.1.0',
    files: [changedAgent, file({ path: 'agents/researcher.md' }), conflict, removed],
    workers: [
      worker({ name: 'web', action: 'add', range: '^1.2', to: '1.2.20' }),
      worker({ name: 'kanban', action: 'none', range: '^0.1.18', range_from: '^0.1.0', installed: '0.1.18' }),
    ],
    capabilities: { workers_added: ['web'], functions_added: [{ agent: 'planner', function: 'web::fetch' }] },
  })

  it('orders the change list like the review walks it', () => {
    expect(updateGroups(p).map((g) => g.id)).toEqual([
      'agents-new',
      'agents-changed',
      'workers-new',
      'workers-changed',
      'skills-changed',
      'skills-removed',
      'permissions',
    ])
    expect(changeCount(p)).toBe(6)
    expect(permissionTally(p)).toBe('+1 worker · +1 function')
  })

  it('keeps Update disabled until a conflict is resolved without markers', () => {
    expect(effectiveDecision(conflict, {})).toBeNull()
    expect(unresolvedFiles(p, {}).map((f) => f.path)).toEqual([conflict.path])
    expect(primaryLabel(p, {})).toBe('Update · 1 conflict pending')
    const markers = '<<<<<<< yours\na\n=======\nb\n>>>>>>> kit\n'
    expect(hasConflictMarkers(markers)).toBe(true)
    expect(unresolvedFiles(p, { [conflict.path]: { decision: 'merged', content: markers } })).toHaveLength(1)
    const resolved = { [conflict.path]: { decision: 'merged' as const, content: 'a and b\n' } }
    expect(unresolvedFiles(p, resolved)).toHaveLength(0)
    expect(primaryLabel(p, resolved)).toBe('Update to 1.1.0')
    expect(decisionsPayload(p, resolved)).toEqual({ [conflict.path]: { choice: 'merged', content: 'a and b\n' } })
    expect(unresolvedFiles(p, { [conflict.path]: { decision: 'mine' } })).toHaveLength(0)
  })

  it('labels removals with the worker count', () => {
    const r = plan({ kind: 'remove', from: '1.0.0', to: null, files: [removed] })
    expect(primaryLabel(r, {}, ['kanban'])).toBe('Remove acme/team · 1 file · 1 worker')
    expect(primaryLabel(r, { [removed.path]: { decision: 'keep' } })).toBe('Remove acme/team')
  })
})

describe('profile fields', () => {
  it('reads frontmatter as display fields and keeps the body', () => {
    const fm = profileFields(
      '---\nname: Planner\nlogo: "📋"\nmodel: opus\nextends: iii-minimal\nskills: [acme/team/flow, kanban/tickets]\nfunctions:\n  - kanban::ticket::create\n---\nPlan things.\n',
    )
    expect(fm.name).toBe('Planner')
    expect(fm.logo).toBe('📋')
    expect(fm.model).toBe('opus')
    expect(fm.extends).toBe('iii-minimal')
    expect(fm.skills).toEqual(['acme/team/flow', 'kanban/tickets'])
    expect(fm.functions).toEqual(['kanban::ticket::create'])
    expect(fm.body.trim()).toBe('Plan things.')
  })
})
