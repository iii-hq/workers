import type { LexicalEditor } from 'lexical'
import { describe, expect, it } from 'vitest'
import { hashTriggerFn } from './FileMentionsPlugin'
import { atTriggerFn } from './MentionsPlugin'

const editor = {} as LexicalEditor

describe('atTriggerFn', () => {
  it('opens on a bare @ and keeps ids and paths whole', () => {
    expect(atTriggerFn('@', editor)).toEqual({
      leadOffset: 0,
      matchingString: '',
      replaceableString: '@',
    })
    expect(atTriggerFn('run @shell::exec', editor)).toEqual({
      leadOffset: 4,
      matchingString: 'shell::exec',
      replaceableString: '@shell::exec',
    })
    expect(
      atTriggerFn('see @src/lib/a-b.test.ts', editor)?.matchingString,
    ).toBe('src/lib/a-b.test.ts')
    expect(atTriggerFn('(@x', editor)?.leadOffset).toBe(1)
  })

  it('stays closed inside words and after a space', () => {
    expect(atTriggerFn('mail me@example.com', editor)).toBeNull()
    expect(atTriggerFn('@shell ', editor)).toBeNull()
    expect(atTriggerFn('@a)', editor)).toBeNull()
  })

  it('keeps a scoped @worker: search open across spaces', () => {
    expect(atTriggerFn('ask @kanban:fix login', editor)).toEqual({
      leadOffset: 4,
      matchingString: 'kanban:fix login',
      replaceableString: '@kanban:fix login',
    })
    expect(atTriggerFn('@kanban:', editor)?.matchingString).toBe('kanban:')
    // A function id is not a scope: it stays one space-free word.
    expect(atTriggerFn('@kanban::ticket::get', editor)?.matchingString).toBe(
      'kanban::ticket::get',
    )
    expect(atTriggerFn('@kanban::ticket get', editor)).toBeNull()
  })
})

describe('hashTriggerFn', () => {
  it('needs one character so headings never flash the menu', () => {
    expect(hashTriggerFn('#', editor)).toBeNull()
    expect(hashTriggerFn('# heading', editor)).toBeNull()
    expect(hashTriggerFn('see #src/a.ts', editor)).toEqual({
      leadOffset: 4,
      matchingString: 'src/a.ts',
      replaceableString: '#src/a.ts',
    })
  })
})
