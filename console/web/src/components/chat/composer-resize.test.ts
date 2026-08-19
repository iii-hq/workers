import { describe, expect, it } from 'vitest'
import { classifyComposerResize } from './composer-resize'

describe('classifyComposerResize', () => {
  it('hydrates the measured height without an entrance animation', () => {
    expect(classifyComposerResize(null, { width: 480, height: 56 })).toBe(
      'initial',
    )
  })

  it('animates a content-driven height change', () => {
    expect(
      classifyComposerResize(
        { width: 480, height: 56 },
        { width: 480, height: 83 },
      ),
    ).toBe('content')
  })

  it('keeps width-driven reflow attached to direct resize', () => {
    expect(
      classifyComposerResize(
        { width: 480, height: 56 },
        { width: 420, height: 83 },
      ),
    ).toBe('container')
  })

  it('ignores subpixel observer noise', () => {
    expect(
      classifyComposerResize(
        { width: 480, height: 56 },
        { width: 480.25, height: 56.25 },
      ),
    ).toBe('unchanged')
  })
})
