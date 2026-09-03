import { describe, expect, it } from 'vitest'
import type { RegisteredComposerAction } from './ui-slots'
import { getExtComposerActions, registerExtComposerAction } from './ui-slots'

function action(id: string, path: string): RegisteredComposerAction {
  return { id, path, scope: path.split('/')[0], render: () => null }
}

describe('composer action slot', () => {
  it('dedupes by id and restores a shadowed registration', () => {
    const offVoice = registerExtComposerAction(
      action('dictate', 'voice/page.js'),
    )
    const offOther = registerExtComposerAction(
      action('dictate', 'other/page.js'),
    )

    expect(getExtComposerActions().map((item) => item.path)).toEqual([
      'other/page.js',
    ])

    offOther()
    expect(getExtComposerActions().map((item) => item.path)).toEqual([
      'voice/page.js',
    ])

    offVoice()
    offVoice()
    expect(getExtComposerActions()).toEqual([])
  })

  it('preserves registration order across distinct ids', () => {
    const offA = registerExtComposerAction(action('dictate', 'voice/page.js'))
    const offB = registerExtComposerAction(action('snippet', 'snip/page.js'))

    expect(getExtComposerActions().map((item) => item.id)).toEqual([
      'dictate',
      'snippet',
    ])

    offA()
    offB()
  })
})
