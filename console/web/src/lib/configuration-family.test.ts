import { describe, expect, it } from 'vitest'
import {
  configurationFormFamily,
  resolveConfigurationFamily,
} from './configuration-family'

describe('configuration form families', () => {
  it('uses ui_form metadata for a dynamically named worker', () => {
    expect(
      configurationFormFamily({
        id: 'browser-team-a',
        metadata: { ui_form: 'browser' },
      }),
    ).toBe('browser')
  })

  it('falls back to the exact id for legacy and malformed metadata', () => {
    expect(configurationFormFamily({ id: 'browser' })).toBe('browser')
    expect(
      configurationFormFamily({
        id: 'browser-team-a',
        metadata: { ui_form: '  ' },
      }),
    ).toBe('browser-team-a')
  })

  it('resolves an exact entry when it is the only family instance', () => {
    expect(
      resolveConfigurationFamily('browser', [
        { id: 'browser', metadata: { ui_form: 'browser' } },
      ]),
    ).toEqual({ kind: 'resolved', id: 'browser' })
  })

  it('resolves one named instance and refuses to choose among several', () => {
    expect(
      resolveConfigurationFamily('browser', [
        { id: 'browser-team-a', metadata: { ui_form: 'browser' } },
      ]),
    ).toEqual({ kind: 'resolved', id: 'browser-team-a' })

    expect(
      resolveConfigurationFamily('browser', [
        { id: 'browser', metadata: { ui_form: 'browser' } },
        { id: 'browser-team-a', metadata: { ui_form: 'browser' } },
      ]),
    ).toEqual({
      kind: 'ambiguous',
      ids: ['browser', 'browser-team-a'],
    })
  })

  it('reports a missing family without inventing a route', () => {
    expect(resolveConfigurationFamily('browser', [])).toEqual({
      kind: 'missing',
    })
  })
})
