import { describe, expect, it } from 'vitest'
import type { RegisteredSessionTurnSummary } from './ui-slots'
import {
  getExtSessionTurnSummaries,
  registerExtSessionTurnSummary,
} from './ui-slots'

function summary(id: string, path: string): RegisteredSessionTurnSummary {
  return { id, path, scope: path.split('/')[0], render: () => null }
}

describe('session turn-summary slot', () => {
  it('dedupes by id and restores a shadowed registration', () => {
    const offShell = registerExtSessionTurnSummary(
      summary('changed-files', 'shell/page.js'),
    )
    const offOther = registerExtSessionTurnSummary(
      summary('changed-files', 'other/page.js'),
    )

    expect(getExtSessionTurnSummaries().map((item) => item.path)).toEqual([
      'other/page.js',
    ])

    offOther()
    expect(getExtSessionTurnSummaries().map((item) => item.path)).toEqual([
      'shell/page.js',
    ])

    offShell()
    offShell()
    expect(getExtSessionTurnSummaries()).toEqual([])
  })

  it('preserves registration order across distinct ids', () => {
    const offA = registerExtSessionTurnSummary(
      summary('changed-files', 'shell/page.js'),
    )
    const offB = registerExtSessionTurnSummary(summary('tests', 'test/page.js'))

    expect(getExtSessionTurnSummaries().map((item) => item.id)).toEqual([
      'changed-files',
      'tests',
    ])

    offA()
    offB()
  })
})
