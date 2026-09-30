import { describe, expect, it } from 'vitest'
import { parseHead } from '../worktree-head'

const ok = (stdout: string) => ({ exit_code: 0, stdout })
// What the path probe prints in a main checkout and in a linked worktree.
const MAIN = ok('/r\n/r/.git\n/r/.git\n')
const LINKED = ok('/r.feat-x\n/r/.git/worktrees/r.feat-x\n/r/.git\n')
const CLEAN = ok('')

describe('parseHead', () => {
  it('names the branch from symbolic-ref, a tag of the same name notwithstanding', () => {
    expect(parseHead(ok('refs/heads/main\n'), MAIN, CLEAN)).toEqual({
      branch: 'main',
      worktree: '/r',
      linked: false,
      dirty: false,
    })
    expect(parseHead(ok('refs/heads/feat/x\n'), LINKED, ok('?? scratch.txt\n'))).toEqual({
      branch: 'feat/x',
      worktree: '/r.feat-x',
      linked: true,
      dirty: true,
    })
  })

  it('reads a detached HEAD, and old git echoing --path-format', () => {
    expect(parseHead({ exit_code: 1, stdout: '' }, MAIN, CLEAN)?.branch).toBeNull()
    // git < 2.31 in a main checkout.
    expect(parseHead(ok('refs/heads/main\n'), ok('/r\n/r/.git\n--path-format=absolute\n.git\n'), CLEAN)?.linked).toBe(
      false,
    )
  })

  it('shows nothing outside a work tree or when a probe dies', () => {
    const fatal = { exit_code: 128, stdout: '' }
    expect(parseHead(fatal, fatal, fatal)).toBeNull()
    // Inside `.git` or a bare repository: a branch, but no work tree.
    expect(parseHead(ok('refs/heads/main\n'), fatal, CLEAN)).toBeNull()
    expect(parseHead(null, MAIN, CLEAN)).toBeNull()
    // A status that fails only leaves the dirty mark off.
    expect(parseHead(ok('refs/heads/main\n'), MAIN, null)?.dirty).toBe(false)
  })
})
