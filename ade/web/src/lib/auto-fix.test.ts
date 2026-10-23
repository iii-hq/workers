import { describe, expect, it } from 'vitest'
import {
  AUTO_FIX_REPORT_CAP,
  type AutoFixSeen,
  autoFixNews,
  autoFixRound,
  autoFixState,
  readPrActivity,
} from './auto-fix'
import type { GhRunner } from './chat-aside'

const pr = { repo: 'o/r', number: 9, url: 'https://github.com/o/r/pull/9' }

/** A `gh` that answers `pr view` and `pr checks` for PR 9. */
function gh(
  view: Record<string, unknown> | null,
  checks: { name: string; bucket: string }[] = [],
): GhRunner {
  return async (args) => {
    if (args[1] === 'view') {
      return view
        ? { exitCode: 0, stdout: JSON.stringify(view), stderr: '' }
        : { exitCode: 1, stdout: '', stderr: 'gh: not logged in' }
    }
    return { exitCode: 1, stdout: JSON.stringify(checks), stderr: '' }
  }
}

const seen = (extra: Partial<AutoFixSeen> = {}): AutoFixSeen => ({
  url: pr.url,
  failing: [],
  reviews: 0,
  conflict: false,
  reports: 0,
  ...extra,
})

describe('readPrActivity', () => {
  it('reads failing checks, others’ reviews asking for something, and conflicts', async () => {
    const activity = await readPrActivity(
      gh(
        {
          state: 'OPEN',
          mergeable: 'CONFLICTING',
          reviews: [
            { author: { login: 'me' }, state: 'COMMENTED' },
            { author: { login: 'bot' }, state: 'COMMENTED' },
            { author: { login: 'lead' }, state: 'CHANGES_REQUESTED' },
            { author: { login: 'lead' }, state: 'APPROVED' },
          ],
        },
        [
          { name: 'test', bucket: 'fail' },
          { name: 'lint', bucket: 'cancel' },
          { name: 'build', bucket: 'pending' },
          { name: 'test', bucket: 'fail' },
        ],
      ),
      pr,
      'me',
    )
    expect(activity).toEqual({
      open: true,
      failing: ['lint', 'test'],
      reviews: 2,
      conflict: true,
    })
  })

  it('reads nothing when gh cannot answer', async () => {
    expect(await readPrActivity(gh(null), pr, 'me')).toBeNull()
  })
})

describe('autoFixNews', () => {
  const now = { open: true, failing: ['test'], reviews: 0, conflict: false }

  it('reports only what changed since the last report', () => {
    expect(autoFixNews(seen(), now)).toEqual(['a check fails: test'])
    expect(autoFixNews(seen({ failing: ['test'] }), now)).toEqual([])
    expect(
      autoFixNews(seen({ failing: ['test'], reviews: 1 }), {
        ...now,
        failing: ['test', 'lint'],
        reviews: 3,
        conflict: true,
      }),
    ).toEqual([
      'a check fails: lint',
      '2 new reviews',
      'it conflicts with its base',
    ])
  })
})

describe('autoFixRound', () => {
  it('builds one message, keeps the new state and drops a closed PR', async () => {
    const run: GhRunner = async (args) => {
      const number = args[2]
      if (args[1] === 'view') {
        const state = number === '9' ? 'OPEN' : 'MERGED'
        return {
          exitCode: 0,
          stdout: JSON.stringify({
            state,
            mergeable: 'MERGEABLE',
            reviews: [],
          }),
          stderr: '',
        }
      }
      return {
        exitCode: 1,
        stdout: JSON.stringify([{ name: 'test', bucket: 'fail' }]),
        stderr: '',
      }
    }
    const { next, message } = await autoFixRound(
      run,
      { 'o/r#9': seen(), 'o/r#3': seen({ url: 'u3' }) },
      'me',
    )
    expect(next).toEqual({
      'o/r#9': seen({ failing: ['test'], reports: 1 }),
    })
    expect(message).toContain('Auto-fix for https://github.com/o/r/pull/9')
    expect(message).toContain('a check fails: test')
  })

  it('keeps the last state when gh cannot read, and stops at the cap', async () => {
    const quiet = await autoFixRound(gh(null), { 'o/r#9': seen() }, 'me')
    expect(quiet).toEqual({ next: { 'o/r#9': seen() }, message: null })
    const capped = await autoFixRound(
      gh({ state: 'OPEN', mergeable: 'MERGEABLE', reviews: [] }, [
        { name: 'test', bucket: 'fail' },
      ]),
      { 'o/r#9': seen({ reports: AUTO_FIX_REPORT_CAP - 1 }) },
      'me',
    )
    expect(capped.message).not.toBeNull()
    expect(capped.next).toEqual({})
  })
})

describe('autoFixState', () => {
  it('reads the watched PRs off the session metadata', () => {
    expect(
      autoFixState({
        sessionMetadata: {
          context_autofix: {
            'o/r#9': { url: pr.url, failing: ['a', 2], reviews: 1 },
            'o/r#1': { failing: [] },
          },
        },
      }),
    ).toEqual({ 'o/r#9': seen({ failing: ['a'], reviews: 1 }) })
    expect(autoFixState(null)).toEqual({})
  })
})
