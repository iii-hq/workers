import { describe, expect, it } from 'vitest'
import { type PaletteEntry, scoreEntry } from '@/lib/palette/sources'
import { SETUP_WIZARD_KEYWORDS } from './open'

const setupRow: PaletteEntry = {
  id: 'action:setup',
  kind: 'action',
  title: 'Set up models and keys',
  detail: 'Connect a provider, store API keys and set up Judge',
  keywords: [...SETUP_WIZARD_KEYWORDS],
  run: () => {},
}

describe('the setup wizard in the palette', () => {
  // QA 2026-11-16 F15: only "set up" (two words) found the wizard; "setup",
  // "wizard" and "tour" said No results.
  it.each(['setup', 'wizard', 'tour', 'onboarding', 'set up'])(
    'is found by "%s"',
    (query) => {
      expect(scoreEntry(setupRow, query)).toBeGreaterThan(0)
    },
  )
})
