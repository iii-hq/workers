import { isValidElement, type ReactNode } from 'react'
import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import { createA2uiTriggerRenderer } from './trigger-renderer'

vi.mock('@iii-dev/console-ui', () => {
  const Passthrough = (props: { children?: ReactNode }) => props.children ?? null
  return {
    Badge: Passthrough,
    Button: Passthrough,
    Input: Passthrough,
    Markdown: Passthrough,
    Skeleton: Passthrough,
    StatusPanel: Passthrough,
  }
})

const host = {} as Host

describe('A2UI trigger renderer', () => {
  it('uses the Console expandable display contract', () => {
    const renderer = createA2uiTriggerRenderer(host)

    expect(renderer.metadata).toEqual({ display: true, displayAction: 'expand' })
    expect(renderer.tryRenderDisplay).toBeTypeOf('function')
  })

  it('renders a compact active surface receipt', () => {
    const renderer = createA2uiTriggerRenderer(host)
    const display = renderer.tryRenderDisplay?.(message())

    expect(display).not.toBeNull()
    expect(textContent(display)).toContain('Incident Triage')
    expect(textContent(display)).toContain('21 components · r6')
    expect(textContent(display)).toContain('Ready')
  })

  it('renders deletion receipts but keeps actions out of the feed', () => {
    const renderer = createA2uiTriggerRenderer(host)
    const deleted = renderer.tryRenderDisplay?.(
      message({ output: receipt({ status: 'deleted' }) }),
    )
    const action = renderer.tryRenderDisplay?.(message({ functionId: 'a2ui::action' }))

    expect(textContent(deleted)).toContain('Deleted')
    expect(action).toBeNull()
  })
})

function message(overrides: Partial<FunctionTriggerMessage> = {}): FunctionTriggerMessage {
  return {
    id: 'call-1',
    role: 'function-trigger',
    functionId: 'a2ui::generate',
    input: {},
    output: receipt(),
    createdAt: 1,
    ...overrides,
  }
}

function receipt(overrides: Record<string, unknown> = {}) {
  return {
    session_id: 'session-1',
    surface_id: 'incident-triage',
    title: 'Incident Triage',
    status: 'active',
    protocol_version: 'v0.9.1',
    catalog_id: 'urn:iii:a2ui:console:v0.1',
    revision: 6,
    component_count: 21,
    page: '#/ext/a2ui',
    ...overrides,
  }
}

function textContent(node: ReactNode): string {
  if (node == null || typeof node === 'boolean') return ''
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(textContent).join(' ')
  if (!isValidElement<{ children?: ReactNode }>(node)) return ''
  return textContent(node.props.children)
}
