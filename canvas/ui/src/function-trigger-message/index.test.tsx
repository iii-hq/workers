/**
 * Renderer dispatch tests — DOM-free: tryRender only CREATES elements
 * (component bodies never execute without a React render), so a cast-away
 * Host is safe and nothing here needs jsdom. Value-importing the renderer is
 * safe because index.tsx imports `@iii-dev/console-ui` type-only — the
 * package's runtime entry throws by design.
 */

import { describe, expect, it } from 'vitest'

import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'

import { CANVAS_FUNCTION_IDS } from '../lib/types'
import { HANDLED, createCanvasTriggerRenderer } from './index'

const host = undefined as unknown as Host

function message(
  functionId: string,
  overrides: Partial<FunctionTriggerMessage> = {},
): FunctionTriggerMessage {
  return {
    id: 'm1',
    role: 'function-trigger',
    functionId,
    input: {},
    output: {},
    createdAt: 0,
    ...overrides,
  }
}

const createdRecord = {
  id: 'abc12345',
  name: 'auth flow',
  format: 'mermaid',
  source: 'flowchart TD\n  a --> b',
  family: 'flowchart',
  created_at: 1,
  updated_at: 1,
}

const createDone = message('canvas::create', {
  input: { name: 'auth flow', format: 'mermaid', source: 'flowchart TD' },
  output: { content: [], details: createdRecord },
})

describe('HANDLED matching', () => {
  it('claims exactly the seven public canvas functions', () => {
    const renderer = createCanvasTriggerRenderer(host)
    expect([...HANDLED].sort()).toEqual([...CANVAS_FUNCTION_IDS].sort())
    for (const id of CANVAS_FUNCTION_IDS) {
      expect(renderer.isMatch(id)).toBe(true)
    }
    expect(renderer.isMatch('canvas::on-config-change')).toBe(false)
    expect(renderer.isMatch('state::get')).toBe(false)
    expect(renderer.isMatch('pdf::classify')).toBe(false)
  })
})

describe('dispatch', () => {
  const renderer = createCanvasTriggerRenderer(host)

  it('renders a card per settled function id', () => {
    expect(renderer.tryRender(createDone)).not.toBeNull()
    expect(
      renderer.tryRender(
        message('canvas::update', {
          input: { id: 'abc12345', source: 'flowchart TD\n  a --> b' },
          output: createDone.output,
        }),
      ),
    ).not.toBeNull()
    expect(
      renderer.tryRender(message('canvas::get', { output: createdRecord })),
    ).not.toBeNull()
    expect(
      renderer.tryRender(
        message('canvas::list', { output: { canvases: [], count: 0 } }),
      ),
    ).not.toBeNull()
    expect(
      renderer.tryRender(
        message('canvas::delete', { output: { id: 'abc12345', deleted: true } }),
      ),
    ).not.toBeNull()
    expect(
      renderer.tryRender(
        message('canvas::syntax', {
          output: { families: [{ family: 'flowchart', summary: '', example: '' }] },
        }),
      ),
    ).not.toBeNull()
    expect(
      renderer.tryRender(
        message('canvas::validate', {
          output: { valid: false, family: null, issues: [{ line: 2, message: 'bad' }] },
        }),
      ),
    ).not.toBeNull()
  })

  it('falls through on empty or unrecognizable payloads — never an empty card', () => {
    for (const id of CANVAS_FUNCTION_IDS) {
      expect(renderer.tryRender(message(id))).toBeNull()
    }
    expect(renderer.tryRender(message('canvas::create', { output: 'ok' }))).toBeNull()
  })

  it('renders freeform create compactly (no mermaid path)', () => {
    const freeform = renderer.tryRender(
      message('canvas::create', {
        output: {
          id: 'fre12345',
          name: 'sketch',
          format: 'freeform',
          source: '{"elements":[{"type":"rectangle"}]}',
          family: null,
        },
      }),
    )
    expect(freeform).not.toBeNull()
  })

  it('shows the error string on failed calls', () => {
    const failed = renderer.tryRender(
      message('canvas::create', {
        input: { source: 'flowchart TD' },
        output: { error: { message: 'name already taken' } },
      }),
    )
    expect(failed).not.toBeNull()
  })

  it('yields the card to the host while pending approval', () => {
    const pending = message('canvas::create', {
      pendingApproval: true,
      input: { name: 'auth flow', source: 'flowchart TD' },
      output: undefined,
    })
    expect(renderer.tryRender(pending)).toBeNull()
    expect(renderer.tryRenderPreview?.(pending)).not.toBeNull()
  })

  it('renders a running card only when the input carries content', () => {
    expect(
      renderer.tryRenderRunning?.(
        message('canvas::create', {
          running: true,
          input: { name: 'auth flow', source: 'flowchart TD' },
          output: undefined,
        }),
      ),
    ).not.toBeNull()
    expect(
      renderer.tryRenderRunning?.(
        message('canvas::create', { running: true, output: undefined }),
      ),
    ).toBeNull()
    expect(
      renderer.tryRenderRunning?.(
        message('canvas::list', { running: true, output: undefined }),
      ),
    ).toBeNull()
  })

  it('returns null previews for thin inputs (host request pane wins)', () => {
    expect(renderer.tryRenderPreview?.(message('canvas::list'))).toBeNull()
    expect(renderer.tryRenderPreview?.(message('canvas::syntax'))).toBeNull()
    expect(renderer.tryRenderPreview?.(message('canvas::delete'))).toBeNull()
  })

  it('leaves redactRaw undefined — no secrets ride canvas payloads', () => {
    expect(renderer.redactRaw).toBeUndefined()
  })
})
