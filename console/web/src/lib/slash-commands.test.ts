import { describe, expect, it } from 'vitest'
import {
  expandSlashInvocations,
  fuzzyFilterSlash,
  loadedSkillIds,
  mergeSlashEntries,
  parseSlashBlockHeader,
  parseSlashInvocations,
  SLASH_COMMANDS,
  setDynamicSlashEntries,
  slashAttachmentBlock,
  slashChip,
  slashCommandLabel,
} from './slash-commands'

describe('parseSlashInvocations', () => {
  it('leaves a leading /name as ordinary text', () => {
    expect(parseSlashInvocations('/review-pr the auth changes')).toEqual([])
    expect(parseSlashInvocations('/review-pr')).toEqual([])
  })

  it('parses /skill:<id> including slashes in the id', () => {
    expect(parseSlashInvocations('/skill:coder/index do the thing')).toEqual([
      { kind: 'skill', id: 'coder/index' },
    ])
  })

  it('finds an invocation anywhere after whitespace or a paren', () => {
    expect(parseSlashInvocations('please run /skill:review-pr on it')).toEqual([
      { kind: 'skill', id: 'review-pr' },
    ])
    expect(parseSlashInvocations('(/skill:a)')).toEqual([
      { kind: 'skill', id: 'a' },
    ])
  })

  it('lists each id once, in order of first appearance', () => {
    expect(
      parseSlashInvocations('/skill:b then /skill:a then /skill:b again'),
    ).toEqual([
      { kind: 'skill', id: 'b' },
      { kind: 'skill', id: 'a' },
    ])
  })

  it('keeps a trailing period out of the id', () => {
    expect(parseSlashInvocations('use /skill:coder/index.')).toEqual([
      { kind: 'skill', id: 'coder/index' },
    ])
    expect(parseSlashInvocations('use /skill:v1.2/x, ok')).toEqual([
      { kind: 'skill', id: 'v1.2/x' },
    ])
  })

  it('built-ins are not invocations', () => {
    expect(parseSlashInvocations('/compact')).toEqual([])
    expect(parseSlashInvocations('/compact now')).toEqual([])
  })

  it('an absolute path or a slash inside a word is not an invocation', () => {
    expect(parseSlashInvocations('/home/x/y is broken')).toEqual([])
    expect(parseSlashInvocations('either/skill:or')).toEqual([])
    expect(parseSlashInvocations('either/or')).toEqual([])
  })
})

describe('slashCommandLabel', () => {
  it('drops the slash and the skill namespace', () => {
    expect(slashCommandLabel('/skill:coder/index')).toBe('coder/index')
    expect(slashCommandLabel('/compact')).toBe('compact')
  })
})

describe('mergeSlashEntries', () => {
  it('built-ins first; dynamic names shadowed by a built-in are dropped', () => {
    const merged = mergeSlashEntries([
      {
        command: '/compact',
        description: 'a skill named compact',
      },
      { command: '/skill:review-pr', description: 'review a pr' },
    ])
    expect(merged.slice(0, SLASH_COMMANDS.length)).toEqual(SLASH_COMMANDS)
    expect(merged.filter((c) => c.command === '/compact')).toHaveLength(1)
    expect(merged.some((c) => c.command === '/skill:review-pr')).toBe(true)
  })
})

describe('slashAttachmentBlock', () => {
  it('wraps skill bodies', () => {
    expect(
      slashAttachmentBlock({ kind: 'skill', id: 'coder/index' }, 'Skill.'),
    ).toBe(
      '<skill id="coder/index">\nThis skill is already loaded. Follow it directly; do not search for or reload it.\n\nSkill.\n</skill>',
    )
  })
})

describe('parseSlashBlockHeader', () => {
  it('round-trips skill blocks back to their invocation', () => {
    const inv = { kind: 'skill', id: 'coder/index' } as const
    expect(parseSlashBlockHeader(slashAttachmentBlock(inv, 'body'))).toEqual(
      inv,
    )
  })

  it('ignores plain text and attached-file blocks', () => {
    expect(parseSlashBlockHeader('hello <command name="x">')).toBeNull()
    expect(
      parseSlashBlockHeader('<attached-file path="a.rs">x</attached-file>'),
    ).toBeNull()
    expect(
      parseSlashBlockHeader('<command name="review-pr">x</command>'),
    ).toBeNull()
  })
})

describe('slashChip', () => {
  it('builds the collapsed skill chip', () => {
    expect(slashChip({ kind: 'skill', id: 'coder/index' }, 7)).toEqual({
      id: 'slash-/skill:coder/index',
      name: '/skill:coder/index',
      size: 7,
      type: 'text/x-skill',
    })
  })
})

describe('expandSlashInvocations gate', () => {
  /* Only the palette-known gate is unit-testable (it returns before any
     client is touched); the resolution path needs a live bus. */
  it('never expands before the palette has fetched entries', async () => {
    setDynamicSlashEntries(null)
    expect(await expandSlashInvocations('/skill:review-pr x')).toEqual([])
  })

  it('never expands slugs the palette did not offer', async () => {
    setDynamicSlashEntries([
      { command: '/skill:review-pr', description: 'review a pr' },
    ])
    expect(await expandSlashInvocations('/review-pr x')).toEqual([])
    expect(await expandSlashInvocations('/etc is full')).toEqual([])
    expect(await expandSlashInvocations('/compact now')).toEqual([])
    expect(await expandSlashInvocations('/skill:coder/index go')).toEqual([])
    setDynamicSlashEntries(null)
  })

  /* The dedupe branch also returns before any client is touched. */
  it('an already-loaded skill expands to a pointer without a refetch', async () => {
    setDynamicSlashEntries([
      { command: '/skill:coder/index', description: 'coder' },
    ])
    const [result] = await expandSlashInvocations(
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

  it('expands every offered invocation in the text, mid-sentence too', async () => {
    setDynamicSlashEntries([
      { command: '/skill:coder/index', description: 'coder' },
      { command: '/skill:review-pr', description: 'review' },
    ])
    const results = await expandSlashInvocations(
      'fix it with /skill:coder/index then /skill:review-pr and /skill:unknown',
      new Set(['coder/index', 'review-pr']),
    )
    expect(results.map((r) => r.status === 'attached' && r.inv.id)).toEqual([
      'coder/index',
      'review-pr',
    ])
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

  it('plain attachments never count', () => {
    expect(
      loadedSkillIds([
        {
          role: 'user',
          attachments: [{ name: 'a.pdf', type: 'application/pdf' }],
        },
      ]).size,
    ).toBe(0)
  })
})

describe('fuzzyFilterSlash', () => {
  const entries = mergeSlashEntries([
    { command: '/skill:coder/index', description: 'coder' },
  ])

  it('empty query returns everything up to the limit', () => {
    expect(fuzzyFilterSlash('', entries)).toHaveLength(entries.length)
  })

  it('matches command and description substrings', () => {
    expect(
      fuzzyFilterSlash('skill:cod', entries).map((c) => c.command),
    ).toEqual(['/skill:coder/index'])
    expect(fuzzyFilterSlash('cod', entries).map((c) => c.command)).toEqual([
      '/skill:coder/index',
    ])
  })
})
