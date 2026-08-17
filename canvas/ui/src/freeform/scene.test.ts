/**
 * Pure-helper tests for the freeform surface: scene-source tolerance,
 * appState sanitization, the mermaid-fallback detector, and export filenames.
 * No DOM, no vendor code — everything under test lives in ./scene.ts.
 */

import { describe, expect, it } from 'vitest'

import {
  exportFilename,
  isImageFallback,
  parseSceneSource,
  sanitizeAppState,
} from './scene'

const BLANK = { elements: [], appState: {}, files: {} }

describe('parseSceneSource', () => {
  it('opens a blank scene for empty and whitespace source', () => {
    expect(parseSceneSource('')).toEqual(BLANK)
    expect(parseSceneSource('   \n\t')).toEqual(BLANK)
  })

  it('opens a blank scene for invalid JSON', () => {
    expect(parseSceneSource('graph TD; a-->b')).toEqual(BLANK)
    expect(parseSceneSource('{"elements": [')).toEqual(BLANK)
  })

  it('opens a blank scene for non-object JSON', () => {
    expect(parseSceneSource('null')).toEqual(BLANK)
    expect(parseSceneSource('42')).toEqual(BLANK)
    expect(parseSceneSource('"scene"')).toEqual(BLANK)
    expect(parseSceneSource('[1, 2]')).toEqual(BLANK)
  })

  it('keeps elements, appState, and files from a valid scene', () => {
    const scene = {
      type: 'excalidraw',
      version: 2,
      elements: [{ id: 'e1', type: 'rectangle' }],
      appState: { viewBackgroundColor: '#ffffff', zenModeEnabled: true },
      files: { f1: { mimeType: 'image/png' } },
    }
    expect(parseSceneSource(JSON.stringify(scene))).toEqual({
      elements: [{ id: 'e1', type: 'rectangle' }],
      appState: { viewBackgroundColor: '#ffffff', zenModeEnabled: true },
      files: { f1: { mimeType: 'image/png' } },
    })
  })

  it('defaults each malformed section independently', () => {
    expect(
      parseSceneSource(
        JSON.stringify({ elements: 'nope', appState: [], files: 7 }),
      ),
    ).toEqual(BLANK)
    expect(
      parseSceneSource(JSON.stringify({ elements: [{ id: 'e1' }] })),
    ).toEqual({ elements: [{ id: 'e1' }], appState: {}, files: {} })
  })

  it('drops the appState keys that break a fresh mount', () => {
    const parsed = parseSceneSource(
      JSON.stringify({
        elements: [],
        appState: {
          collaborators: {},
          width: 1440,
          height: 900,
          offsetTop: 12,
          offsetLeft: 8,
          gridSize: 20,
        },
      }),
    )
    expect(parsed.appState).toEqual({ gridSize: 20 })
  })
})

describe('sanitizeAppState', () => {
  it('is a no-op on already-clean state', () => {
    expect(sanitizeAppState({ theme: 'dark', zoom: { value: 1 } })).toEqual({
      theme: 'dark',
      zoom: { value: 1 },
    })
  })

  it('never mutates its input', () => {
    const input = { collaborators: {}, gridSize: 20 }
    sanitizeAppState(input)
    expect(input).toEqual({ collaborators: {}, gridSize: 20 })
  })
})

describe('isImageFallback', () => {
  it('flags the single-image-element fallback shape', () => {
    expect(isImageFallback([{ type: 'image' }])).toBe(true)
  })

  it('does not flag native conversions or empty output', () => {
    expect(isImageFallback([])).toBe(false)
    expect(isImageFallback([{ type: 'rectangle' }, { type: 'arrow' }])).toBe(
      false,
    )
    expect(isImageFallback([{ type: 'image' }, { type: 'text' }])).toBe(false)
  })
})

describe('exportFilename', () => {
  it('slugifies the record name', () => {
    expect(exportFilename('Payment Flow (v2)', 'png')).toBe(
      'payment-flow-v2.png',
    )
    expect(exportFilename('Sales — Q3 / final', 'svg')).toBe(
      'sales-q3-final.svg',
    )
  })

  it('keeps already-safe names', () => {
    expect(exportFilename('deploy_graph.v1', 'svg')).toBe('deploy_graph.v1.svg')
  })

  it('falls back to canvas when nothing safe remains', () => {
    expect(exportFilename('', 'png')).toBe('canvas.png')
    expect(exportFilename('   ', 'png')).toBe('canvas.png')
    expect(exportFilename('🎨🎨', 'svg')).toBe('canvas.svg')
  })

  it('trims leading/trailing separators and caps the stem at 64 chars', () => {
    expect(exportFilename('--hello--', 'png')).toBe('hello.png')
    expect(exportFilename('...dots...', 'png')).toBe('dots.png')
    const long = 'x'.repeat(200)
    expect(exportFilename(long, 'png')).toBe(`${'x'.repeat(64)}.png`)
  })
})
