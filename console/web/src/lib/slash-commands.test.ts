import { describe, expect, it } from 'vitest'
import {
  expandSlashInvocation,
  fuzzyFilterSlash,
  loadedSkillIds,
  mergeSlashEntries,
  parseSlashBlockHeader,
  parseSlashInvocation,
  SLASH_COMMANDS,
  setDynamicSlashEntries,
  slashAttachmentBlock,
  slashChip,
} from './slash-commands'

describe('parseSlashInvocation', () => {
  it('parses a leading /name as a prompt, args untouched', () => {
    expect(parseSlashInvocation('/review-pr the auth changes')).toEqual({
      kind: 'prompt',
      name: 'review-pr',
    })
    expect(parseSlashInvocation('/review-pr')).toEqual({
      kind: 'prompt',
      name: 'review-pr',
    })
  })

  it('parses /skill:<id> including slashes in the id', () => {
    expect(parseSlashInvocation('/skill:coder/index do the thing')).toEqual({
      kind: 'skill',
      id: 'coder/index',
    })
  })

  it('built-ins are not invocations', () => {
    expect(parseSlashInvocation('/compact')).toBeNull()
    expect(parseSlashInvocation('/compact now')).toBeNull()
  })

  it('a leading absolute path is not an invocation', () => {
    expect(parseSlashInvocation('/home/x/y is broken')).toBeNull()
  })

  it('plain text is not an invocation', () => {
    expect(parseSlashInvocation('hello /review-pr')).toBeNull()
    expect(parseSlashInvocation('either/or')).toBeNull()
  })
})

describe('mergeSlashEntries', () => {
  it('built-ins first; dynamic names shadowed by a built-in are dropped', () => {
    const merged = mergeSlashEntries([
      {
        command: '/compact',
        description: 'a prompt named compact',
        kind: 'prompt',
      },
      { command: '/review-pr', description: 'review a pr', kind: 'prompt' },
    ])
    expect(merged.slice(0, SLASH_COMMANDS.length)).toEqual(SLASH_COMMANDS)
    expect(merged.filter((c) => c.command === '/compact')).toHaveLength(1)
    expect(merged.some((c) => c.command === '/review-pr')).toBe(true)
  })
})

describe('slashAttachmentBlock', () => {
  it('wraps prompt and skill bodies distinctly', () => {
    expect(
      slashAttachmentBlock({ kind: 'prompt', name: 'review-pr' }, 'Check X.'),
    ).toBe('<command name="review-pr">\nCheck X.\n</command>')
    expect(
      slashAttachmentBlock({ kind: 'skill', id: 'coder/index' }, 'Skill.'),
    ).toBe(
      '<skill id="coder/index">\nThis skill is already loaded. Follow it directly; do not search for or reload it.\n\nSkill.\n</skill>',
    )
  })
})

describe('parseSlashBlockHeader', () => {
  it('round-trips both block shapes back to their invocation', () => {
    for (const inv of [
      { kind: 'prompt', name: 'review-pr' },
      { kind: 'skill', id: 'coder/index' },
    ] as const) {
      expect(parseSlashBlockHeader(slashAttachmentBlock(inv, 'body'))).toEqual(
        inv,
      )
    }
  })

  it('ignores plain text and attached-file blocks', () => {
    expect(parseSlashBlockHeader('hello <command name="x">')).toBeNull()
    expect(
      parseSlashBlockHeader('<attached-file path="a.rs">x</attached-file>'),
    ).toBeNull()
  })
})

describe('slashChip', () => {
  it('builds the collapsed chip per kind', () => {
    expect(slashChip({ kind: 'prompt', name: 'review-pr' }, 42)).toEqual({
      id: 'slash-/review-pr',
      name: '/review-pr',
      size: 42,
      type: 'text/x-slash-command',
    })
    expect(slashChip({ kind: 'skill', id: 'coder/index' }, 7)).toEqual({
      id: 'slash-/skill:coder/index',
      name: '/skill:coder/index',
      size: 7,
      type: 'text/x-skill',
    })
  })
})

describe('expandSlashInvocation gate', () => {
  /* Only the palette-known gate is unit-testable (it returns before any
     client is touched); the resolution path needs a live bus. */
  it('never expands before the palette has fetched entries', async () => {
    setDynamicSlashEntries(null)
    expect(await expandSlashInvocation('/review-pr x')).toBeNull()
  })

  it('never expands slugs the palette did not offer', async () => {
    setDynamicSlashEntries([
      { command: '/review-pr', description: 'review a pr', kind: 'prompt' },
    ])
    expect(await expandSlashInvocation('/etc is full')).toBeNull()
    expect(await expandSlashInvocation('/compact now')).toBeNull()
    expect(await expandSlashInvocation('/skill:coder/index go')).toBeNull()
    setDynamicSlashEntries(null)
  })

  /* The dedupe branch also returns before any client is touched. */
  it('an already-loaded skill expands to a pointer without a refetch', async () => {
    setDynamicSlashEntries([
      { command: '/skill:coder/index', description: 'coder', kind: 'skill' },
    ])
    const result = await expandSlashInvocation(
      '/skill:coder/index go',
      new Set(['coder/index']),
    )
    expect(result?.status).toBe('attached')
    if (result?.status !== 'attached') throw new Error('unreachable')
    expect(result.block).toContain('already loaded')
    expect(result.block.length).toBeLessThan(200)
    // The pointer still collapses to the same chip on hydration.
    expect(parseSlashBlockHeader(result.block)).toEqual({
      kind: 'skill',
      id: 'coder/index',
    })
    setDynamicSlashEntries(null)
  })
})

describe('loadedSkillIds', () => {
  const skillMsg = {
    role: 'user',
    attachments: [slashChip({ kind: 'skill', id: 'coder/index' }, 7)],
  }

  it('collects skill chips from prior messages', () => {
    expect(loadedSkillIds([skillMsg]).has('coder/index')).toBe(true)
  })

  it('a compaction marker resets what counts as loaded', () => {
    expect(
      loadedSkillIds([skillMsg, { role: 'system', kind: 'compaction' }]).size,
    ).toBe(0)
  })

  it('prompt chips and plain attachments never count', () => {
    expect(
      loadedSkillIds([
        {
          role: 'user',
          attachments: [
            slashChip({ kind: 'prompt', name: 'review-pr' }, 7),
            { name: 'a.pdf', type: 'application/pdf' },
          ],
        },
      ]).size,
    ).toBe(0)
  })
})

describe('fuzzyFilterSlash', () => {
  const entries = mergeSlashEntries([
    { command: '/review-pr', description: 'review a pr', kind: 'prompt' },
    { command: '/skill:coder/index', description: 'coder', kind: 'skill' },
  ])

  it('empty query returns everything up to the limit', () => {
    expect(fuzzyFilterSlash('', entries)).toHaveLength(entries.length)
  })

  it('matches command and description substrings across kinds', () => {
    expect(fuzzyFilterSlash('review', entries).map((c) => c.command)).toEqual([
      '/review-pr',
    ])
    expect(
      fuzzyFilterSlash('skill:cod', entries).map((c) => c.command),
    ).toEqual(['/skill:coder/index'])
  })
})
