import { describe, expect, it } from 'vitest'
import { composerModelFor } from './composer-model'

const sonnet = 'claude-code::claude-code/claude-sonnet-5'
const opus = 'anthropic::claude-opus-5'

describe('composerModelFor', () => {
  it("answers the conversation's own pick", () => {
    expect(composerModelFor([{ id: 'a', model: opus }], 'a', sonnet)).toBe(opus)
  })

  it('answers the model a new chat starts on when nothing was picked in this browser', () => {
    expect(composerModelFor([{ id: 'a', model: null }], 'a', sonnet)).toBe(sonnet)
    expect(composerModelFor([{ id: 'a', model: '  ' }], 'a', sonnet)).toBe(sonnet)
  })

  it('answers the starting model for a session it has not loaded and with no chat open', () => {
    expect(composerModelFor([], 's_new_schedule', sonnet)).toBe(sonnet)
    expect(composerModelFor([], null, sonnet)).toBe(sonnet)
  })

  it('answers null only when no model is known at all', () => {
    expect(composerModelFor([], null, null)).toBeNull()
    expect(composerModelFor([{ id: 'a' }], 'a', undefined)).toBeNull()
  })
})
