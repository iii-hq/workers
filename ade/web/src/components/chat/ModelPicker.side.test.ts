// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { pickMenuSide } from './ModelPicker'

function triggerAt(top: number, height = 32): HTMLElement {
  const element = document.createElement('button')
  element.getBoundingClientRect = () =>
    ({
      top,
      bottom: top + height,
      left: 0,
      right: 0,
      width: 0,
      height,
    }) as DOMRect
  return element
}

describe('pickMenuSide', () => {
  it('opens above the composer trigger', () => {
    window.innerHeight = 900
    expect(pickMenuSide(triggerAt(700))).toBe('top')
  })

  it('opens below a trigger near the top with more room below', () => {
    window.innerHeight = 900
    expect(pickMenuSide(triggerAt(80))).toBe('bottom')
  })

  it('keeps above when it is tight both ways but above has more', () => {
    window.innerHeight = 500
    expect(pickMenuSide(triggerAt(300))).toBe('top')
  })
})
