import { describe, expect, it } from 'vitest'
import {
  MIN_SHAPE_SIZE,
  addAnnotation,
  addShape,
  annotationFileName,
  annotationPinFileName,
  annotationsMarkdown,
  containedImageBox,
  moveAnnotation,
  noteAnnotation,
  removeAnnotation,
  resizeAnnotation,
  undoAnnotation,
} from './annotations'

describe('annotations', () => {
  it('draws shapes: start equal, resize clamps, undo drops the newest', () => {
    let list = addShape([], 'rect', 0.2, 0.3, '#e5484d')
    const rect = list[0]
    expect(rect).toMatchObject({
      kind: 'rect',
      x: 0.2,
      y: 0.3,
      x2: 0.2,
      y2: 0.3,
      color: '#e5484d',
    })
    list = resizeAnnotation(list, rect.id, 1.4, 0.9)
    expect(list[0]).toMatchObject({ x2: 1, y2: 0.9 })
    list = addShape(list, 'arrow', 0.5, 0.5)
    expect(list[1].kind).toBe('arrow')
    expect(undoAnnotation(list)).toHaveLength(1)
    expect(MIN_SHAPE_SIZE).toBeGreaterThan(0)
  })

  it('keeps pins as clamped fractions and numbers them by order', () => {
    let list = addAnnotation([], 0.25, 0.5)
    list = addAnnotation(list, 1.4, -0.2)
    expect(list.map((a) => [a.x, a.y])).toEqual([
      [0.25, 0.5],
      [1, 0],
    ])
    list = moveAnnotation(list, list[0].id, 0.3, 0.6)
    list = noteAnnotation(list, list[1].id, 'the button is cut off')
    expect(list[0]).toMatchObject({ x: 0.3, y: 0.6, note: '' })
    expect(list[1].note).toBe('the button is cut off')
    const removed = removeAnnotation(list, list[0].id)
    expect(removed).toHaveLength(1)
    expect(removed[0].note).toBe('the button is cut off')
  })

  it('finds the painted box of a contained image', () => {
    expect(
      containedImageBox(
        { width: 400, height: 400 },
        { width: 200, height: 100 },
      ),
    ).toEqual({ left: 0, top: 100, width: 400, height: 200 })
    expect(
      containedImageBox(
        { width: 100, height: 400 },
        { width: 200, height: 100 },
      ),
    ).toEqual({ left: 0, top: 175, width: 100, height: 50 })
    expect(
      containedImageBox({ width: 300, height: 200 }, { width: 0, height: 0 }),
    ).toEqual({ left: 0, top: 0, width: 300, height: 200 })
  })

  it('describes a set for the chat and names its file', () => {
    const set = {
      subject: 'https://example.com/pricing?plan=team',
      imageUrl: 'data:image/png;base64,',
      width: 1280,
      height: 720,
      annotations: [
        {
          id: 'a',
          x: 0.1,
          y: 0.1,
          note: 'logo is blurry',
          label: 'img.logo (ref e3)',
        },
        { id: 'b', x: 0.5, y: 0.5, note: '' },
        { id: 'c', x: 0.7, y: 0.7, note: '', label: 'a.cta "Learn more"' },
      ],
      capturedAt: Date.UTC(2026, 7, 21, 18, 0, 0),
    }
    expect(annotationsMarkdown(set)).toBe(
      [
        'Annotations on https://example.com/pricing?plan=team (3 pins)',
        '1. logo is blurry (img.logo (ref e3))',
        '2. (no note)',
        '3. a.cta "Learn more"',
      ].join('\n'),
    )
    expect(annotationFileName(set, 'png')).toBe(
      'annotations-example.com-pricing-plan-team-2026-08-21T18-00-00-000Z.png',
    )
    expect(
      set.annotations.map((a, i) => annotationPinFileName(a, i, 'png')),
    ).toEqual([
      'pin-1-logo-is-blurry.png',
      'pin-2.png',
      'pin-3-a-cta-Learn-more.png',
    ])
  })
})
