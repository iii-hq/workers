import { describe, expect, it } from 'vitest'
import { metadataFor, patchSessionMetadata } from '@/hooks/use-conversations'
import type { Conversation } from '@/types/chat'
import type { RegisteredComposerControl } from './ui-slots'
import { getExtComposerControls, registerExtComposerControl } from './ui-slots'

function control(id: string, path: string): RegisteredComposerControl {
  return { id, path, scope: path.split('/')[0], render: () => null }
}

describe('composer control slot', () => {
  it('dedupes by id and restores a shadowed registration', () => {
    const offJudge = registerExtComposerControl(
      control('judge-provider', 'judge/page.js'),
    )
    const offOther = registerExtComposerControl(
      control('judge-provider', 'other/page.js'),
    )
    expect(getExtComposerControls().map((item) => item.path)).toEqual([
      'other/page.js',
    ])
    offOther()
    expect(getExtComposerControls().map((item) => item.path)).toEqual([
      'judge/page.js',
    ])
    offJudge()
    expect(getExtComposerControls()).toEqual([])
  })
})

describe('session metadata written by a composer control', () => {
  it('merges keys, removes undefined ones, and survives the console rewrite', () => {
    const stored = {
      judge_provider: 'typesafe',
      parent_session_id: 's1',
      model: 'm',
    }
    expect(patchSessionMetadata(stored, { judge_provider: 'semif' })).toEqual({
      ...stored,
      judge_provider: 'semif',
    })
    expect(patchSessionMetadata(stored, { judge_provider: undefined })).toEqual(
      {
        parent_session_id: 's1',
        model: 'm',
      },
    )
    // The console rebuilds its own keys on every write, so a control cannot
    // overwrite them, while its key rides along untouched.
    const conversation = {
      model: 'claude-opus-5-5',
      sessionMetadata: patchSessionMetadata(stored, {
        judge_provider: 'semif',
        model: 'hijack',
      }),
    } as unknown as Conversation
    const written = metadataFor(conversation)
    expect(written.judge_provider).toBe('semif')
    expect(written.model).toBe('claude-opus-5-5')
  })
})
