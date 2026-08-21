import { describe, expect, it } from 'vitest'
import {
  centred,
  clampOffset,
  clampScale,
  dataUrlBytes,
  fitScale,
  MAX_PREVIEW_BYTES,
  MAX_SCALE,
  MIN_SCALE,
  panBy,
  pinchGeometry,
  steppedScale,
  zoomAbout,
  zoomPercent,
} from './image-viewer-math'

const stage = { width: 1000, height: 600 }

describe('fitScale', () => {
  it('contains a large image on its longer side', () => {
    expect(fitScale({ width: 4000, height: 1200 }, stage)).toBeCloseTo(0.25)
    expect(fitScale({ width: 1000, height: 2400 }, stage)).toBeCloseTo(0.25)
  })

  it('never upscales a small image past its natural size', () => {
    expect(fitScale({ width: 200, height: 100 }, stage)).toBe(1)
  })

  it('falls back to 1 for a degenerate stage or image', () => {
    expect(fitScale({ width: 0, height: 0 }, stage)).toBe(1)
    expect(fitScale({ width: 10, height: 10 }, { width: 0, height: 0 })).toBe(1)
  })
})

describe('clampScale', () => {
  it('stays inside the global bounds and never below fit', () => {
    expect(clampScale(100, 0.5)).toBe(MAX_SCALE)
    expect(clampScale(0.0001, 0.5)).toBe(MIN_SCALE)
    expect(clampScale(0.01, 0.02)).toBe(0.02)
    expect(clampScale(Number.NaN, 0.3)).toBe(0.3)
  })
})

describe('clampOffset', () => {
  it('keeps a small image centred', () => {
    const view = clampOffset(
      { scale: 1, x: 300, y: -200 },
      { width: 200, height: 100 },
      stage,
    )
    expect(view).toEqual({ scale: 1, x: 0, y: 0 })
  })

  it('lets a large image pan only until its edge meets the stage edge', () => {
    const image = { width: 2000, height: 600 }
    const view = clampOffset({ scale: 1, x: 900, y: 50 }, image, stage)
    expect(view.x).toBe(500)
    expect(view.y).toBe(0)
    expect(clampOffset({ scale: 1, x: -900, y: 0 }, image, stage).x).toBe(-500)
  })
})

describe('zoomAbout', () => {
  const image = { width: 2000, height: 2000 }

  it('keeps the image point under the focus in place', () => {
    const before = centred(0.3)
    const focus = { x: 100, y: -50 }
    const after = zoomAbout(before, 0.6, focus, image, stage, 0.3)
    // The point under focus maps to image-space (focus - x) / scale; it must
    // be the same image point before and after the zoom.
    const imageBefore = {
      x: (focus.x - before.x) / before.scale,
      y: (focus.y - before.y) / before.scale,
    }
    const imageAfter = {
      x: (focus.x - after.x) / after.scale,
      y: (focus.y - after.y) / after.scale,
    }
    expect(imageAfter.x).toBeCloseTo(imageBefore.x)
    expect(imageAfter.y).toBeCloseTo(imageBefore.y)
  })

  it('zooming out to fit recentres', () => {
    const zoomed = zoomAbout(
      centred(0.3),
      2,
      { x: 300, y: 200 },
      image,
      stage,
      0.3,
    )
    expect(zoomed.x).not.toBe(0)
    const back = zoomAbout(zoomed, 0.3, { x: 0, y: 0 }, image, stage, 0.3)
    expect(back).toEqual({ scale: 0.3, x: 0, y: 0 })
  })
})

describe('panBy', () => {
  it('moves and clamps', () => {
    const image = { width: 2000, height: 2000 }
    const view = panBy(centred(1), -10_000, 25, image, stage)
    expect(view.x).toBe(-500)
    expect(view.y).toBe(25)
  })
})

describe('steppedScale', () => {
  it('walks the ladder in both directions', () => {
    expect(steppedScale(1, 1)).toBe(1.25)
    expect(steppedScale(1, -1)).toBe(0.75)
    expect(steppedScale(0.3, 1)).toBe(0.33)
    expect(steppedScale(0.3, -1)).toBe(0.25)
  })

  it('keeps going past the ladder ends without leaving the bounds', () => {
    expect(steppedScale(32, 1)).toBe(MAX_SCALE)
    expect(steppedScale(0.05, -1)).toBe(MIN_SCALE)
  })
})

describe('pinchGeometry', () => {
  it('needs two pointers and reports distance and midpoint', () => {
    expect(pinchGeometry([{ id: 1, x: 0, y: 0 }])).toBeNull()
    expect(
      pinchGeometry([
        { id: 1, x: 0, y: 0 },
        { id: 2, x: 30, y: 40 },
      ]),
    ).toEqual({ distance: 50, midpoint: { x: 15, y: 20 } })
  })
})

describe('dataUrlBytes', () => {
  it('sizes base64 payloads and ignores other sources', () => {
    expect(dataUrlBytes('data:image/png;base64,AAAA')).toBe(3)
    expect(dataUrlBytes('data:image/png;base64,AAA=')).toBe(2)
    expect(dataUrlBytes('data:text/plain,hello')).toBe(5)
    expect(dataUrlBytes('blob:http://x/y')).toBeNull()
    expect(MAX_PREVIEW_BYTES).toBeGreaterThan(32 * 1024 * 1024)
  })
})

describe('zoomPercent', () => {
  it('rounds to whole percent', () => {
    expect(zoomPercent(0.333)).toBe('33%')
    expect(zoomPercent(2)).toBe('200%')
  })
})
