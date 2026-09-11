import type { LexicalEditor } from 'lexical'
import { describe, expect, it } from 'vitest'
import { slashTriggerFn } from './SlashCommandsPlugin'

const editor = {} as LexicalEditor

describe('slashTriggerFn', () => {
  it('opens on a bare / and follows the slug', () => {
    expect(slashTriggerFn('/', editor)).toEqual({
      leadOffset: 0,
      matchingString: '',
      replaceableString: '/',
    })
    expect(slashTriggerFn('/compact', editor)?.matchingString).toBe('compact')
    expect(slashTriggerFn('/rev', editor)?.matchingString).toBe('rev')
    expect(slashTriggerFn('/skill:coder/index', editor)?.matchingString).toBe(
      'skill:coder/index',
    )
  })

  it('fires mid-sentence after whitespace or a paren, never inside a word', () => {
    expect(slashTriggerFn('please /rev', editor)).toEqual({
      leadOffset: 7,
      matchingString: 'rev',
      replaceableString: '/rev',
    })
    expect(slashTriggerFn('(/comp', editor)?.leadOffset).toBe(1)
    expect(slashTriggerFn('either/or', editor)).toBeNull()
  })

  it('stays out of a typed path and closes once the slug is done', () => {
    expect(slashTriggerFn('/home/x', editor)).toBeNull()
    expect(slashTriggerFn('/compact ', editor)).toBeNull()
    expect(slashTriggerFn('a / b', editor)).toBeNull()
  })
})
