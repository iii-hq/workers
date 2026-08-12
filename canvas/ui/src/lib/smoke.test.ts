/**
 * Smoke test for the injected UI's pure surface: the wire types, the envelope
 * unwrap, and the chat renderer's claim set. Everything imported here is
 * type-only against @iii-dev/console-ui (its runtime entry throws by design —
 * it must stay external and value-imports belong only in bundled assets).
 */

import { describe, expect, it } from 'vitest'

import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'
import {
  HANDLED,
  createCanvasTriggerRenderer,
} from '../function-trigger-message'
import { CANVAS_FUNCTION_IDS, unwrapEnvelope } from './types'

const host = undefined as unknown as Host

function message(functionId: string): FunctionTriggerMessage {
  return {
    id: 'm1',
    role: 'function-trigger',
    functionId,
    input: {},
    output: {},
    createdAt: 0,
  }
}

describe('canvas trigger renderer', () => {
  it('claims exactly the seven public canvas functions', () => {
    expect(CANVAS_FUNCTION_IDS).toHaveLength(7)
    expect([...HANDLED].sort()).toEqual([...CANVAS_FUNCTION_IDS].sort())

    const renderer = createCanvasTriggerRenderer(host)
    for (const id of CANVAS_FUNCTION_IDS) {
      expect(renderer.isMatch(id)).toBe(true)
    }
    expect(renderer.isMatch('state::get')).toBe(false)
    expect(renderer.isMatch('canvas::on-config-change')).toBe(false)
  })

  it('falls through to the console card while unimplemented', () => {
    const renderer = createCanvasTriggerRenderer(host)
    expect(renderer.tryRender(message('canvas::create'))).toBeNull()
    expect(renderer.tryRenderPreview?.(message('canvas::list'))).toBeNull()
  })
})

describe('unwrapEnvelope', () => {
  it('unwraps the harness content/details envelope', () => {
    const details = { id: 'abc12345' }
    expect(unwrapEnvelope({ content: [], details })).toBe(details)
  })

  it('passes every other shape through', () => {
    const plain = { id: 'abc12345' }
    expect(unwrapEnvelope(plain)).toBe(plain)
    expect(unwrapEnvelope(null)).toBeNull()
    expect(unwrapEnvelope([1])).toEqual([1])
    expect(unwrapEnvelope('x')).toBe('x')
  })
})
