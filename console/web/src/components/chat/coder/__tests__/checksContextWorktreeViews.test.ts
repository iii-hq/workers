import { describe, expect, it } from 'vitest'
import { checkStatus } from '../ChecksView'
import { dirtyLabel } from '../ContextView'
import { removeDisposition } from '../WorktreeView'

describe('checkStatus', () => {
  it('is green for exit 0', () => {
    expect(
      checkStatus({
        command: 'cargo check',
        exit_code: 0,
        output: '',
        truncated: false,
      }),
    ).toEqual({ label: 'exit 0', tone: 'accent' })
  })

  it('is red for a non-zero exit', () => {
    expect(
      checkStatus({
        command: 'cargo clippy',
        exit_code: 101,
        output: 'error: unused variable\n',
        truncated: false,
      }),
    ).toEqual({ label: 'exit 101', tone: 'alert' })
  })

  it('is amber when error is set — even alongside an exit code', () => {
    expect(
      checkStatus({
        command: 'cargo test',
        exit_code: 0,
        output: '',
        truncated: true,
        error: 'timed out after 120s',
      }),
    ).toEqual({ label: 'timed out after 120s', tone: 'warn' })
  })

  it('is a neutral dash when neither exit code nor error arrived', () => {
    expect(
      checkStatus({ command: 'true', output: '', truncated: false }),
    ).toEqual({ label: '—', tone: 'default' })
  })
})

describe('dirtyLabel', () => {
  it('reads clean with no status lines', () => {
    expect(dirtyLabel([], false)).toBe('clean')
  })

  it('counts status lines as dirty entries', () => {
    expect(dirtyLabel([' M a.rs', '?? b/'], false)).toBe('2 dirty')
  })

  it('marks a truncated status as a floor, not an exact count', () => {
    expect(dirtyLabel([' M a.rs'], true)).toBe('1+ dirty')
    // Truncated with zero lines still cannot claim clean.
    expect(dirtyLabel([], true)).toBe('0+ dirty')
  })
})

describe('removeDisposition', () => {
  const base = {
    path: '/work/.worktrees/fix',
    branch: 'worktree-fix',
  }

  it('reports removed with the branch deleted', () => {
    expect(
      removeDisposition({
        ...base,
        removed: true,
        dirty: false,
        branch_deleted: true,
      }),
    ).toEqual({ label: 'removed · branch deleted', tone: 'accent' })
  })

  it('reports removed with the branch kept', () => {
    expect(
      removeDisposition({
        ...base,
        removed: true,
        dirty: false,
        branch_deleted: false,
      }),
    ).toEqual({ label: 'removed · branch kept', tone: 'accent' })
  })

  it('never reads a dirty refusal as a removal', () => {
    expect(
      removeDisposition({
        ...base,
        removed: false,
        dirty: true,
        branch_deleted: false,
      }),
    ).toEqual({ label: 'kept — dirty', tone: 'warn' })
  })

  it('falls back to a neutral not-removed for clean refusals', () => {
    expect(
      removeDisposition({
        ...base,
        removed: false,
        dirty: false,
        branch_deleted: false,
      }),
    ).toEqual({ label: 'not removed', tone: 'default' })
  })
})
