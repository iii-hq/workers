import { describe, expect, it } from 'vitest'
import { derivedKind, parsePatch } from '../ApplyPatchView'

const FULL_PATCH = [
  '*** Begin Patch',
  '*** Update File: src/index.ts',
  '*** Move to: src/main.ts',
  '@@ iii.registerFunction(',
  "-  'demo::add',",
  "+  'demo::sum',",
  '   async (payload) => {',
  '*** Add File: src/lib/log.ts',
  '+export function log(msg: string): void {',
  "+  process.stdout.write(msg + '\\n')",
  '+}',
  '*** Delete File: src/adapters.ts',
  '*** End Patch',
  '',
].join('\n')

describe('parsePatch', () => {
  it('splits a full patch into per-file hunks in patch order', () => {
    const hunks = parsePatch(FULL_PATCH)
    expect(hunks).toHaveLength(3)
    expect(hunks[0]?.kind).toBe('update')
    expect(hunks[1]?.kind).toBe('add')
    expect(hunks[2]).toEqual({ kind: 'delete', path: 'src/adapters.ts' })
  })

  it('strips the + prefix from Add File bodies and keeps a trailing newline', () => {
    const hunks = parsePatch(
      [
        '*** Begin Patch',
        '*** Add File: notes.md',
        '+# notes',
        '+',
        '+done',
        '*** End Patch',
      ].join('\n'),
    )
    expect(hunks[0]).toEqual({
      kind: 'add',
      path: 'notes.md',
      content: '# notes\n\ndone\n',
    })
  })

  it('builds update sides from context, - and + lines', () => {
    const hunk = parsePatch(FULL_PATCH)[0]
    if (hunk?.kind !== 'update') throw new Error('expected update hunk')
    expect(hunk.path).toBe('src/index.ts')
    expect(hunk.moveTo).toBe('src/main.ts')
    // @@ locator + context lines appear on BOTH sides; -/+ on one each.
    expect(hunk.oldText).toBe(
      "iii.registerFunction(\n  'demo::add',\n  async (payload) => {",
    )
    expect(hunk.newText).toBe(
      "iii.registerFunction(\n  'demo::sum',\n  async (payload) => {",
    )
  })

  it('skips bare @@ separators but keeps @@ locator text as shared context', () => {
    const hunk = parsePatch(
      [
        '*** Update File: a.ts',
        '@@',
        '-old()',
        '+new()',
        '@@ function tail() {',
        ' unchanged',
      ].join('\n'),
    )[0]
    if (hunk?.kind !== 'update') throw new Error('expected update hunk')
    expect(hunk.oldText).toBe('old()\nfunction tail() {\nunchanged')
    expect(hunk.newText).toBe('new()\nfunction tail() {\nunchanged')
  })

  it('keeps no moveTo when the Update hunk has no Move to line', () => {
    const hunk = parsePatch(['*** Update File: a.ts', '-x', '+y'].join('\n'))[0]
    if (hunk?.kind !== 'update') throw new Error('expected update hunk')
    expect(hunk.moveTo).toBeNull()
  })

  it('tolerates a missing Begin/End envelope', () => {
    const hunks = parsePatch('*** Delete File: gone.rs')
    expect(hunks).toEqual([{ kind: 'delete', path: 'gone.rs' }])
  })

  it('skips unknown *** markers without ending the parse', () => {
    const hunks = parsePatch(
      [
        '*** Update File: a.ts',
        '+added',
        '*** End of File',
        '*** Delete File: b.ts',
      ].join('\n'),
    )
    expect(hunks).toHaveLength(2)
    expect(hunks[1]?.kind).toBe('delete')
  })

  it('returns [] for text that is not a patch', () => {
    expect(parsePatch('just some prose\nwith lines\n')).toEqual([])
    expect(parsePatch('')).toEqual([])
  })
})

describe('derivedKind', () => {
  it('maps hunk kinds to result kinds, moved only with a Move to', () => {
    expect(derivedKind({ kind: 'add', path: 'a', content: '' })).toBe('added')
    expect(derivedKind({ kind: 'delete', path: 'a' })).toBe('deleted')
    expect(
      derivedKind({
        kind: 'update',
        path: 'a',
        moveTo: null,
        oldText: '',
        newText: '',
      }),
    ).toBe('modified')
    expect(
      derivedKind({
        kind: 'update',
        path: 'a',
        moveTo: 'b',
        oldText: '',
        newText: '',
      }),
    ).toBe('moved')
  })
})
