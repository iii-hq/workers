import { describe, expect, it } from 'vitest'
import {
  formatLandBlockedNotice,
  formatLandedNotice,
  lifecycleTone,
  parseLandBlockedEvent,
  parseLandedEvent,
  parseWorktreeInfo,
  shortWorktreeId,
  WORKTREE_CLAIM_FUNCTION_ID,
  WORKTREE_GET_FUNCTION_ID,
  WORKTREE_LAND_BLOCKED_TRIGGER,
  WORKTREE_LANDED_TRIGGER,
  WORKTREE_LIFECYCLE_TRIGGERS,
  WORKTREE_LIFECYCLES,
  WORKTREE_LIST_FUNCTION_ID,
  WORKTREE_RELEASE_FUNCTION_ID,
  WORKTREE_VALIDATE_FUNCTION_ID,
  worktreeIndicators,
} from './worktrees'

describe('worktree control-plane wiring', () => {
  it('uses the worktree worker function ids', () => {
    expect(WORKTREE_LIST_FUNCTION_ID).toBe('worktree::list')
    expect(WORKTREE_GET_FUNCTION_ID).toBe('worktree::get')
    expect(WORKTREE_VALIDATE_FUNCTION_ID).toBe('worktree::validate')
    expect(WORKTREE_CLAIM_FUNCTION_ID).toBe('worktree::claim')
    expect(WORKTREE_RELEASE_FUNCTION_ID).toBe('worktree::release')
  })

  it('binds the worktree trigger types', () => {
    expect(WORKTREE_LANDED_TRIGGER).toBe('worktree::landed')
    expect(WORKTREE_LAND_BLOCKED_TRIGGER).toBe('worktree::land-blocked')
  })

  it('the lifecycle feed covers all six trigger types the worker emits', () => {
    // The graph page refreshes on ANY of these; a missing one means a stale
    // graph for that mutation.
    expect([...WORKTREE_LIFECYCLE_TRIGGERS]).toEqual([
      'worktree::created',
      'worktree::claimed',
      'worktree::released',
      'worktree::removed',
      'worktree::landed',
      'worktree::land-blocked',
    ])
  })

  it('every lifecycle variant maps to a tone', () => {
    for (const lifecycle of WORKTREE_LIFECYCLES) {
      expect(['ink', 'accent', 'warn', 'alert']).toContain(
        lifecycleTone(lifecycle),
      )
    }
    // The non-neutral states must be visually distinct from each other.
    expect(
      new Set([
        lifecycleTone('landing'),
        lifecycleTone('land-blocked'),
        lifecycleTone('orphaned'),
      ]).size,
    ).toBe(3)
  })
})

const status = {
  clean: false,
  ahead: 2,
  behind: 0,
  staged: 1,
  unstaged: 0,
  untracked: 0,
  conflicted: 0,
  unpushed: 2,
  in_rebase: false,
}

describe('parseWorktreeInfo', () => {
  it('accepts a full record and tolerates extra fields', () => {
    const info = parseWorktreeInfo({
      worktree_id: 'wt_1f2e3d4c',
      repo_path: '/home/dev/app',
      repo_key: '/home/dev/app/.git',
      path: '/home/dev/.iii/worktrees/app/wt_1f2e3d4c',
      branch: 'iii/wt_1f2e3d4c',
      base_ref: 'HEAD',
      lifecycle: 'claimed',
      session_id: 'console-1',
      status,
      created_at: 1,
      updated_at: 2,
    })
    expect(info?.branch).toBe('iii/wt_1f2e3d4c')
    expect(info?.lifecycle).toBe('claimed')
    expect(info?.status?.ahead).toBe(2)
  })

  it('rejects unknown lifecycles and missing fields', () => {
    expect(
      parseWorktreeInfo({
        worktree_id: 'wt_1',
        repo_path: '/r',
        path: '/p',
        branch: 'b',
        lifecycle: 'bogus',
      }),
    ).toBeNull()
    expect(parseWorktreeInfo({ worktree_id: 'wt_1' })).toBeNull()
    expect(parseWorktreeInfo(null)).toBeNull()
  })
})

describe('badge helpers', () => {
  it('shortWorktreeId strips the wt_ prefix only', () => {
    expect(shortWorktreeId('wt_1f2e3d4c')).toBe('1f2e3d4c')
    expect(shortWorktreeId('custom')).toBe('custom')
  })

  it('lifecycleTone maps per the status palette', () => {
    expect(lifecycleTone('active')).toBe('ink')
    expect(lifecycleTone('claimed')).toBe('ink')
    expect(lifecycleTone('landing')).toBe('accent')
    expect(lifecycleTone('land-blocked')).toBe('alert')
    expect(lifecycleTone('orphaned')).toBe('warn')
  })

  it('worktreeIndicators derives dirty and ahead', () => {
    expect(worktreeIndicators(status)).toEqual({ dirty: true, ahead: 2 })
    expect(worktreeIndicators({ ...status, clean: true, ahead: 0 })).toEqual({
      dirty: false,
      ahead: 0,
    })
    expect(worktreeIndicators(null)).toEqual({ dirty: false, ahead: 0 })
  })
})

describe('live event parsing', () => {
  it('parses landed events and formats the notice', () => {
    const evt = parseLandedEvent({
      worktree_id: 'wt_1f2e3d4c',
      repo_path: '/home/dev/app',
      branch: 'iii/wt_1f2e3d4c',
      target_branch: 'main',
      merged_sha: 'a1b2c3d4e5f60718',
      timestamp: 1,
    })
    expect(evt).not.toBeNull()
    expect(formatLandedNotice(evt as NonNullable<typeof evt>)).toBe(
      'worktree iii/wt_1f2e3d4c landed onto main (a1b2c3d4)',
    )
  })

  it('rejects malformed landed payloads', () => {
    expect(parseLandedEvent({ worktree_id: 'wt_1' })).toBeNull()
    expect(parseLandedEvent('nope')).toBeNull()
  })

  it('parses land-blocked events and formats per reason', () => {
    const conflict = parseLandBlockedEvent({
      worktree_id: 'wt_1f2e3d4c',
      repo_path: '/home/dev/app',
      target_branch: 'main',
      reason: 'rebase_conflict',
      code: 'W410',
      conflict_files: ['src/a.rs', 'src/b.rs'],
    })
    expect(conflict?.code).toBe('W410')
    const msg = formatLandBlockedNotice(
      conflict as NonNullable<typeof conflict>,
    )
    expect(msg).toContain('rebase conflicts in src/a.rs, src/b.rs')
    expect(msg).toContain('1f2e3d4c')

    const tests = parseLandBlockedEvent({
      worktree_id: 'wt_1f2e3d4c',
      repo_path: '/home/dev/app',
      target_branch: 'main',
      reason: 'test_failed',
      code: 'W411',
      exit_code: 1,
    })
    expect(
      formatLandBlockedNotice(tests as NonNullable<typeof tests>),
    ).toContain('tests failed (exit 1)')

    const moved = parseLandBlockedEvent({
      worktree_id: 'wt_1f2e3d4c',
      repo_path: '/home/dev/app',
      target_branch: 'main',
      reason: 'target_moved_exhausted',
      code: 'W412',
    })
    expect(
      formatLandBlockedNotice(moved as NonNullable<typeof moved>),
    ).toContain('kept moving')

    const unknown = parseLandBlockedEvent({
      worktree_id: 'wt_1f2e3d4c',
      repo_path: '/home/dev/app',
      target_branch: 'main',
      reason: 'something_new',
      code: 'W499',
    })
    expect(
      formatLandBlockedNotice(unknown as NonNullable<typeof unknown>),
    ).toContain('something_new (W499)')
  })
})
