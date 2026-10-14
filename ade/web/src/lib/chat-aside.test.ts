import { describe, expect, it } from 'vitest'
import type { Conversation, Message } from '@/types/chat'
import {
  capRows,
  collectChatLinks,
  findSideChat,
  hideSideChats,
  parseGithubRemote,
  parseNumstat,
  parseStatusHeader,
  readGitSummary,
  readPullRequests,
  sideChatMessages,
  sideChatMetadata,
  sideChatQuote,
  summarizeChecks,
} from './chat-aside'

const chat = (
  id: string,
  createdAt: number,
  metadata?: Record<string, unknown>,
): Conversation => ({
  id,
  title: id,
  model: null,
  messages: [],
  createdAt,
  updatedAt: createdAt,
  sessionMetadata: metadata,
})

const say = (
  role: 'user' | 'assistant',
  content: string,
  createdAt = 1,
): Message => ({ id: `${role}-${createdAt}`, role, content, createdAt })

describe('side chats', () => {
  it('finds the newest side chat of a conversation and ignores the rest', () => {
    const list = [
      chat('main', 1),
      chat('old', 2, { side_chat_of: 'main' }),
      chat('new', 3, { side_chat_of: 'main' }),
      chat('other', 4, { side_chat_of: 'elsewhere' }),
    ]
    expect(findSideChat(list, 'main')?.id).toBe('new')
    expect(findSideChat(list, 'nobody')).toBeNull()
  })

  it('shows only what came after the fork', () => {
    const copied = say('assistant', 'copied', 10)
    const own = say('user', 'own', 20)
    expect(sideChatMessages([copied, own], 20)).toEqual([own])
  })

  it('quotes every line and trims long text', () => {
    expect(sideChatQuote(' one\n\ntwo ')).toBe('> one\n>\n> two\n\n')
    const long = sideChatQuote('x'.repeat(700))
    expect(long.endsWith('…\n\n')).toBe(true)
    expect(long.length).toBeLessThan(620)
  })
})

describe('capRows', () => {
  const rows = [1, 2, 3, 4, 5]
  it('caps and counts the rest', () => {
    expect(capRows(rows, 3, false)).toEqual({ shown: [1, 2, 3], hidden: 2 })
  })
  it('shows one extra row instead of "Show 1 more"', () => {
    expect(capRows(rows, 4, false)).toEqual({ shown: rows, hidden: 0 })
  })
  it('shows everything once expanded', () => {
    expect(capRows(rows, 2, true)).toEqual({ shown: rows, hidden: 0 })
  })
})

describe('collectChatLinks', () => {
  it('splits pull requests from other links, newest first, deduped', () => {
    const messages: Message[] = [
      say('user', 'see https://docs.example.com/console.', 1),
      say(
        'assistant',
        'Opened https://github.com/iii-hq/workers/pull/1371 and https://docs.example.com/console',
        2,
      ),
      say(
        'assistant',
        'Again: https://github.com/iii-hq/workers/pull/1371/files',
        3,
      ),
    ]
    const { links, pullRequests } = collectChatLinks(messages)
    expect(pullRequests).toEqual([
      {
        repo: 'iii-hq/workers',
        number: 1371,
        url: 'https://github.com/iii-hq/workers/pull/1371',
      },
    ])
    expect(links).toEqual([
      {
        url: 'https://docs.example.com/console',
        host: 'docs.example.com',
        label: 'console',
      },
    ])
  })

  it('reads the PR a github::pr::create call returned', () => {
    const call: Message = {
      id: 'c',
      role: 'function-trigger',
      functionId: 'github::pr::create',
      input: {},
      output: { url: 'https://github.com/o/r/pull/7' },
      createdAt: 1,
    }
    expect(collectChatLinks([call]).pullRequests[0]?.number).toBe(7)
  })
})

describe('git parsing', () => {
  it('reads owner/name from GitHub remotes only', () => {
    expect(parseGithubRemote('git@github.com:iii-hq/workers.git\n')).toBe(
      'iii-hq/workers',
    )
    expect(parseGithubRemote('https://github.com/iii-hq/workers')).toBe(
      'iii-hq/workers',
    )
    expect(parseGithubRemote('ssh://git@github.com/o/r.git')).toBe('o/r')
    expect(parseGithubRemote('git@gitlab.com:o/r.git')).toBeNull()
  })

  it('reads branch and ahead from the status header', () => {
    expect(
      parseStatusHeader('## feat/x...origin/feat/x [ahead 3, behind 1]'),
    ).toEqual({
      branch: 'feat/x',
      ahead: 3,
    })
    expect(parseStatusHeader('## main')).toEqual({ branch: 'main', ahead: 0 })
    expect(parseStatusHeader('## No commits yet on main')).toEqual({
      branch: 'main',
      ahead: 0,
    })
    expect(parseStatusHeader('## HEAD (no branch)')).toEqual({
      branch: null,
      ahead: 0,
    })
  })

  it('sums numstat and skips binary files', () => {
    expect(parseNumstat('10\t2\ta.ts\n-\t-\timg.png\n3\t0\tb.ts\n')).toEqual({
      added: 13,
      removed: 2,
    })
  })

  it('summarizes a repository and treats a failed status as not a repo', async () => {
    const outputs: Record<string, { exit_code: number; stdout: string }> = {
      status: {
        exit_code: 0,
        stdout: '## feat/x...origin/feat/x [ahead 2]\n M a.ts\n?? b.ts\n',
      },
      diff: { exit_code: 0, stdout: '5\t1\ta.ts\n' },
      remote: { exit_code: 0, stdout: 'git@github.com:o/r.git\n' },
    }
    const trigger = (async (_fn: string, payload?: Record<string, unknown>) => {
      const args = payload?.args as string[]
      return outputs[args.find((arg) => arg in outputs) ?? '']
    }) as Parameters<typeof readGitSummary>[0]
    expect(await readGitSummary(trigger, '/repo')).toEqual({
      branch: 'feat/x',
      ahead: 2,
      changed: 2,
      added: 5,
      removed: 1,
      repo: 'o/r',
    })
    outputs.status = { exit_code: 128, stdout: '' }
    expect(await readGitSummary(trigger, '/tmp')).toBeNull()
  })
})

describe('pull requests', () => {
  it('summarizes checks by their worst bucket', () => {
    expect(summarizeChecks([])).toBeNull()
    expect(summarizeChecks([{ bucket: 'pass' }, { bucket: 'pending' }])).toBe(
      'pending',
    )
    expect(summarizeChecks([{ bucket: 'pass' }, { bucket: 'fail' }])).toBe(
      'fail',
    )
    expect(summarizeChecks([{ bucket: 'pass' }, { bucket: 'skipping' }])).toBe(
      'pass',
    )
  })

  it('merges the branch PR with mentioned ones, open first, checks on open only', async () => {
    const calls: string[] = []
    const trigger = (async (fn: string, payload?: Record<string, unknown>) => {
      calls.push(`${fn} ${payload?.number ?? payload?.search ?? ''}`)
      if (fn === 'github::pr::list')
        return {
          value: [{ number: 9, title: 'Branch PR', url: 'u9', state: 'OPEN' }],
        }
      if (fn === 'github::pr::view')
        return {
          value: { number: 4, title: 'Old', url: 'u4', state: 'MERGED' },
        }
      if (fn === 'github::pr::checks') return { value: [{ bucket: 'fail' }] }
      throw new Error(fn)
    }) as Parameters<typeof readPullRequests>[0]
    const rows = await readPullRequests(trigger, {
      repo: 'o/r',
      branch: 'feat/x',
      mentioned: [
        { repo: 'o/r', number: 4, url: 'u4' },
        { repo: 'o/r', number: 9, url: 'u9' },
      ],
    })
    expect(rows.map((r) => [r.number, r.state, r.checks, r.mentioned])).toEqual(
      [
        [9, 'open', 'fail', false],
        [4, 'merged', null, true],
      ],
    )
    expect(calls).toEqual([
      'github::pr::list head:feat/x',
      'github::pr::view 4',
      'github::pr::checks 9',
    ])
  })
})

describe('review fixes', () => {
  it('keeps a link whose path has a malformed escape instead of throwing', () => {
    const { links } = collectChatLinks([
      say('assistant', 'GET https://api.example.com/users/%s please', 1),
    ])
    expect(links).toEqual([
      {
        url: 'https://api.example.com/users/%s',
        host: 'api.example.com',
        label: 'users/%s',
      },
    ])
  })

  it('treats pull request URLs case-insensitively', () => {
    const { pullRequests } = collectChatLinks([
      say('user', 'https://github.com/IIi-hq/Workers/pull/5', 1),
      say('user', 'https://github.com/iii-hq/workers/pull/5', 2),
    ])
    expect(pullRequests).toHaveLength(1)
  })

  it('reads GitHub remotes that carry a user or a port', () => {
    expect(parseGithubRemote('https://bot@github.com/o/r.git')).toBe('o/r')
    expect(parseGithubRemote('ssh://git@github.com:22/o/r.git')).toBe('o/r')
  })

  it('tags a fork and drops the harness links to its parent', () => {
    expect(
      sideChatMetadata(
        {
          agent_profile: { id: 'p' },
          parent_session_id: 'orchestrator',
          parent_function_call_id: 'call-1',
          depth: 1,
          spawned_by: 'agent',
          root_session_id: 'root',
          side_chat_of: 'stale',
        },
        'main',
      ),
    ).toEqual({ agent_profile: { id: 'p' }, side_chat_of: 'main' })
  })

  it('hides side chats and everything spawned inside them', () => {
    const list: Conversation[] = [
      chat('main', 1),
      chat('side', 2, { side_chat_of: 'main' }),
      { ...chat('helper', 3), parentId: 'side' },
      { ...chat('grandchild', 4), parentId: 'helper' },
      { ...chat('child', 5), parentId: 'main' },
    ]
    expect(hideSideChats(list).map((c) => c.id)).toEqual(['main', 'child'])
  })

  it('keeps only PRs whose head is the branch and skips default branches', async () => {
    const searched: unknown[] = []
    const trigger = (async (fn: string, payload?: Record<string, unknown>) => {
      if (fn === 'github::pr::list') {
        searched.push(payload?.search)
        return {
          value: [
            {
              number: 1,
              title: 'Mine',
              url: 'u1',
              state: 'OPEN',
              headRefName: 'feat/x',
            },
            {
              number: 2,
              title: 'Prefix',
              url: 'u2',
              state: 'OPEN',
              headRefName: 'feat/x-2',
            },
          ],
        }
      }
      return { value: [] }
    }) as Parameters<typeof readPullRequests>[0]
    const rows = await readPullRequests(trigger, {
      repo: 'o/r',
      branch: 'feat/x',
      mentioned: [],
    })
    expect(rows.map((r) => r.number)).toEqual([1])
    await readPullRequests(trigger, {
      repo: 'o/r',
      branch: 'main',
      mentioned: [],
    })
    expect(searched).toEqual(['head:feat/x'])
  })

  it('reads git with fsmonitor off and untracked dirs collapsed', async () => {
    const calls: string[][] = []
    const trigger = (async (_fn: string, payload?: Record<string, unknown>) => {
      calls.push(payload?.args as string[])
      return { exit_code: 0, stdout: '## main\n' }
    }) as Parameters<typeof readGitSummary>[0]
    await readGitSummary(trigger, '/repo')
    for (const args of calls)
      expect(args.slice(0, 3)).toEqual([
        '--no-optional-locks',
        '-c',
        'core.fsmonitor=false',
      ])
    expect(calls[0]).toContain('--untracked-files=normal')
  })
})
