import { describe, expect, it } from 'vitest'
import {
  isWorktreeAvailable,
  WORKTREE_WATCH_FN,
  WORKTREE_WORKER_NAME,
} from './use-worktree-status'

describe('worktree presence probe wiring', () => {
  it('probes the worktree worker with its own watch handler id', () => {
    expect(WORKTREE_WORKER_NAME).toBe('worktree')
    // Must be unique per presence probe so the browser-local `worker` trigger
    // handlers never collide (shell uses console::shell-watch).
    expect(WORKTREE_WATCH_FN).toBe('console::worktree-watch')
  })

  it('gates on both presence and the initial probe settling', () => {
    expect(
      isWorktreeAvailable({
        present: true,
        loading: false,
        state: 'connected',
        detail: null,
      }),
    ).toBe(true)
    expect(
      isWorktreeAvailable({
        present: true,
        loading: true,
        state: 'unknown',
        detail: null,
      }),
    ).toBe(false)
    expect(
      isWorktreeAvailable({
        present: false,
        loading: false,
        state: 'absent',
        detail: null,
      }),
    ).toBe(false)
  })
})
