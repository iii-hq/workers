import type { WorktreeInfo, WorktreeStatusInfo } from '@/lib/worktrees'
import type { FunctionCallMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()
const WT = 'wt_9f8e7d6c'
const WT_PATH = `/home/dev/.iii/worktrees/app-1f2e3d/${WT}`

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionCallMessage>,
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId,
    input,
    output,
    durationMs: 240,
    createdAt: now,
    ...extra,
  }
}

export const worktreeCreateDone = base(
  'wt-create',
  'worktree::create',
  { repo_path: '/home/dev/app', session_id: 'console-1a2b3c' },
  wrapHarness({
    worktree_id: WT,
    path: WT_PATH,
    branch: `iii/${WT}`,
    base_ref: 'HEAD',
    base_sha: '8fbe7a1c9d0e2f3a4b5c6d7e8f9a0b1c2d3e4f5a',
    repo_key: '/home/dev/app/.git',
    created_at: now - 60_000,
  }),
)

export const worktreeListDone = base(
  'wt-list',
  'worktree::list',
  { include_status: true },
  wrapHarness({
    worktrees: [
      {
        worktree_id: WT,
        repo_path: '/home/dev/app',
        path: WT_PATH,
        branch: `iii/${WT}`,
        lifecycle: 'claimed',
        session_id: 'console-1a2b3c',
        status: {
          clean: false,
          ahead: 2,
          behind: 0,
          staged: 0,
          unstaged: 1,
          untracked: 0,
          conflicted: 0,
          unpushed: 2,
          in_rebase: false,
        },
      },
      {
        worktree_id: 'wt_11aa22bb',
        repo_path: '/home/dev/app',
        path: '/home/dev/.iii/worktrees/app-1f2e3d/wt_11aa22bb',
        branch: 'iii/wt_11aa22bb',
        lifecycle: 'active',
        session_id: null,
        status: {
          clean: true,
          ahead: 0,
          behind: 0,
          staged: 0,
          unstaged: 0,
          untracked: 0,
          conflicted: 0,
          unpushed: 0,
          in_rebase: false,
        },
      },
    ],
    unmanaged: [],
  }),
)

export const worktreeStatusDone = base(
  'wt-status',
  'worktree::status',
  { worktree_id: WT },
  {
    worktree_id: WT,
    branch: `iii/${WT}`,
    lifecycle: 'claimed',
    clean: true,
    ahead: 2,
    behind: 0,
    staged: 0,
    unstaged: 0,
    untracked: 0,
    conflicted: 0,
    diffstat: '3 files changed, 42 insertions(+), 7 deletions(-)',
    unpushed: 2,
    in_rebase: false,
    head_sha: 'a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e',
  },
)

export const worktreeLandQueued = base(
  'wt-land',
  'worktree::land',
  {
    worktree_id: WT,
    target_branch: 'main',
    test_cmd: 'cargo test',
  },
  wrapHarness({ job_id: 'job_4d5e6f70', queued: true }),
)

export const worktreeLandConflicted = base(
  'wt-land-conflicted',
  'worktree::land',
  { worktree_id: WT, target_branch: 'main' },
  wrapHarness({
    code: 'W402',
    message:
      'worktree wt_9f8e7d6c has a rebase in progress from a previous land; ' +
      'resolve it in place (then rerun worktree::land) or pass force_restart ' +
      'to abort and start over',
  }),
)

export const worktreeRemovePending = base(
  'wt-remove-pending',
  'worktree::remove',
  { worktree_id: WT, force: true, delete_branch: true },
  undefined,
  { pendingApproval: true },
)

export const worktreeFixtures = [
  worktreeCreateDone,
  worktreeListDone,
  worktreeStatusDone,
  worktreeLandQueued,
  worktreeLandConflicted,
  worktreeRemovePending,
] as const

// ---------------------------------------------------------------------------
// Registry fixtures for the worktrees graph page: two repos, every lifecycle
// (active / claimed / landing / land-blocked / orphaned) so each tint renders.
// ---------------------------------------------------------------------------

const cleanStatus: WorktreeStatusInfo = {
  clean: true,
  ahead: 0,
  behind: 0,
  staged: 0,
  unstaged: 0,
  untracked: 0,
  conflicted: 0,
  unpushed: 0,
  in_rebase: false,
}

function graphWt(
  id: string,
  repo: string,
  lifecycle: WorktreeInfo['lifecycle'],
  createdAt: number,
  extra?: Partial<WorktreeInfo>,
): WorktreeInfo {
  const repoName = repo.split('/').filter(Boolean).pop() ?? 'repo'
  return {
    worktree_id: id,
    repo_path: repo,
    repo_key: `${repo}/.git`,
    path: `/home/dev/.iii/worktrees/${repoName}-1f2e3d/${id}`,
    branch: `iii/${id}`,
    base_ref: 'HEAD',
    base_sha: '8fbe7a1c9d0e2f3a4b5c6d7e8f9a0b1c2d3e4f5a',
    lifecycle,
    session_id: null,
    created_at: createdAt,
    updated_at: createdAt + 30_000,
    status: cleanStatus,
    ...extra,
  }
}

export const worktreeGraphFixtures: WorktreeInfo[] = [
  graphWt('wt_11aa22bb', '/home/dev/app', 'active', now - 500_000),
  graphWt('wt_9f8e7d6c', '/home/dev/app', 'claimed', now - 400_000, {
    session_id: 'console-1a2b3c4d',
    status: {
      ...cleanStatus,
      clean: false,
      unstaged: 2,
      ahead: 2,
      unpushed: 2,
      diffstat: '3 files changed, 42 insertions(+), 7 deletions(-)',
    },
  }),
  graphWt('wt_33cc44dd', '/home/dev/app', 'landing', now - 300_000, {
    session_id: 'console-5e6f7a8b',
    status: { ...cleanStatus, ahead: 1, unpushed: 1 },
  }),
  graphWt('wt_55ee66ff', '/home/dev/api', 'land-blocked', now - 200_000, {
    base_ref: 'release/1.2',
    session_id: 'console-9c0d1e2f',
    status: { ...cleanStatus, conflicted: 2, in_rebase: true, ahead: 3 },
  }),
  graphWt('wt_7700aa11', '/home/dev/api', 'orphaned', now - 100_000, {
    status: null,
  }),
]
