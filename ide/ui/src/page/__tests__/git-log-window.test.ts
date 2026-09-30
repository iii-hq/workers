import { describe, expect, it, vi } from 'vitest'
import {
  fromFolder,
  labelsBySha,
  logArgs,
  logTips,
  parseRecords,
  parseRefs,
  type RefsSnapshot,
  readRefs,
  refsTree,
  showsGraph,
} from '../git-log-window'

const exec = (
  overrides: Partial<{ exit_code: number; stdout: string; stderr: string; stdout_truncated: boolean }>,
) => ({
  exit_code: 0,
  stdout: '',
  stderr: '',
  timed_out: false,
  stdout_truncated: false,
  stderr_truncated: false,
  ...overrides,
})

const ref = (...values: string[]) => `${values.join('\0')}\0\n`
const A = 'a'.repeat(40)
const B = 'b'.repeat(40)
const LISTING =
  ref('refs/heads/feat/x', A, ' ', '', '', '', '', 'feat work', '100') +
  ref('refs/heads/main', B, '*', '', 'origin/main', 'ahead 1, behind 2', '/r/main\nfolder', 'tip', '200') +
  ref('refs/remotes/origin/HEAD', A, ' ', 'refs/remotes/origin/main', '', '', '', 'x', '1') +
  ref('refs/remotes/origin/main', A, ' ', '', '', '', '', 'x', '1') +
  ref('refs/tags/v1', A, ' ', '', '', '', '', 'tag message', '')

describe('parseRecords', () => {
  it('keeps the complete records and drops what the output cap cut', () => {
    expect(parseRecords(`a\0b\0\nc\0d\0\n`, 2)).toEqual([
      ['a', 'b'],
      ['c', 'd'],
    ])
    expect(parseRecords(`a\0b\0\nc\0`, 2)).toEqual([['a', 'b']])
    expect(parseRecords('', 2)).toEqual([])
  })
})

describe('parseRefs', () => {
  it('reads kinds, upstream counts, worktrees and tips, and leaves symbolic refs out', () => {
    const refs = parseRefs(LISTING)
    expect(refs.map((r) => r.name)).toEqual(['feat/x', 'main', 'origin/main', 'v1'])
    expect(refs[1]).toMatchObject({
      kind: 'local',
      current: true,
      upstream: 'origin/main',
      ahead: 1,
      behind: 2,
      worktree: '/r/main\nfolder',
      subject: 'tip',
      date: 200,
    })
    expect(refs[3]).toMatchObject({ kind: 'tag', sha: A })
    expect(refs[3].date).toBeUndefined()
    expect(parseRefs(ref('refs/heads/g', A, ' ', '', 'origin/g', 'gone', '', 's', '1'))[0].gone).toBe(true)
  })
})

describe('readRefs', () => {
  const host = (probe: ReturnType<typeof exec>) =>
    ({
      iii: {
        trigger: vi.fn(async (_fn: string, payload: { args: string[] }) =>
          payload.args[0] === 'rev-parse' ? probe : exec({ stdout: LISTING }),
        ),
      },
    }) as unknown as Parameters<typeof readRefs>[0]

  it('reads HEAD, the prefix and a shallow clone; an unborn HEAD is null', async () => {
    const born = await readRefs(host(exec({ stdout: `true\nsub/\n${B}\n` })), '/r')
    expect(born).toMatchObject({ head: B, shallow: true, prefix: 'sub/', truncated: false })
    expect(born?.signature.startsWith(`${B}\n`)).toBe(true)
    const unborn = await readRefs(host(exec({ exit_code: 1, stdout: 'false\n\n' })), '/r')
    expect(unborn?.head).toBeNull()
  })

  it('is null outside a repository', async () => {
    const outside = exec({
      exit_code: 128,
      stderr: 'fatal: not a git repository (or any of the parent directories): .git',
    })
    expect(await readRefs(host(outside), '/r')).toBeNull()
  })
})

describe('the log', () => {
  const snapshot: RefsSnapshot = {
    refs: parseRefs(LISTING),
    head: B,
    shallow: false,
    prefix: '',
    truncated: false,
    signature: '',
  }

  it('starts from every ref and HEAD, or from one ref', () => {
    expect(new Set(logTips(snapshot, null))).toEqual(new Set([A, B]))
    expect(logTips(snapshot, 'refs/heads/main')).toEqual([B])
    expect(logTips(snapshot, 'HEAD')).toEqual([B])
    expect(logTips(snapshot, 'refs/heads/gone')).toEqual([])
  })

  it('maps filters to flags, and draws the graph only for branch and path filters', () => {
    expect(logArgs({}, 0, 10)).toEqual([
      '-c',
      'log.follow=false',
      'log',
      '--stdin',
      '--topo-order',
      '--no-color',
      '--no-show-signature',
      '--format=%H%x00%P%x00%aN%x00%aE%x00%at%x00%s%x00',
      '--skip=0',
      '--max-count=10',
    ])
    const args = logArgs({ text: 'fix', author: 'ana', since: 5, until: 9, paths: ['a/b'] }, 20, 10)
    expect(args).toEqual(
      expect.arrayContaining([
        '--grep=fix',
        '--fixed-strings',
        '--regexp-ignore-case',
        '--author=ana',
        '--since=@5',
        '--until=@9',
        '--parents',
      ]),
    )
    expect(args.slice(-2)).toEqual(['--', 'a/b'])
    expect(logArgs({ text: 'a.b', regex: true, caseSensitive: true }, 0, 1)).toEqual(
      expect.arrayContaining(['--extended-regexp']),
    )
    expect(logArgs({ text: 'a.b', regex: true, caseSensitive: true }, 0, 1)).not.toContain('--regexp-ignore-case')
    // An author name is literal: fixed on its own, escaped beside a regex.
    expect(logArgs({ author: 'dependabot[bot]' }, 0, 1)).toEqual(
      expect.arrayContaining(['--fixed-strings', '--author=dependabot[bot]']),
    )
    expect(logArgs({ text: 'x', regex: true, author: 'dependabot[bot]' }, 0, 1)).toContain(
      '--author=dependabot\\[bot\\]',
    )
    expect(showsGraph({ paths: ['a'] })).toBe(true)
    expect(showsGraph({ author: 'ana' })).toBe(false)
  })

  it('sees a repository path from the browsed folder, climbing out with ../', () => {
    expect(fromFolder('', 'ide/src/x.rs')).toBe('ide/src/x.rs')
    expect(fromFolder('ide/', 'ide/src/x.rs')).toBe('src/x.rs')
    expect(fromFolder('template/', 'ide/src/x.rs')).toBe('../ide/src/x.rs')
    expect(fromFolder('a/b/', 'a/c.txt')).toBe('../c.txt')
    expect(fromFolder('a/b/', 'README.md')).toBe('../../README.md')
  })

  it('labels each commit, current branch first, tags last', () => {
    expect(
      labelsBySha(snapshot)
        .get(A)
        ?.map((r) => r.name),
    ).toEqual(['feat/x', 'origin/main', 'v1'])
    expect(
      labelsBySha(snapshot)
        .get(B)
        ?.map((r) => r.name),
    ).toEqual(['main'])
  })

  it('builds the branches tree: HEAD, local (default first) and remote grouped by prefix, tags', () => {
    const tree = refsTree(snapshot, 'main')
    expect(tree.map((node) => node.id)).toEqual(['head', 'local', 'remote', 'tag'])
    const local = tree[1]
    expect(local.kind === 'section' && local.children.map((node) => node.id)).toEqual([
      'ref:refs/heads/main',
      'ref:refs/heads/feat/x',
    ])
    const remote = tree[2]
    expect(remote.kind === 'section' && remote.children[0]).toMatchObject({
      kind: 'folder',
      id: 'remote/origin',
      children: [{ kind: 'ref', label: 'main' }],
    })
  })
})
