import { describe, expect, it } from 'vitest'
import {
  WORKTREE_CLAIM_FUNCTION_ID,
  WORKTREE_LIST_FUNCTION_ID,
} from '@/lib/worktrees'
import {
  WORKSPACE_LIST_FUNCTION_ID,
  WORKSPACE_ROOTS_FUNCTION_ID,
  WORKSPACE_VALIDATE_FUNCTION_ID,
} from './DirectoryPicker'

describe('DirectoryPicker workspace API wiring', () => {
  it('uses shell workspace control-plane functions', () => {
    expect(WORKSPACE_ROOTS_FUNCTION_ID).toBe('shell::workspace::roots')
    expect(WORKSPACE_LIST_FUNCTION_ID).toBe('shell::workspace::list')
    expect(WORKSPACE_VALIDATE_FUNCTION_ID).toBe('shell::workspace::validate')
  })

  it('the worktrees tab drives the worktree worker surface', () => {
    // The tab lists via worktree::list {include_status: true}; picking a row
    // claims via worktree::claim {worktree_id, session_id} (fired by the
    // ChatView pick handler alongside the workingDir change).
    expect(WORKTREE_LIST_FUNCTION_ID).toBe('worktree::list')
    expect(WORKTREE_CLAIM_FUNCTION_ID).toBe('worktree::claim')
  })
})
