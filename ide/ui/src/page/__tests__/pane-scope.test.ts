import { describe, expect, it } from 'vitest'
import { paneScopeToken, paneStateKey } from '../pane-scope'

describe('pane-scope', () => {
  it('keys state by the pane when the console names one', () => {
    expect(paneStateKey('tab-1', 'tab-1:pane:1')).toBe('tab-1:pane:1')
    expect(paneStateKey('tab-1', 'pane-abc')).toBe('pane-abc')
  })

  it('falls back to the workspace tab on older consoles', () => {
    expect(paneStateKey('tab-1', undefined)).toBe('tab-1')
    expect(paneStateKey('tab-1', '')).toBe('tab-1')
    expect(paneStateKey('', '')).toBe('')
  })

  it('turns a key into one function-id segment', () => {
    expect(paneScopeToken('tab-dir-123:pane:0')).toBe('tab-dir-123-pane-0')
    expect(paneScopeToken('a b::c/d')).toBe('a-b-c-d')
    expect(paneScopeToken('plain_key-1')).toBe('plain_key-1')
  })
})
