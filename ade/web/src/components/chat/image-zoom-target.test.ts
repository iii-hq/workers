import { describe, expect, it } from 'vitest'
import { imageZoomTarget } from './image-zoom-target'

function img(attrs: {
  width?: number
  height?: number
  src?: string
  alt?: string
  inControl?: boolean
}): EventTarget {
  return {
    tagName: 'IMG',
    src: attrs.src ?? 'data:image/png;base64,AAAA',
    alt: attrs.alt ?? '',
    closest: (selector: string) =>
      attrs.inControl && selector.includes('[role="button"]') ? {} : null,
    getBoundingClientRect: () => ({
      width: attrs.width ?? 200,
      height: attrs.height ?? 120,
    }),
  } as unknown as EventTarget
}

describe('imageZoomTarget', () => {
  it('opens a plain content image, with its alt text', () => {
    expect(imageZoomTarget(img({ alt: 'capture of a page' }))).toEqual({
      src: 'data:image/png;base64,AAAA',
      alt: 'capture of a page',
    })
  })

  it('falls back to a generic alt', () => {
    expect(imageZoomTarget(img({}))?.alt).toBe('image')
  })

  it('ignores anything that is not an image', () => {
    expect(imageZoomTarget({ tagName: 'DIV' } as unknown as EventTarget)).toBeNull()
    expect(imageZoomTarget(null)).toBeNull()
  })

  it('leaves an image that sits inside a control alone', () => {
    expect(imageZoomTarget(img({ inControl: true }))).toBeNull()
  })

  it('leaves icons and avatars alone', () => {
    expect(imageZoomTarget(img({ width: 24, height: 24 }))).toBeNull()
    expect(imageZoomTarget(img({ width: 200, height: 16 }))).toBeNull()
  })

  it('needs a source', () => {
    expect(imageZoomTarget(img({ src: '' }))).toBeNull()
  })
})
