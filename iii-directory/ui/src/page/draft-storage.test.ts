import { describe, expect, it } from 'vitest'
import { draftAction, parseStoredDraft } from './draft-storage'

describe('parseStoredDraft', () => {
  it('round-trips a written draft', () => {
    const draft = { creating: true, key: null, content: '# hi' }
    expect(parseStoredDraft(JSON.stringify(draft))).toEqual(draft)
  })

  it('treats absent, malformed, or contentless entries as no draft', () => {
    expect(parseStoredDraft(null)).toBeNull()
    expect(parseStoredDraft('')).toBeNull()
    expect(parseStoredDraft('not json')).toBeNull()
    expect(parseStoredDraft('{"creating":true}')).toBeNull()
    expect(parseStoredDraft('{"content":42}')).toBeNull()
  })

  it('normalises a partial entry rather than trusting it', () => {
    expect(parseStoredDraft('{"content":"x","key":7}')).toEqual({
      creating: false,
      key: null,
      content: 'x',
    })
  })
})

describe('draftAction', () => {
  it('persists a new entry from the first keystroke', () => {
    // startCreate seeds loaded.content = '' so the scaffold already counts
    // as unsaved work.
    const action = draftAction({
      creating: true,
      selected: null,
      draft: '# new skill',
      loadedContent: '',
    })
    expect(action).toEqual({
      kind: 'write',
      draft: { creating: true, key: null, content: '# new skill' },
    })
  })

  it('persists an edited existing entry and clears once it matches disk', () => {
    const base = { creating: false, selected: 'coder/index', loadedContent: 'a' }
    expect(draftAction({ ...base, draft: 'a+' })).toEqual({
      kind: 'write',
      draft: { creating: false, key: 'coder/index', content: 'a+' },
    })
    expect(draftAction({ ...base, draft: 'a' })).toEqual({ kind: 'clear' })
  })

  it('keeps storage while a baseline load is in flight', () => {
    // The restore path mounts with the draft but no baseline yet; clearing
    // here would destroy the work we just restored.
    expect(
      draftAction({
        creating: false,
        selected: 'coder/index',
        draft: 'restored',
        loadedContent: null,
      }),
    ).toEqual({ kind: 'keep' })
  })

  it('clears when nothing is open', () => {
    expect(
      draftAction({
        creating: false,
        selected: null,
        draft: '',
        loadedContent: null,
      }),
    ).toEqual({ kind: 'clear' })
  })
})
