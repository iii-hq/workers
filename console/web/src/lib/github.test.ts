import { describe, expect, it } from 'vitest'
import {
  parseIssues,
  parsePrChecks,
  parsePrs,
  parseReleases,
  parseRuns,
  parseSearchItems,
  releaseUrl,
  timeAgoIso,
  unwrapValue,
} from './github'

/**
 * Parser contract against the github worker's wire shapes. The payloads
 * mirror the worker's `--json` field sets (github/tests/golden/schemas/*)
 * and its live output; a worker-side field-set change should surface here.
 */

const PR = {
  number: 529,
  title: '(MOT-4106) feat(github): GitHub CLI (gh) as an iii worker',
  state: 'OPEN',
  url: 'https://github.com/iii-hq/workers/pull/529',
  author: { id: 'U_x', is_bot: false, login: 'andersonleal', name: '' },
  headRefName: 'feat/gh-worker',
  baseRefName: 'main',
  isDraft: false,
  labels: [
    { color: 'D6393F', description: '', id: 'LA_x', name: 'needs-triage' },
  ],
  createdAt: '2026-07-18T08:18:49Z',
  updatedAt: '2026-07-18T09:00:00Z',
}

describe('unwrapValue', () => {
  it('unwraps the ValueResponse envelope and tolerates junk', () => {
    expect(unwrapValue({ value: [1, 2] })).toEqual([1, 2])
    expect(unwrapValue({ value: null })).toBeNull()
    expect(unwrapValue('nope')).toBeNull()
    expect(unwrapValue(undefined)).toBeNull()
  })
})

describe('parsePrs', () => {
  it('parses the pr list wire shape, extra fields ignored', () => {
    const prs = parsePrs([PR])
    expect(prs).toHaveLength(1)
    expect(prs[0]?.number).toBe(529)
    expect(prs[0]?.author?.login).toBe('andersonleal')
    expect(prs[0]?.labels?.[0]?.name).toBe('needs-triage')
  })

  it('drops invalid rows instead of failing the whole list', () => {
    expect(parsePrs([PR, { number: 'not-a-number' }, null])).toHaveLength(1)
  })

  it('returns empty for a non-array (null value, error payloads)', () => {
    expect(parsePrs(null)).toEqual([])
    expect(parsePrs({})).toEqual([])
  })
})

describe('parsePrChecks', () => {
  it('parses the checks rollup rows', () => {
    const checks = parsePrChecks([
      {
        bucket: 'skipping',
        completedAt: '2026-07-18T08:18:53Z',
        description: '',
        link: 'https://github.com/iii-hq/workers/actions/runs/1/job/2',
        name: 'close-no-help-wanted',
        startedAt: '2026-07-18T08:19:01Z',
        state: 'SKIPPED',
        workflow: 'PR Triaging',
      },
    ])
    expect(checks).toHaveLength(1)
    expect(checks[0]?.bucket).toBe('skipping')
    expect(checks[0]?.workflow).toBe('PR Triaging')
  })
})

describe('parseIssues / parseRuns / parseReleases', () => {
  it('parses an issue row', () => {
    const issues = parseIssues([
      {
        number: 7,
        title: 'bug',
        state: 'OPEN',
        url: 'https://github.com/o/r/issues/7',
        author: { login: 'octocat' },
        labels: [],
        assignees: [],
        milestone: null,
        createdAt: '2026-07-01T00:00:00Z',
        updatedAt: '2026-07-02T00:00:00Z',
      },
    ])
    expect(issues[0]?.number).toBe(7)
  })

  it('parses a run row (nullable conclusion while in progress)', () => {
    const runs = parseRuns([
      {
        databaseId: 29646046993,
        number: 12,
        displayTitle: '(MOT-4106) feat(github): ...',
        name: 'CI',
        workflowName: 'CI',
        headBranch: 'feat/gh-worker',
        headSha: 'abc123',
        event: 'pull_request',
        status: 'in_progress',
        conclusion: null,
        attempt: 1,
        createdAt: '2026-07-18T13:20:00Z',
        startedAt: '2026-07-18T13:20:05Z',
        updatedAt: '2026-07-18T13:21:00Z',
        url: 'https://github.com/iii-hq/workers/actions/runs/29646046993',
      },
    ])
    expect(runs[0]?.databaseId).toBe(29646046993)
    expect(runs[0]?.conclusion).toBeNull()
  })

  it('parses a release row and builds its url', () => {
    const releases = parseReleases([
      {
        tagName: 'fp/v0.2.0',
        name: 'fp 0.2.0',
        isDraft: false,
        isLatest: true,
        isPrerelease: false,
        createdAt: '2026-07-16T00:00:00Z',
        publishedAt: '2026-07-16T00:05:00Z',
      },
    ])
    expect(releases[0]?.tagName).toBe('fp/v0.2.0')
    expect(releaseUrl('iii-hq/workers', 'fp/v0.2.0')).toBe(
      'https://github.com/iii-hq/workers/releases/tag/fp%2Fv0.2.0',
    )
  })
})

describe('parseSearchItems', () => {
  it('parses each kind with its own row shape', () => {
    const repos = parseSearchItems('repos', [
      {
        fullName: 'cli/cli',
        url: 'https://github.com/cli/cli',
        language: 'Go',
      },
    ])
    expect(repos.kind).toBe('repos')
    expect(repos.items).toHaveLength(1)

    const prs = parseSearchItems('prs', [
      {
        number: 1,
        title: 't',
        state: 'OPEN',
        url: 'u',
        repository: { name: 'workers', nameWithOwner: 'iii-hq/workers' },
        isDraft: true,
      },
    ])
    expect(prs.kind).toBe('prs')
    expect(prs.items).toHaveLength(1)

    const code = parseSearchItems('code', [
      { path: 'src/gh.rs', repository: { nameWithOwner: 'iii-hq/workers' } },
    ])
    expect(code.kind).toBe('code')
    // discriminant narrowing: `items` is only GithubSearchCode[] inside the guard
    if (code.kind !== 'code') throw new Error('unreachable')
    expect(code.items[0]?.path).toBe('src/gh.rs')
  })
})

describe('timeAgoIso', () => {
  const now = Date.parse('2026-07-18T12:00:00Z')
  it('formats compact relative times', () => {
    expect(timeAgoIso('2026-07-18T11:59:30Z', now)).toBe('30s ago')
    expect(timeAgoIso('2026-07-18T11:30:00Z', now)).toBe('30m ago')
    expect(timeAgoIso('2026-07-18T09:00:00Z', now)).toBe('3h ago')
    expect(timeAgoIso('2026-07-10T12:00:00Z', now)).toBe('8d ago')
  })
  it('is empty for missing or malformed timestamps', () => {
    expect(timeAgoIso(undefined, now)).toBe('')
    expect(timeAgoIso(null, now)).toBe('')
    expect(timeAgoIso('not-a-date', now)).toBe('')
  })
})
