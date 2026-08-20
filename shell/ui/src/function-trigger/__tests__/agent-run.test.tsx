import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'

vi.mock('@iii-dev/console-ui', () => ({
  Button: ({ children, title, onClick }: { children: ReactNode; title?: string; onClick?: () => void }) => (
    <button type="button" title={title} onClick={onClick}>
      {children}
    </button>
  ),
}))

import { createAgentRunRenderer } from '../AgentRunView'

function message(functionId: string, extra: Partial<FunctionTriggerMessage> = {}): FunctionTriggerMessage {
  return {
    id: 'message-1',
    role: 'function-trigger',
    functionId,
    input: { cwd: '/workspace/project' },
    output: { ok: true },
    createdAt: 0,
    ...extra,
  }
}

function hostWithPanels() {
  const open = vi.fn()
  return {
    host: { panels: { open } } as unknown as Host,
    open,
  }
}

describe('agent run renderer', () => {
  it('does not claim a function owned by another renderer', () => {
    const { host } = hostWithPanels()
    const renderer = createAgentRunRenderer(host)

    expect(renderer.tryRender(message('coder::tree'))).toBeNull()
    expect(renderer.tryRenderRunning?.(message('coder::tree'))).toBeNull()
  })

  it('renders the terminal action for a supported agent run', () => {
    const { host } = hostWithPanels()
    const renderer = createAgentRunRenderer(host)
    const node = renderer.tryRender(message('codex::run'))

    expect(node).not.toBeNull()
    expect(renderToStaticMarkup(node)).toContain('Open terminal here')
  })

  it('falls through when the run failed or has no working directory', () => {
    const { host } = hostWithPanels()
    const renderer = createAgentRunRenderer(host)

    expect(renderer.tryRender(message('codex::run', { output: { is_error: true } }))).toBeNull()
    expect(renderer.tryRender(message('codex::run', { input: {} }))).toBeNull()
  })

  it('falls through on consoles without contextual panels', () => {
    const renderer = createAgentRunRenderer({} as Host)

    expect(renderer.tryRender(message('codex::run'))).toBeNull()
  })
})
