import { describe, expect, it } from 'vitest'
import type { Conversation, Message } from '@/types/chat'
import {
  autoFixTriggers,
  autoFixWatchId,
  capRows,
  collectChatImages,
  collectChatLinks,
  collectChatTickets,
  dismissedPullRequests,
  findSideChat,
  folderGh,
  ghSource,
  githubWorkerGh,
  hideSideChats,
  type KanbanBoard,
  lastSpokenEntry,
  parseGithubRemote,
  parseNumstat,
  parseStatusHeader,
  pullRequestCommand,
  readGitSummary,
  readPullRequests,
  runPullRequestAction,
  sideChatMessages,
  sideChatMetadata,
  sideChatQuote,
  sideChatQuotes,
  splitQuotedMessage,
  summarizeChecks,
  ticketRows,
  withSideChatQuote,
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
    const trigger = workerExec((verb) => {
      if (verb === 'list')
        return [
          {
            number: 9,
            title: 'Branch PR',
            url: 'u9',
            state: 'OPEN',
            additions: 447,
            deletions: 50,
            autoMergeRequest: { enabledAt: 'x' },
          },
        ]
      if (verb === 'view')
        return { number: 4, title: 'Old', url: 'u4', state: 'MERGED' }
      return [{ bucket: 'fail' }, { bucket: 'pass' }, { bucket: 'cancel' }]
    }, calls)
    const rows = await readPullRequests(ghSource(githubWorkerGh(trigger)), {
      repo: 'o/r',
      branch: 'feat/x',
      mentioned: [
        { repo: 'o/r', number: 4, url: 'u4' },
        { repo: 'o/r', number: 9, url: 'u9' },
      ],
    })
    expect(
      rows.map((r) => [
        r.number,
        r.state,
        r.checks,
        r.failing,
        r.additions,
        r.autoMerge,
        r.mentioned,
      ]),
    ).toEqual([
      [9, 'open', 'fail', 2, 447, true, false],
      [4, 'merged', null, 0, null, false, true],
    ])
    expect(calls).toEqual(['list feat/x', 'view 4', 'checks 9'])
  })
})

/** The github worker's `github::exec`: answers `gh pr list|view|checks`. */
function workerExec(
  answer: (verb: string, args: string[]) => unknown,
  calls: string[] = [],
) {
  return (async (fn: string, payload?: Record<string, unknown>) => {
    if (fn !== 'github::exec') throw new Error(fn)
    const args = payload?.args as string[]
    const verb = args[1]
    calls.push(
      `${verb} ${verb === 'list' ? args[args.indexOf('--head') + 1] : args[2]}`,
    )
    const value = answer(verb, args)
    return {
      value: { exit_code: 0, stdout: JSON.stringify(value), stderr: '' },
    }
  }) as Parameters<typeof githubWorkerGh>[0]
}

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
    const calls: string[] = []
    const trigger = workerExec(
      (verb) =>
        verb === 'list'
          ? [
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
            ]
          : [],
      calls,
    )
    const source = ghSource(githubWorkerGh(trigger))
    const rows = await readPullRequests(source, {
      repo: 'o/r',
      branch: 'feat/x',
      mentioned: [],
    })
    expect(rows.map((r) => r.number)).toEqual([1])
    await readPullRequests(source, {
      repo: 'o/r',
      branch: 'main',
      mentioned: [],
    })
    expect(calls.filter((c) => c.startsWith('list'))).toEqual(['list feat/x'])
  })

  it('reads git with fsmonitor off, counting untracked files like the IDE', async () => {
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
    expect(calls[0]).toContain('--untracked-files=all')
  })
})

describe('side chat quotes', () => {
  it('reads recorded quotes and skips malformed ones', () => {
    expect(
      sideChatQuotes({
        sessionMetadata: {
          side_chat_quotes: [
            { message_id: 'm1', text: 'a', agent: 'Bot' },
            { message_id: 2, text: 'bad' },
            'junk',
            { message_id: 'm3', text: 'c' },
          ],
        },
      }),
    ).toEqual([
      { messageId: 'm1', text: 'a', agent: 'Bot' },
      { messageId: 'm3', text: 'c' },
    ])
    expect(sideChatQuotes(null)).toEqual([])
  })

  it('records a quote last and keeps the newest twenty', () => {
    const many = Array.from({ length: 20 }, (_, i) => ({
      messageId: `m${i}`,
      text: `t${i}`,
    }))
    const stored = withSideChatQuote(many, {
      messageId: 'new',
      text: 'n',
      agent: 'A',
    })
    expect(stored).toHaveLength(20)
    expect(stored[0]).toEqual({ message_id: 'm1', text: 't1' })
    expect(stored.at(-1)).toEqual({ message_id: 'new', text: 'n', agent: 'A' })
  })

  it('splits only the exact quote the side chat sent', () => {
    const quote = { messageId: 'm1', text: 'line one\nline two' }
    expect(
      splitQuotedMessage(`${sideChatQuote(quote.text)}why?`, [quote]),
    ).toEqual({ quote, rest: 'why?' })
    expect(splitQuotedMessage('> other\n\nwhy?', [quote])).toBeNull()
  })
})

describe('side chat anchor', () => {
  const copiedNote: Message = {
    id: 'c-note',
    role: 'system',
    kind: 'model-note',
    tone: 'info',
    content: 'Note to the model',
    // A transcript page gives notes no time; they are stamped at load.
    createdAt: 9_999,
  }
  const anchorText = say('assistant', 'last copied reply', 5)
  const anchored = [
    say('user', 'copied question', 4),
    copiedNote,
    { ...anchorText, id: 'f1:0' },
    say('user', 'own question', 30),
  ]

  it('shows only what follows the copied entry, notes included', () => {
    expect(sideChatMessages(anchored, 20, 'f1').map((m) => m.id)).toEqual([
      anchored[3].id,
    ])
  })

  it('falls back to the fork time without an anchor in the window', () => {
    expect(sideChatMessages(anchored, 20, 'gone').map((m) => m.id)).toEqual([
      'c-note',
      anchored[3].id,
    ])
  })

  it('cuts at the last user or assistant entry', () => {
    expect(
      lastSpokenEntry([
        { entry_id: 'a', message: { role: 'user' } },
        { entry_id: 'b', message: { role: 'assistant' } },
        { entry_id: 'c', message: { role: 'function_result' } },
      ]),
    ).toBe('b')
    expect(
      lastSpokenEntry([
        { entry_id: 'c', message: { role: 'function_result' } },
      ]),
    ).toBeNull()
  })

  it('records the anchor next to the tag', () => {
    expect(sideChatMetadata({}, 'main', 'f1')).toEqual({
      side_chat_of: 'main',
      side_chat_from: 'f1',
    })
  })
})

describe('pull requests through gh in the folder', () => {
  it('asks gh for the branch head and reads checks whatever the exit code', async () => {
    const calls: string[][] = []
    const trigger = (async (_fn: string, payload?: Record<string, unknown>) => {
      const args = payload?.args as string[]
      calls.push(args)
      expect(payload?.command).toBe('gh')
      expect(payload?.cwd).toBe('/repo')
      if (args[1] === 'list')
        return {
          exit_code: 0,
          stdout: JSON.stringify([
            {
              number: 3,
              title: 'Mine',
              url: 'u3',
              state: 'OPEN',
              headRefName: 'feat/x',
            },
          ]),
        }
      // `gh pr checks` exits 8 while checks are pending.
      return { exit_code: 8, stdout: '[{"bucket":"pending"}]' }
    }) as Parameters<typeof folderGh>[0]
    const rows = await readPullRequests(ghSource(folderGh(trigger, '/repo')), {
      repo: 'o/r',
      branch: 'feat/x',
      mentioned: [],
    })
    expect(rows.map((r) => [r.number, r.checks])).toEqual([[3, 'pending']])
    expect(calls[0]).toEqual([
      'pr',
      'list',
      '-R',
      'o/r',
      '--head',
      'feat/x',
      '--state',
      'all',
      '--limit',
      '5',
      '--json',
      'number,title,url,state,isDraft,headRefName,additions,deletions,autoMergeRequest',
    ])
  })

  it('reads no pull requests when gh is missing or prints nothing', async () => {
    const trigger = (async () => {
      throw new Error('gh: not found')
    }) as Parameters<typeof folderGh>[0]
    expect(
      await readPullRequests(ghSource(folderGh(trigger, '/repo')), {
        repo: 'o/r',
        branch: 'feat/x',
        mentioned: [],
      }),
    ).toEqual([])
  })
})

describe('collectChatImages', () => {
  it('lists attached and tool images, newest first', () => {
    const attached: Message = {
      id: 'u1',
      role: 'user',
      content: 'look',
      createdAt: 1,
      attachments: [
        {
          id: 'a1',
          name: 'shot.png',
          size: 10,
          type: 'image/png',
          attachmentId: 'a1',
        },
        { id: 'a2', name: 'notes.md', size: 10, type: 'text/markdown' },
      ],
    }
    const tool: Message = {
      id: 'c1',
      role: 'function-trigger',
      functionId: 'browser::screenshot',
      input: {},
      output: {
        content: [
          { type: 'text', text: 'done' },
          { type: 'image', data: 'AAAA', mimeType: 'image/jpeg' },
        ],
        details: { preview: 'data:image/webp;base64,BBBB' },
      },
      createdAt: 2,
    }
    const images = collectChatImages([attached, tool])
    expect(images.map((i) => [i.id, i.type])).toEqual([
      ['c1:image-1', 'image/jpeg'],
      ['c1:image-2', 'image/webp'],
      ['a1', 'image/png'],
    ])
    expect(images[0].dataUrl).toBe('data:image/jpeg;base64,AAAA')
    // Cached by message: the same objects come back.
    expect(collectChatImages([attached, tool])[2]).toBe(images[2])
  })
})

describe('collectChatTickets', () => {
  const call = (
    functionId: string,
    input: unknown,
    output: unknown,
    createdAt: number,
  ): Message =>
    ({
      id: `${functionId}-${createdAt}`,
      role: 'function-trigger',
      functionId,
      input,
      output,
      createdAt,
    }) as Message

  it('reads its own kanban calls, the latest per ticket winning', () => {
    const touches = collectChatTickets([
      call('kanban::ticket::create', { title: 'x' }, { key: 'KAN-9' }, 1),
      call('kanban::ticket::update', { id: 'KAN-9' }, null, 2),
      call('kanban::ticket::move', '{"id":"KAN-9","status":"done"}', null, 3),
      call('kanban::comment::create', { ticket_id: 'uuid-1' }, null, 4),
      call(
        'kanban::ticket::create',
        {},
        { content: [], details: { key: 'KAN-10' } },
        5,
      ),
    ])
    expect(touches).toEqual([
      { ref: 'KAN-9', action: 'moved', at: 3 },
      { ref: 'uuid-1', action: 'commented', at: 4 },
      { ref: 'KAN-10', action: 'created', at: 5 },
    ])
  })

  it('credits a sub-agent by its instruction or title, a call beating both', () => {
    const touches = collectChatTickets(
      [
        call(
          'harness::spawn',
          { task: 'Fix KAN-12 and KAN-13', display: { name: 'Implementer' } },
          null,
          1,
        ),
        call('kanban::ticket::move', { id: 'KAN-13' }, null, 2),
      ],
      [{ name: 'Reviewer', title: 'KAN-14 review', at: 3 }],
    )
    expect(touches).toEqual([
      { ref: 'KAN-12', by: 'Implementer', at: 1 },
      { ref: 'KAN-13', action: 'moved', at: 2 },
      { ref: 'KAN-14', by: 'Reviewer', at: 3 },
    ])
  })

  it('takes keys and @kanban mentions from what was said', () => {
    const touches = collectChatTickets([
      {
        id: 'u',
        role: 'user',
        content: 'Look at KAN-7 and @kanban(id="uuid-2")',
        createdAt: 1,
      },
      {
        id: 'a',
        role: 'assistant',
        content: 'KAN-7 is done; UTF-8 too',
        createdAt: 2,
      },
    ] as Message[])
    expect(touches).toEqual([
      { ref: 'KAN-7', at: 2 },
      { ref: 'uuid-2', at: 1 },
      { ref: 'UTF-8', at: 2 },
    ])
  })
})

describe('ticketRows', () => {
  const ticket = (
    id: string,
    key: string,
    deletedAt: string | null = null,
  ) => ({
    id,
    key,
    title: key,
    status: 'in_review',
    priority: 'low',
    deletedAt,
  })
  const board: KanbanBoard = {
    columns: new Map([['in_review', 'In review']]),
    tickets: new Map(
      [
        ticket('u1', 'KAN-1'),
        ticket('u2', 'KAN-2'),
        ticket('u3', 'KAN-3', '2026-10-01'),
        ticket('u4', 'KAN-4'),
      ].flatMap((t) => [
        [t.key, t],
        [t.id, t],
      ]),
    ),
  }

  it('keeps what the board knows: worked newest first, mentioned, deleted', () => {
    const rows = ticketRows(
      [
        { ref: 'KAN-1', at: 9 },
        { ref: 'u1', action: 'moved', at: 2 },
        { ref: 'KAN-2', by: 'Implementer', at: 5 },
        { ref: 'KAN-3', action: 'created', at: 8 },
        { ref: 'KAN-4', at: 7 },
        { ref: 'UTF-8', at: 9 },
      ],
      board,
    )
    expect(
      rows.map((row) => [row.ticket.key, row.action ?? row.by ?? '-']),
    ).toEqual([
      ['KAN-2', 'Implementer'],
      ['KAN-1', 'moved'],
      ['KAN-4', '-'],
      ['KAN-3', 'created'],
    ])
    expect(rows[0].status).toBe('In review')
  })
})

describe('pull request actions', () => {
  const pr = { repo: 'o/r', number: 7, url: 'https://github.com/o/r/pull/7' }

  it('spells each action as gh does', () => {
    expect(pullRequestCommand(pr, 'merge-when-ready')).toEqual([
      'pr',
      'merge',
      '7',
      '-R',
      'o/r',
      '--auto',
      '--squash',
    ])
    expect(pullRequestCommand(pr, 'cancel-merge-when-ready')).toEqual([
      'pr',
      'merge',
      '7',
      '-R',
      'o/r',
      '--disable-auto',
    ])
    expect(pullRequestCommand(pr, 'draft')).toEqual([
      'pr',
      'ready',
      '7',
      '-R',
      'o/r',
      '--undo',
    ])
    expect(pullRequestCommand(pr, 'ready')).toEqual([
      'pr',
      'ready',
      '7',
      '-R',
      'o/r',
    ])
  })

  it('reports what gh said when an action fails', async () => {
    const ok = async () => ({ exitCode: 0, stdout: '', stderr: '' })
    const refused = async () => ({
      exitCode: 1,
      stdout: '',
      stderr: 'Pull request is in clean status\n',
    })
    expect(await runPullRequestAction(ok, pr, 'ready')).toBeNull()
    expect(await runPullRequestAction(refused, pr, 'merge-when-ready')).toBe(
      'Pull request is in clean status',
    )
    expect(await runPullRequestAction(async () => null, pr, 'draft')).toBe(
      'gh could not run.',
    )
  })

  it('finds the chat subscriptions that wake it on a PR', () => {
    const triggers = [
      {
        id: 'a',
        triggerType: 'github::pr::event',
        config: { watch_id: autoFixWatchId(pr) },
      },
      {
        id: 'b',
        triggerType: 'github::pr::event',
        config: { repo: 'O/R', number: 7 },
      },
      {
        id: 'c',
        triggerType: 'github::pr::event',
        config: { repo: 'o/r', number: 8 },
      },
      {
        id: 'd',
        triggerType: 'cron',
        config: { watch_id: autoFixWatchId(pr) },
      },
      {
        id: 'e',
        triggerType: 'github::pr::event',
        fired: true,
        config: { repo: 'o/r', number: 7 },
      },
    ]
    expect(autoFixTriggers(triggers, pr).map((t) => t.id)).toEqual(['a', 'b'])
    expect(autoFixWatchId(pr)).toBe('ade-autofix-o-r-7')
  })

  it('reads the dismissed PRs off the session metadata', () => {
    expect(
      dismissedPullRequests({
        sessionMetadata: { context_dismissed_prs: ['o/r#7', 3] },
      }),
    ).toEqual(['o/r#7'])
    expect(dismissedPullRequests(null)).toEqual([])
  })
})
