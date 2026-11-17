import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import { saveRule } from '../src/page/memory-data'
import { baseAfterSave, mergeRule } from '../src/page/rule-merge'

interface SetRequest {
  bank: string
  name: string
  content: string
  expected?: string
}

/** The two rule functions of the memory worker over an in-memory bank:
 * `rule::set` refuses a stale `expected` the way the worker does, and
 * `store` stands in for what the worker does to a write (redact, cap). */
function fakeWorker(disk: Map<string, string>, store = (content: string) => content) {
  const sets: SetRequest[] = []
  const call = async (fn: string, req: SetRequest) => {
    if (fn === 'memory::rule::list') {
      return { rules: [...disk].map(([name, content]) => ({ name, content, updated_at: 0 })) }
    }
    sets.push(req)
    if (req.expected !== undefined && req.expected !== (disk.get(req.name) ?? '')) {
      throw new Error(`invalid_input: rule_changed: rule \`${req.name}\` changed since \`expected\` was read`)
    }
    if (req.content === '') disk.delete(req.name)
    else disk.set(req.name, store(req.content))
    return { ok: true, exists: req.content !== '' }
  }
  return { host: { iii: { call } } as unknown as Host, sets }
}

const base = '# learned\n- a\n- b\n- c\n'
// The user removed `- b` and added a line.
const mine = '# learned\n- a\n- c\n- mine\n'
// Meanwhile the worker demoted `- a` and learned `- d`.
const theirs = '# learned\n- b\n- c\n- d\n'

describe('mergeRule', () => {
  const merged = mergeRule(base, mine, theirs)

  it('keeps a line the machine appended', () => {
    expect(merged).toBe('# learned\n- c\n- mine\n- d\n')
  })

  it('drops a line demoted on disk', () => {
    expect(merged).not.toContain('- a\n')
  })

  it('keeps a line the user deleted deleted', () => {
    expect(merged).not.toContain('- b\n')
  })

  it('moves a line restated on disk to the end, so the cap does not take it first', () => {
    // The user fixed `- c`; meanwhile the user restated `- a` in chat.
    expect(mergeRule(base, '# learned\n- a\n- b\n- c2\n', '# learned\n- b\n- c\n- a\n')).toBe(
      '# learned\n- b\n- c2\n- a\n',
    )
  })
})

describe('saveRule', () => {
  it('merges once the worker refuses a stale write and returns the re-listed rule', async () => {
    const disk = new Map([['learned', theirs]])
    // The worker caps the rule on write, so what it stores is not what was sent.
    const { host, sets } = fakeWorker(disk, (content) => content.replace('- c\n', ''))

    const saved = await saveRule(host, 'main', 'learned', mine, base)

    expect(sets).toHaveLength(2)
    expect(sets[0]).toEqual({ bank: 'main', name: 'learned', content: mine, expected: base })
    expect(sets[1]).toEqual({ bank: 'main', name: 'learned', content: mergeRule(base, mine, theirs), expected: theirs })
    expect(saved).toBe('# learned\n- mine\n- d\n')
  })

  it('never merges a create', async () => {
    const disk = new Map([['style', '# style\n- terse\n']])
    const { host, sets } = fakeWorker(disk)

    await expect(saveRule(host, 'main', 'style', '# style\n', '')).rejects.toThrow('rule_changed')
    expect(sets).toHaveLength(1)
    expect(disk.get('style')).toBe('# style\n- terse\n')
  })

  it('never merges a delete', async () => {
    const disk = new Map([['learned', theirs]])
    const { host, sets } = fakeWorker(disk)

    await expect(saveRule(host, 'main', 'learned', '', base)).rejects.toThrow('rule_changed')
    expect(sets).toHaveLength(1)
    expect(disk.get('learned')).toBe(theirs)
  })

  it('never sends a merge that empties the rule', async () => {
    // This tab removed `y`; another removed `x` first.
    const disk = new Map([['notes', 'y\n']])
    const { host, sets } = fakeWorker(disk)

    await expect(saveRule(host, 'main', 'notes', 'x\n', 'x\ny\n')).rejects.toThrow('rule_changed')
    expect(sets).toHaveLength(1)
    expect(disk.get('notes')).toBe('y\n')
  })

  it('never turns a merge into a create once the rule is gone', async () => {
    const disk = new Map<string, string>()
    const { host, sets } = fakeWorker(disk)

    await expect(saveRule(host, 'main', 'style', '# style\n- A\n- terse\n', '# style\n- A\n')).rejects.toThrow(
      'rule_changed',
    )
    expect(sets).toHaveLength(1)
    expect(disk.has('style')).toBe(false)
  })
})

describe('baseAfterSave', () => {
  it('keeps the line a save merged in when the user typed during that save', async () => {
    // Extraction appended `- c` after the editor loaded the rule.
    const loaded = '# learned\n- a\n- b\n'
    const disk = new Map([['learned', `${loaded}- c\n`]])
    const { host } = fakeWorker(disk)
    const sent = `${loaded}- mine\n`

    const saved = await saveRule(host, 'main', 'learned', sent, loaded)
    expect(saved).toBe(`${sent}- c\n`)
    expect(baseAfterSave(sent, sent, saved)).toBe(saved)

    // `- more` was typed while that save ran, so the editor never showed `- c`.
    const now = `${sent}- more\n`
    await saveRule(host, 'main', 'learned', now, baseAfterSave(sent, now, saved))
    expect(disk.get('learned')).toBe(`${now}- c\n`)
  })
})
