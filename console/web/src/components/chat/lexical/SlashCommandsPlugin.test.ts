import type { LexicalEditor } from 'lexical'
import { describe, expect, it } from 'vitest'
import { slashTriggerFn } from './SlashCommandsPlugin'

const editor = {} as LexicalEditor

describe('slashTriggerFn', () => {
  it('triggers only built-in prefixes and skill invocations', () => {
    expect(slashTriggerFn('/review-pr', editor)).toBeNull()

    for (const text of [
      '/',
      '/c',
      '/co',
      '/com',
      '/comp',
      '/compa',
      '/compac',
      '/compact',
      '/skill:',
      '/skill:coder/index',
    ]) {
      expect(slashTriggerFn(text, editor)).not.toBeNull()
    }
  })
})
